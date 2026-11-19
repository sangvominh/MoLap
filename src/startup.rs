use crate::win::{reg_string, wide, Handle, HKCU};
use std::{ffi::c_void, io, os::windows::ffi::OsStrExt};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const ENTRY: &str = "MoLap";

#[link(name = "advapi32")]
extern "system" {
    fn RegSetKeyValueW(
        key: Handle,
        subkey: *const u16,
        name: *const u16,
        kind: u32,
        data: *const c_void,
        bytes: u32,
    ) -> i32;
    fn RegDeleteKeyValueW(key: Handle, subkey: *const u16, name: *const u16) -> i32;
}

pub fn enabled() -> bool {
    reg_string(HKCU, RUN_KEY, ENTRY).is_some()
}

fn command_line(path: &std::path::Path) -> io::Result<Vec<u16>> {
    let mut command = vec![b'"' as u16];
    command.extend(path.as_os_str().encode_wide());
    command.extend([b'"' as u16, 0]);
    // Run entries accept a command line of at most 260 characters.
    if command.len() > 261 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The path to MoLap.exe is too long for Windows startup.",
        ));
    }
    Ok(command)
}

pub fn set_enabled(enable: bool) -> io::Result<()> {
    let key = wide(RUN_KEY);
    let name = wide(ENTRY);
    let result = if enable {
        let command = command_line(&std::env::current_exe()?)?;
        unsafe {
            RegSetKeyValueW(
                HKCU,
                key.as_ptr(),
                name.as_ptr(),
                1, // REG_SZ
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        }
    } else {
        unsafe { RegDeleteKeyValueW(HKCU, key.as_ptr(), name.as_ptr()) }
    };
    match result {
        0 => Ok(()),
        2 if !enable => Ok(()), // Another process may already have removed the value.
        error => Err(io::Error::from_raw_os_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_executable_path_for_startup() {
        let path = std::path::Path::new(r"C:\My Apps\MoLap.exe");
        let command = command_line(path).unwrap();
        assert_eq!(command, wide(r#""C:\My Apps\MoLap.exe""#));
    }
}
