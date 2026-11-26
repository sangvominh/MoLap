use crate::win::{wide, Handle, Library, PostMessageW};
use std::{
    collections::VecDeque,
    ffi::c_void,
    mem::transmute,
    ptr::{null, null_mut},
    sync::mpsc::{self, Receiver, Sender},
    thread::JoinHandle,
};

pub const WM_LYRICS_READY: u32 = 0x8018;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LyricsStatus {
    Idle,
    Loading,
    Synced(Vec<LyricLine>),
    Instrumental,
    NotFound,
}

pub struct LyricsUpdate {
    pub track_key: (String, String),
    pub status: LyricsStatus,
}

enum LyricsCmd {
    Fetch {
        title: String,
        artist: String,
        album: String,
        duration_secs: u32,
    },
    Clear,
    Quit,
}

pub struct LyricsHandle {
    tx: Sender<LyricsCmd>,
    rx: Receiver<LyricsUpdate>,
    thread: Option<JoinHandle<()>>,
    requested_key: (String, String),
    active_key: (String, String),
    status: LyricsStatus,
}

impl LyricsHandle {
    pub fn start(hwnd: Handle) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (upd_tx, upd_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || worker(hwnd, cmd_rx, upd_tx))
            .ok();
        Self {
            tx: cmd_tx,
            rx: upd_rx,
            thread,
            requested_key: (String::new(), String::new()),
            active_key: (String::new(), String::new()),
            status: LyricsStatus::Idle,
        }
    }

    pub fn request(&mut self, title: &str, artist: &str, album: &str, duration_secs: u32) {
        let key = (title.to_owned(), artist.to_owned());
        if key == self.requested_key {
            return;
        }
        self.requested_key = key.clone();
        self.active_key = key;
        self.status = if title.is_empty() {
            LyricsStatus::Idle
        } else {
            LyricsStatus::Loading
        };
        if !title.is_empty() {
            let _ = self.tx.send(LyricsCmd::Fetch {
                title: title.to_owned(),
                artist: artist.to_owned(),
                album: album.to_owned(),
                duration_secs,
            });
        } else {
            let _ = self.tx.send(LyricsCmd::Clear);
        }
    }

    pub fn clear(&mut self) {
        if self.requested_key.0.is_empty() && self.status == LyricsStatus::Idle {
            return;
        }
        self.requested_key = (String::new(), String::new());
        self.active_key = (String::new(), String::new());
        self.status = LyricsStatus::Idle;
        let _ = self.tx.send(LyricsCmd::Clear);
    }

    pub fn drain(&mut self) -> bool {
        let mut changed = false;
        while let Ok(upd) = self.rx.try_recv() {
            if upd.track_key == self.requested_key || self.requested_key.0.is_empty() {
                self.active_key = upd.track_key;
                self.status = upd.status;
                changed = true;
            }
        }
        changed
    }

    pub fn status(&self) -> &LyricsStatus {
        &self.status
    }
}

impl Drop for LyricsHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(LyricsCmd::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn active_lyric(lines: &[LyricLine], pos_ms: u64) -> Option<(usize, &str, Option<&str>)> {
    if lines.is_empty() {
        return None;
    }
    let idx = lines.partition_point(|l| l.time_ms <= pos_ms);
    if idx == 0 {
        return Some((0, "", lines.first().map(|l| l.text.as_str())));
    }
    let cur = idx - 1;
    let next = lines.get(cur + 1).map(|l| l.text.as_str());
    Some((cur, lines[cur].text.as_str(), next))
}

pub fn parse_lrc(raw: &str) -> Vec<LyricLine> {
    let mut offset_ms: i64 = 0;
    let mut entries: Vec<(i64, String)> = Vec::new();

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('[') {
            continue;
        }
        let mut rest = line;
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else {
                break;
            };
            let tag = &stripped[..end];
            if let Some(ms) = parse_lrc_timestamp(tag) {
                times.push(ms as i64);
                rest = &stripped[end + 1..];
            } else if let Some(off) = tag.strip_prefix("offset:") {
                if let Ok(parsed) = off.trim().parse::<i64>() {
                    offset_ms = parsed;
                }
                rest = "";
                break;
            } else {
                rest = "";
                break;
            }
        }
        let text = rest.trim();
        if times.is_empty() || text.is_empty() {
            continue;
        }
        for t in times {
            entries.push((t, text.to_owned()));
        }
    }

    let mut lines: Vec<LyricLine> = entries
        .into_iter()
        .map(|(t, text)| LyricLine {
            time_ms: (t - offset_ms).max(0) as u64,
            text,
        })
        .collect();
    lines.sort_by_key(|l| l.time_ms);
    lines
}

fn parse_lrc_timestamp(tag: &str) -> Option<u64> {
    let (min_s, sec_part) = tag.split_once(':')?;
    let mins: u64 = min_s.trim().parse().ok()?;
    let (secs, ms) = if let Some((s_str, frac_str)) = sec_part.split_once('.') {
        let s: u64 = s_str.trim().parse().ok()?;
        let frac = frac_str.trim();
        let val: u64 = frac.parse().ok()?;
        let ms = match frac.len() {
            1 => val * 100,
            2 => val * 10,
            _ => val / 10u64.pow((frac.len() as u32).saturating_sub(3)),
        };
        (s, ms)
    } else if let Some((s_str, frac_str)) = sec_part.split_once(':') {
        let s: u64 = s_str.trim().parse().ok()?;
        let frac: u64 = frac_str.trim().parse().ok()?;
        (s, frac * 10)
    } else {
        (sec_part.trim().parse().ok()?, 0)
    };
    Some(mins * 60_000 + secs * 1_000 + ms.min(999))
}

pub fn clean_track_title(title: &str) -> String {
    let mut out = title.trim().to_owned();
    if let Some((before, _)) = out.split_once(" | ") {
        let trimmed = before.trim();
        if !trimmed.is_empty() {
            out = trimmed.to_owned();
        }
    }
    for (open, close) in [('(', ')'), ('[', ']')] {
        loop {
            let mut removed = false;
            let mut search_from = 0usize;
            while let Some(rel_start) = out[search_from..].find(open) {
                let start = search_from + rel_start;
                if let Some(rel_end) = out[start..].find(close) {
                    let end = start + rel_end;
                    let inner = out[start + 1..end].to_ascii_lowercase();
                    if is_noise_tag(&inner) {
                        let candidate = format!("{}{}", &out[..start], &out[end + 1..]);
                        let trimmed = candidate.trim().to_owned();
                        if !trimmed.is_empty() {
                            out = trimmed;
                            removed = true;
                            break;
                        }
                    }
                    search_from = end + 1;
                } else {
                    break;
                }
            }
            if !removed {
                break;
            }
        }
    }
    while let Some(pos) = find_ascii_case(&out, " feat.").or_else(|| find_ascii_case(&out, " ft."))
    {
        let candidate = out[..pos].trim();
        if candidate.is_empty() {
            break;
        }
        out = candidate.to_owned();
    }
    let cleaned = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        title.trim().to_owned()
    } else {
        cleaned
    }
}

fn is_noise_tag(inner: &str) -> bool {
    const TAGS: [&str; 13] = [
        "official",
        "video",
        "audio",
        "lyric",
        "mv",
        "feat",
        "ft.",
        "Visualizer",
        "visualizer",
        "prod",
        "remaster",
        "live",
        "hd",
    ];
    TAGS.iter().any(|t| inner.contains(&t.to_ascii_lowercase()))
}

fn find_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.to_ascii_lowercase();
    h.find(&needle.to_ascii_lowercase())
}

pub fn primary_artist(artist: &str) -> String {
    let mut cut = artist.trim();
    for sep in [",", "&", " feat.", " ft.", " x ", " X ", ";", "/"] {
        if let Some(idx) = find_ascii_case(cut, sep) {
            let head = cut[..idx].trim();
            if !head.is_empty() {
                cut = head;
            }
        }
    }
    cut.to_owned()
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            b' ' => out.push_str("%20"),
            other => {
                out.push('%');
                let hi = other >> 4;
                let lo = other & 0x0f;
                out.push(hex_char(hi));
                out.push(hex_char(lo));
            }
        }
    }
    out
}

fn hex_char(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        _ => (b'A' + (v - 10)) as char,
    }
}

pub fn parse_lrclib_single(json: &str) -> Option<LyricsStatus> {
    let obj = json.trim();
    if !obj.starts_with('{') {
        return None;
    }
    if let Some(synced) = extract_json_string(obj, "syncedLyrics") {
        let lines = parse_lrc(&synced);
        if !lines.is_empty() {
            return Some(LyricsStatus::Synced(lines));
        }
    }
    if extract_json_bool(obj, "instrumental") == Some(true) {
        return Some(LyricsStatus::Instrumental);
    }
    None
}

pub fn parse_lrclib_search(json: &str, target_dur_secs: u32) -> Option<LyricsStatus> {
    let trimmed = json.trim();
    if !trimmed.starts_with('[') {
        return None;
    }
    let mut best: Option<(u64, LyricsStatus)> = None;
    let mut has_instrumental = false;
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    let mut obj_start = None;

    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else {
            match b {
                b'"' => in_str = true,
                b'{' => {
                    if depth == 0 {
                        obj_start = Some(i);
                    }
                    depth += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(start) = obj_start.take() {
                            let obj = &trimmed[start..=i];
                            if let Some(synced) = extract_json_string(obj, "syncedLyrics") {
                                let lines = parse_lrc(&synced);
                                if !lines.is_empty() {
                                    let diff = if target_dur_secs > 0 {
                                        extract_json_number(obj, "duration")
                                            .map(|d| {
                                                (d.round() as i64 - target_dur_secs as i64)
                                                    .unsigned_abs()
                                            })
                                            .unwrap_or(500)
                                    } else {
                                        0
                                    };
                                    if best.as_ref().is_none_or(|(bd, _)| diff < *bd) {
                                        best = Some((diff, LyricsStatus::Synced(lines)));
                                    }
                                }
                            } else if extract_json_bool(obj, "instrumental") == Some(true) {
                                has_instrumental = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }

    best.map(|(_, st)| st)
        .or(has_instrumental.then_some(LyricsStatus::Instrumental))
}

fn find_json_field_value<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let bytes = obj.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    let mut escaped = false;
    let mut depth = 0i32;

    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if b == b'{' || b == b'[' {
            depth += 1;
            i += 1;
            continue;
        }
        if b == b'}' || b == b']' {
            depth -= 1;
            i += 1;
            continue;
        }
        if b == b'"' {
            if depth == 1 && obj[i..].starts_with(&needle) {
                let mut rest = obj[i + needle.len()..].trim_start();
                if let Some(after_colon) = rest.strip_prefix(':') {
                    rest = after_colon.trim_start();
                    return Some(rest);
                }
            }
            in_str = true;
        }
        i += 1;
    }
    None
}

fn extract_json_bool(obj: &str, key: &str) -> Option<bool> {
    let val = find_json_field_value(obj, key)?;
    if val.starts_with("true") {
        Some(true)
    } else if val.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn extract_json_number(obj: &str, key: &str) -> Option<f64> {
    let val = find_json_field_value(obj, key)?;
    let end = val
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .unwrap_or(val.len());
    val[..end].parse().ok()
}

fn extract_json_string(obj: &str, key: &str) -> Option<String> {
    let val = find_json_field_value(obj, key)?;
    let mut chars = val.chars();
    if chars.next()? != '"' {
        return None;
    }
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            return Some(out);
        }
        if ch == '\\' {
            match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{0008}'),
                'f' => out.push('\u{000c}'),
                'u' => {
                    let mut hex = String::with_capacity(4);
                    for _ in 0..4 {
                        hex.push(chars.next()?);
                    }
                    let code = u16::from_str_radix(&hex, 16).ok()?;
                    if (0xD800..=0xDBFF).contains(&code) {
                        let saved = chars.clone();
                        if chars.next() == Some('\\') && chars.next() == Some('u') {
                            let mut hex2 = String::with_capacity(4);
                            for _ in 0..4 {
                                if let Some(c) = chars.next() {
                                    hex2.push(c);
                                }
                            }
                            if let Ok(low) = u16::from_str_radix(&hex2, 16) {
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    let combined = 0x10000
                                        + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                    if let Some(cp) = char::from_u32(combined) {
                                        out.push(cp);
                                        continue;
                                    }
                                }
                            }
                        }
                        chars = saved;
                    } else if let Some(cp) = char::from_u32(code as u32) {
                        out.push(cp);
                    }
                }
                other => out.push(other),
            }
        } else {
            out.push(ch);
        }
    }
    None
}

struct WinHttp {
    _lib: Library,
    open: unsafe extern "system" fn(*const u16, u32, *const u16, *const u16, u32) -> *mut c_void,
    set_timeouts: unsafe extern "system" fn(*mut c_void, i32, i32, i32, i32) -> i32,
    connect: unsafe extern "system" fn(*mut c_void, *const u16, u16, u32) -> *mut c_void,
    open_request: unsafe extern "system" fn(
        *mut c_void,
        *const u16,
        *const u16,
        *const u16,
        *const u16,
        *const *const u16,
        u32,
    ) -> *mut c_void,
    send_request: unsafe extern "system" fn(
        *mut c_void,
        *const u16,
        u32,
        *const c_void,
        u32,
        u32,
        usize,
    ) -> i32,
    receive_response: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    query_headers: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const u16,
        *mut c_void,
        *mut u32,
        *mut u32,
    ) -> i32,
    query_available: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    read_data: unsafe extern "system" fn(*mut c_void, *mut u8, u32, *mut u32) -> i32,
    close_handle: unsafe extern "system" fn(*mut c_void) -> i32,
}

struct WinHttpHandle<'a> {
    api: &'a WinHttp,
    raw: *mut c_void,
}

impl Drop for WinHttpHandle<'_> {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                (self.api.close_handle)(self.raw);
            }
        }
    }
}

impl WinHttp {
    fn load() -> Option<Self> {
        let lib = Library::system("winhttp.dll")?;
        unsafe {
            Some(Self {
                open: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(
                        *const u16,
                        u32,
                        *const u16,
                        *const u16,
                        u32,
                    ) -> *mut c_void,
                >(lib.symbol(b"WinHttpOpen\0")?),
                set_timeouts: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*mut c_void, i32, i32, i32, i32) -> i32,
                >(lib.symbol(b"WinHttpSetTimeouts\0")?),
                connect: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*mut c_void, *const u16, u16, u32) -> *mut c_void,
                >(lib.symbol(b"WinHttpConnect\0")?),
                open_request: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(
                        *mut c_void,
                        *const u16,
                        *const u16,
                        *const u16,
                        *const u16,
                        *const *const u16,
                        u32,
                    ) -> *mut c_void,
                >(lib.symbol(b"WinHttpOpenRequest\0")?),
                send_request: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(
                        *mut c_void,
                        *const u16,
                        u32,
                        *const c_void,
                        u32,
                        u32,
                        usize,
                    ) -> i32,
                >(lib.symbol(b"WinHttpSendRequest\0")?),
                receive_response: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
                >(lib.symbol(b"WinHttpReceiveResponse\0")?),
                query_headers: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(
                        *mut c_void,
                        u32,
                        *const u16,
                        *mut c_void,
                        *mut u32,
                        *mut u32,
                    ) -> i32,
                >(lib.symbol(b"WinHttpQueryHeaders\0")?),
                query_available: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
                >(lib.symbol(b"WinHttpQueryDataAvailable\0")?),
                read_data: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*mut c_void, *mut u8, u32, *mut u32) -> i32,
                >(lib.symbol(b"WinHttpReadData\0")?),
                close_handle: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*mut c_void) -> i32,
                >(lib.symbol(b"WinHttpCloseHandle\0")?),
                _lib: lib,
            })
        }
    }

    fn get_https(&self, host: &str, path_and_query: &str) -> Option<(u32, String)> {
        unsafe {
            let agent = wide("MoLap/1.0 (Windows)");
            let mut raw_sess = (self.open)(agent.as_ptr(), 4, null(), null(), 0);
            if raw_sess.is_null() {
                raw_sess = (self.open)(agent.as_ptr(), 0, null(), null(), 0);
            }
            if raw_sess.is_null() {
                return None;
            }
            let session = WinHttpHandle {
                api: self,
                raw: raw_sess,
            };
            (self.set_timeouts)(session.raw, 5000, 5000, 5000, 8000);

            let host_w = wide(host);
            let raw_conn = (self.connect)(session.raw, host_w.as_ptr(), 443, 0);
            if raw_conn.is_null() {
                return None;
            }
            let conn = WinHttpHandle {
                api: self,
                raw: raw_conn,
            };

            let verb = wide("GET");
            let path_w = wide(path_and_query);
            let raw_req = (self.open_request)(
                conn.raw,
                verb.as_ptr(),
                path_w.as_ptr(),
                null(),
                null(),
                null(),
                0x0080_0000, // WINHTTP_FLAG_SECURE
            );
            if raw_req.is_null() {
                return None;
            }
            let req = WinHttpHandle {
                api: self,
                raw: raw_req,
            };

            let headers = wide("Accept: application/json\r\n");
            let hdr_len = (headers.len() - 1) as u32;
            if (self.send_request)(req.raw, headers.as_ptr(), hdr_len, null(), 0, 0, 0) == 0 {
                return None;
            }
            if (self.receive_response)(req.raw, null_mut()) == 0 {
                return None;
            }

            let mut status = 0u32;
            let mut status_len = 4u32;
            (self.query_headers)(
                req.raw,
                0x2000_0013, // WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER
                null(),
                (&mut status as *mut u32).cast(),
                &mut status_len,
                null_mut(),
            );

            let mut body = Vec::with_capacity(4096);
            loop {
                let mut avail = 0u32;
                if (self.query_available)(req.raw, &mut avail) == 0 || avail == 0 {
                    break;
                }
                let to_read = avail.min(16_384);
                if body.len() + to_read as usize > 512 * 1024 {
                    break;
                }
                let old_len = body.len();
                body.resize(old_len + to_read as usize, 0);
                let mut got = 0u32;
                if (self.read_data)(req.raw, body[old_len..].as_mut_ptr(), to_read, &mut got) == 0 {
                    body.truncate(old_len);
                    break;
                }
                body.truncate(old_len + got as usize);
                if got == 0 {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&body).into_owned();
            Some((status, text))
        }
    }
}

fn fetch_from_lrclib(
    http: Option<&WinHttp>,
    title: &str,
    artist: &str,
    album: &str,
    duration_secs: u32,
) -> LyricsStatus {
    let Some(http) = http else {
        return LyricsStatus::NotFound;
    };
    let clean_title = clean_track_title(title);
    let main_artist = primary_artist(artist);

    if !main_artist.is_empty() {
        let mut q = format!(
            "/api/get?artist_name={}&track_name={}",
            url_encode(&main_artist),
            url_encode(&clean_title)
        );
        if !album.trim().is_empty() {
            q.push_str("&album_name=");
            q.push_str(&url_encode(album.trim()));
        }
        if duration_secs > 0 {
            q.push_str(&format!("&duration={duration_secs}"));
        }
        if let Some((200, body)) = http.get_https("lrclib.net", &q) {
            if let Some(st) = parse_lrclib_single(&body) {
                return st;
            }
        }

        let q2 = format!(
            "/api/search?track_name={}&artist_name={}",
            url_encode(&clean_title),
            url_encode(&main_artist)
        );
        if let Some((200, body)) = http.get_https("lrclib.net", &q2) {
            if let Some(st) = parse_lrclib_search(&body, duration_secs) {
                return st;
            }
        }
    }

    let query = if main_artist.is_empty() {
        clean_title
    } else {
        format!("{clean_title} {main_artist}")
    };
    let q3 = format!("/api/search?q={}", url_encode(&query));
    if let Some((200, body)) = http.get_https("lrclib.net", &q3) {
        if let Some(st) = parse_lrclib_search(&body, duration_secs) {
            return st;
        }
    }
    LyricsStatus::NotFound
}

fn worker(hwnd: Handle, rx: Receiver<LyricsCmd>, tx: Sender<LyricsUpdate>) {
    let http = WinHttp::load();
    let mut cache: VecDeque<((String, String), LyricsStatus)> = VecDeque::with_capacity(32);

    while let Ok(mut cmd) = rx.recv() {
        while let Ok(newer) = rx.try_recv() {
            cmd = newer;
            if matches!(cmd, LyricsCmd::Quit) {
                break;
            }
        }
        match cmd {
            LyricsCmd::Quit => break,
            LyricsCmd::Clear => {}
            LyricsCmd::Fetch {
                title,
                artist,
                album,
                duration_secs,
            } => {
                let key = (title.clone(), artist.clone());
                let status = if let Some((_, cached)) = cache.iter().find(|(k, _)| *k == key) {
                    cached.clone()
                } else {
                    let fetched =
                        fetch_from_lrclib(http.as_ref(), &title, &artist, &album, duration_secs);
                    if cache.len() >= 32 {
                        cache.pop_front();
                    }
                    cache.push_back((key.clone(), fetched.clone()));
                    fetched
                };
                if tx
                    .send(LyricsUpdate {
                        track_key: key,
                        status,
                    })
                    .is_ok()
                {
                    unsafe {
                        PostMessageW(hwnd, WM_LYRICS_READY, 0, 0);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_parsing_handles_multi_timestamps_offset_and_lookup() {
        let raw = "\
[ti:Test Song]
[offset:500]
[00:10.50]First line
[00:15.000][00:25.25]Chorus line
[00:20.00]
";
        let lines = parse_lrc(raw);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].time_ms, 10_000);
        assert_eq!(lines[0].text, "First line");
        assert_eq!(lines[1].time_ms, 14_500);
        assert_eq!(lines[2].time_ms, 24_750);

        assert_eq!(
            active_lyric(&lines, 5_000),
            Some((0, "", Some("First line")))
        );
        assert_eq!(
            active_lyric(&lines, 10_000),
            Some((0, "First line", Some("Chorus line")))
        );
        assert_eq!(active_lyric(&lines, 24_750), Some((2, "Chorus line", None)));
    }

    #[test]
    fn clean_title_artist_and_url_encode() {
        assert_eq!(
            clean_track_title("Tìm Em (feat. Bảo Anh) [Official MV] | Live"),
            "Tìm Em"
        );
        assert_eq!(primary_artist("Hngle, Bảo Anh"), "Hngle");
        assert_eq!(url_encode("Tìm Em"), "T%C3%ACm%20Em");
    }

    #[test]
    fn json_extraction_parses_lrclib_single_and_search() {
        let single = r#"{"id":1,"trackName":"A","instrumental":false,"syncedLyrics":"[00:01.00]Hello\n[00:05.50]World\u0021"}"#;
        let parsed = parse_lrclib_single(single).unwrap();
        match parsed {
            LyricsStatus::Synced(lines) => {
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[1].text, "World!");
            }
            _ => panic!("expected synced"),
        }

        let search = r#"[{"id":1,"duration":180.0,"instrumental":false,"syncedLyrics":"[00:02.00]Short"},{"id":2,"duration":275.0,"instrumental":false,"syncedLyrics":"[00:02.00]Exact"}]"#;
        match parse_lrclib_search(search, 275) {
            Some(LyricsStatus::Synced(lines)) => assert_eq!(lines[0].text, "Exact"),
            _ => panic!("expected best duration match"),
        }
    }
}
