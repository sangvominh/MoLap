// Reconstructed QuartzCore Liquid Glass equations & fluid droplet navigation.
// Optical formulas and interaction physics adapted from frixaco/liquid-glass-rs (MIT License).

use crate::gdiplus::RectF;

const KEY_LIGHT: [f32; 2] = [-0.581_238_2, -0.813_733_46]; // normalize([-0.58, -0.81])
const RIM_LIGHT: [f32; 2] = [-0.503_871, -0.863_778_9]; // normalize([-0.5, -0.85])

#[inline]
pub fn sat(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

#[inline]
fn sq(x: f32) -> f32 {
    x * x
}

#[inline]
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)]
}

#[inline]
fn dot2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

#[inline]
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn len2(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// QuartzCore `glass_background_base %300..312`: circular meniscus lens profile.
/// Negative `distance` is inside the glass shape.
#[inline]
pub fn lens(distance: f32, amount: f32, inv_height: f32) -> f32 {
    let t = sat(-distance * inv_height);
    amount * (1.0 - sat((t * (2.0 - t)).sqrt()))
}

/// QuartzCore `%365..370`: radius-to-mipmap level mapping.
#[inline]
pub fn blur_lod(radius: f32) -> f32 {
    let inner = if radius < 2.0 {
        1.0 + radius * 0.5
    } else {
        radius
    };
    inner.log2().max(0.0)
}

/// QuartzCore `%187..198`: 7th-degree odd polynomial shadow CDF.
#[inline]
pub fn shadow_cdf(x: f32) -> f32 {
    let t = 4.0 * sat(x * 0.25 + 0.5) - 2.0;
    let t2 = t * t;
    (((0.002_954_483 * t2 - 0.034_454_346) * t2 + 0.168_212_89) * t2 - 0.560_546_9) * t + 0.5
}

/// QuartzCore `%117..158`: contact ring shadow around the glass edge.
#[inline]
pub fn ring_shadow(ds: f32, radius: f32, stroke: f32, opacity: f32) -> f32 {
    let inv_r = std::f32::consts::FRAC_1_SQRT_2 / radius.max(0.001);
    sat(shadow_cdf(ds * inv_r) - shadow_cdf((ds + stroke) * inv_r)) * opacity
}

/// QuartzCore `%50..104`: dual-lobe directional bevel key/fill highlight (zero full-perimeter stroke).
#[inline]
pub fn key_fill(
    d: f32,
    normal: [f32; 2],
    direction: [f32; 2],
    height: f32,
    spread: f32,
    amount: f32,
    w: f32,
) -> f32 {
    let falloff = 1.0 - sat(-d / height);
    let edge_mix = if falloff != 0.0 { 1.0 } else { 0.0 };
    let band = mix(edge_mix, falloff, 0.75) * sat((d + height) / w + 0.5) * sat(-d / w + 0.5);
    let n = dot2(normal, direction);
    let denom_spread = (1.0 - spread).max(0.000_001);
    let l0 = ((n - spread) / denom_spread).clamp(0.0, 1.0) * band;
    let l1 = ((-n - spread) / denom_spread).clamp(0.0, 1.0) * band;
    let s0 = l0 / (1.0 + amount * (1.0 - l0)).max(0.000_001);
    let s1 = l1 / (1.0 + amount * (1.0 - l1)).max(0.000_001);
    s0 + s1
}

/// QuartzCore `%833..838`: nonlinear highlight color bias.
#[inline]
pub fn highlight_color(color: [f32; 3], intensity: f32, bias: f32) -> [f32; 3] {
    [
        color[0] * (1.0 + bias * intensity * (3.0 - 2.0 * color[0])),
        color[1] * (1.0 + bias * intensity * (3.0 - 2.0 * color[1])),
        color[2] * (1.0 + bias * intensity * (3.0 - 2.0 * color[2])),
    ]
}

#[inline]
pub fn coverage(d: f32, width: f32) -> f32 {
    sat(0.5 - d / width.max(0.0001))
}

#[inline]
pub fn rounded_box_sdf(p: [f32; 2], half_size: [f32; 2], radius: f32) -> f32 {
    let r = radius.min(half_size[0]).min(half_size[1]).max(0.0);
    let qx = p[0].abs() - half_size[0] + r;
    let qy = p[1].abs() - half_size[1] + r;
    let outside = len2([qx.max(0.0), qy.max(0.0)]);
    let inside = qx.max(qy).min(0.0);
    outside + inside - r
}

#[inline]
pub fn rounded_box_normal(p: [f32; 2], half_size: [f32; 2], radius: f32) -> [f32; 2] {
    let r = radius.min(half_size[0]).min(half_size[1]).max(0.0);
    let ax = p[0].abs() - half_size[0] + r;
    let ay = p[1].abs() - half_size[1] + r;
    let sx = if p[0] >= 0.0 { 1.0 } else { -1.0 };
    let sy = if p[1] >= 0.0 { 1.0 } else { -1.0 };
    if ax > 0.0 || ay > 0.0 {
        let gx = ax.max(0.0) * sx;
        let gy = ay.max(0.0) * sy;
        let m = len2([gx, gy]);
        if m > 1e-6 {
            return [gx / m, gy / m];
        }
    }
    if ax >= ay {
        [sx, 0.0]
    } else {
        [0.0, sy]
    }
}

struct DropletGeom {
    center: [f32; 2],
    axis: [f32; 2],
    hs: [f32; 2],
    r: f32,
    trail_shift: f32,
    neck: f32,
    tail_r: f32,
    is_resting: bool,
}

impl DropletGeom {
    #[inline]
    fn new(
        center: [f32; 2],
        half_size: [f32; 2],
        corner_r: f32,
        shown: f32,
        velocity: [f32; 2],
    ) -> Self {
        let speed = len2(velocity);
        let axis = if speed > 1.0 {
            [velocity[0] / speed, velocity[1] / speed]
        } else {
            [1.0, 0.0]
        };
        let stretch = (speed / 1500.0).min(0.46);
        let sx = 1.0 + stretch;
        let sy = 1.0 / sx.sqrt();
        let shown = shown.clamp(0.0, 1.0);
        let lift_x = 1.0 + 0.06 * shown;
        let lift_y = 1.0 + 0.14 * shown;
        let hs = [half_size[0] * lift_x * sx, half_size[1] * lift_y * sy];
        let r = corner_r.min(hs[0]).min(hs[1]);
        let trail_shift = (speed * 0.022).min(hs[0] * 0.85);
        let neck = shown.max((speed / 280.0).clamp(0.0, 1.0));
        let tail_r = (hs[1] * 0.68).max(2.0);
        Self {
            center,
            axis,
            hs,
            r,
            trail_shift,
            neck,
            tail_r,
            is_resting: speed < 1.0 && shown < 0.005,
        }
    }

    #[inline]
    fn sdf(&self, p: [f32; 2]) -> f32 {
        let q = [p[0] - self.center[0], p[1] - self.center[1]];
        if self.is_resting {
            return rounded_box_sdf(q, self.hs, self.r);
        }
        let local = [
            q[0] * self.axis[0] + q[1] * self.axis[1],
            q[0] * -self.axis[1] + q[1] * self.axis[0],
        ];
        let body = rounded_box_sdf(local, self.hs, self.r);
        let tail = len2([local[0] + self.trail_shift, local[1]]) - self.tail_r;
        let h = (9.0 - (body - tail).abs()).max(0.0) / 9.0;
        body.min(tail) - h * h * 2.4 * self.neck
    }

    #[inline]
    fn normal(&self, p: [f32; 2]) -> [f32; 2] {
        if self.is_resting {
            return rounded_box_normal(
                [p[0] - self.center[0], p[1] - self.center[1]],
                self.hs,
                self.r,
            );
        }
        let gx = self.sdf([p[0] + 0.5, p[1]]) - self.sdf([p[0] - 0.5, p[1]]);
        let gy = self.sdf([p[0], p[1] + 0.5]) - self.sdf([p[0], p[1] - 0.5]);
        let m = len2([gx, gy]);
        if m > 1e-6 {
            [gx / m, gy / m]
        } else {
            [0.0, -1.0]
        }
    }
}

/// Authored velocity-aligned liquid droplet SDF with trailing lobe smooth union.
#[inline]
#[allow(dead_code)]
pub fn liquid_droplet_sdf(
    p: [f32; 2],
    center: [f32; 2],
    half_size: [f32; 2],
    corner_r: f32,
    shown: f32,
    velocity: [f32; 2],
) -> f32 {
    DropletGeom::new(center, half_size, corner_r, shown, velocity).sdf(p)
}

#[inline]
#[allow(dead_code)]
pub fn liquid_droplet_normal(
    p: [f32; 2],
    center: [f32; 2],
    half_size: [f32; 2],
    corner_r: f32,
    shown: f32,
    velocity: [f32; 2],
) -> [f32; 2] {
    DropletGeom::new(center, half_size, corner_r, shown, velocity).normal(p)
}

#[derive(Clone, Copy)]
pub struct GlassSpec {
    pub blur_radius: f32,
    pub refract_inner: f32,
    pub refract_outer: f32,
    pub aberrate_amount: f32,
    pub saturation: f32,
    pub base_rgb: [f32; 3],
    pub face_mul: [f32; 3],
    pub face_add: [f32; 3],
    pub face_opacity: f32,
    pub key_height: f32,
    pub key_spread: f32,
    pub key_amount: f32,
    pub key_bias: f32,
    pub shadow_rgb: [f32; 3],
    pub shadow_opacity: f32,
    pub shadow_radius: f32,
    pub shadow_offset_y: f32,
    pub ring_opacity: f32,
    pub ring_radius: f32,
    pub ring_stroke: f32,
    pub bleed_amount: f32,
    pub bleed_rgb: Option<[f32; 3]>,
    pub refract_scale: f32,
    pub light: bool,
}

impl GlassSpec {
    pub fn shell(light: bool, bleed_rgb: Option<(u8, u8, u8)>) -> Self {
        let bleed =
            bleed_rgb.map(|(r, g, b)| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]);
        if light {
            Self {
                blur_radius: 14.0,
                refract_inner: 16.0,
                refract_outer: 24.0,
                aberrate_amount: 0.9,
                saturation: 1.06,
                base_rgb: [0.935, 0.945, 0.962],
                face_mul: [0.82, 0.83, 0.84],
                face_add: [0.165, 0.168, 0.174],
                face_opacity: 0.96,
                key_height: 2.4,
                key_spread: 0.14,
                key_amount: 0.60,
                key_bias: 0.45,
                shadow_rgb: [0.06, 0.08, 0.14],
                shadow_opacity: 0.20,
                shadow_radius: 13.0,
                shadow_offset_y: 4.5,
                ring_opacity: 0.18,
                ring_radius: 2.2,
                ring_stroke: 1.8,
                bleed_amount: if bleed.is_some() { 0.36 } else { 0.0 },
                bleed_rgb: bleed,
                refract_scale: 0.0,
                light: true,
            }
        } else {
            Self {
                blur_radius: 14.0,
                refract_inner: 16.0,
                refract_outer: 24.0,
                aberrate_amount: 0.9,
                saturation: 1.08,
                base_rgb: [0.065, 0.068, 0.078],
                face_mul: [0.76, 0.76, 0.78],
                face_add: [0.052, 0.054, 0.060],
                face_opacity: 0.95,
                key_height: 2.2,
                key_spread: 0.16,
                key_amount: 0.58,
                key_bias: 0.60,
                shadow_rgb: [0.0, 0.0, 0.0],
                shadow_opacity: 0.36,
                shadow_radius: 13.0,
                shadow_offset_y: 5.0,
                ring_opacity: 0.24,
                ring_radius: 2.8,
                ring_stroke: 2.2,
                bleed_amount: if bleed.is_some() { 0.42 } else { 0.0 },
                bleed_rgb: bleed,
                refract_scale: 0.0,
                light: false,
            }
        }
    }

    pub fn panel(light: bool) -> Self {
        if light {
            Self {
                blur_radius: 11.0,
                refract_inner: 24.0,
                refract_outer: 38.0,
                aberrate_amount: 1.25,
                saturation: 1.10,
                base_rgb: [0.96, 0.97, 0.985],
                face_mul: [0.78, 0.79, 0.81],
                face_add: [0.20, 0.204, 0.212],
                face_opacity: 0.48,
                key_height: 2.6,
                key_spread: 0.12,
                key_amount: 0.58,
                key_bias: 0.62,
                shadow_rgb: [0.05, 0.08, 0.15],
                shadow_opacity: 0.14,
                shadow_radius: 12.0,
                shadow_offset_y: 5.0,
                ring_opacity: 0.15,
                ring_radius: 2.2,
                ring_stroke: 1.7,
                bleed_amount: 0.32,
                bleed_rgb: None,
                refract_scale: 1.0,
                light: true,
            }
        } else {
            Self {
                blur_radius: 11.0,
                refract_inner: 24.0,
                refract_outer: 38.0,
                aberrate_amount: 1.25,
                saturation: 1.12,
                base_rgb: [0.11, 0.12, 0.14],
                face_mul: [0.74, 0.75, 0.78],
                face_add: [0.075, 0.078, 0.090],
                face_opacity: 0.54,
                key_height: 2.6,
                key_spread: 0.12,
                key_amount: 0.58,
                key_bias: 0.72,
                shadow_rgb: [0.0, 0.0, 0.0],
                shadow_opacity: 0.30,
                shadow_radius: 12.0,
                shadow_offset_y: 5.5,
                ring_opacity: 0.22,
                ring_radius: 2.4,
                ring_stroke: 1.9,
                bleed_amount: 0.36,
                bleed_rgb: None,
                refract_scale: 1.0,
                light: false,
            }
        }
    }

    pub fn pill(light: bool, tint: Option<[f32; 3]>, active: bool, press_t: f32) -> Self {
        let lift = press_t.clamp(0.0, 1.0);
        let boost = if active { 1.0 } else { 0.84 } + lift * 0.22;
        if light {
            let mut face_mul = if active {
                [0.66, 0.67, 0.69]
            } else {
                [0.80, 0.81, 0.83]
            };
            let mut face_add = if active {
                [0.33, 0.333, 0.34]
            } else {
                [0.18 + 0.04 * lift, 0.184 + 0.04 * lift, 0.192 + 0.04 * lift]
            };
            if let Some(t) = tint {
                let chroma = (t[0] - t[1])
                    .abs()
                    .max((t[1] - t[2]).abs())
                    .max((t[0] - t[2]).abs());
                if chroma > 0.06 {
                    face_mul = [
                        mix(face_mul[0], 0.58 + t[0] * 0.34, 0.45),
                        mix(face_mul[1], 0.58 + t[1] * 0.34, 0.45),
                        mix(face_mul[2], 0.58 + t[2] * 0.34, 0.45),
                    ];
                    face_add = [
                        mix(face_add[0], t[0] * 0.32, 0.40),
                        mix(face_add[1], t[1] * 0.32, 0.40),
                        mix(face_add[2], t[2] * 0.32, 0.40),
                    ];
                }
            }
            Self {
                blur_radius: if active { 6.0 } else { 4.5 },
                refract_inner: if active { 13.5 * boost } else { 10.5 * boost },
                refract_outer: if active { 20.0 * boost } else { 16.0 * boost },
                aberrate_amount: if active { 0.90 * boost } else { 0.75 * boost },
                saturation: 1.08,
                base_rgb: if active {
                    [0.99, 0.995, 1.0]
                } else {
                    [0.96, 0.97, 0.985]
                },
                face_mul,
                face_add,
                face_opacity: if active { 0.50 } else { 0.24 },
                key_height: 2.3,
                key_spread: 0.10,
                key_amount: 0.50,
                key_bias: if active { 0.72 } else { 0.64 } + 0.16 * lift,
                shadow_rgb: [0.05, 0.08, 0.15],
                shadow_opacity: if active { 0.14 } else { 0.10 },
                shadow_radius: if active { 6.0 } else { 5.0 },
                shadow_offset_y: if active { 2.2 } else { 1.8 },
                ring_opacity: if active { 0.17 } else { 0.15 },
                ring_radius: 1.8,
                ring_stroke: 1.5,
                bleed_amount: 0.20,
                bleed_rgb: None,
                refract_scale: 1.0,
                light: true,
            }
        } else {
            let mut face_mul = if active {
                [0.76, 0.77, 0.80]
            } else {
                [0.84, 0.85, 0.87]
            };
            let mut face_add = if active {
                [0.15 + 0.05 * lift, 0.154 + 0.05 * lift, 0.166 + 0.05 * lift]
            } else {
                [
                    0.085 + 0.04 * lift,
                    0.088 + 0.04 * lift,
                    0.098 + 0.04 * lift,
                ]
            };
            if let Some(t) = tint {
                let chroma = (t[0] - t[1])
                    .abs()
                    .max((t[1] - t[2]).abs())
                    .max((t[0] - t[2]).abs());
                if chroma > 0.06 {
                    face_mul = [
                        mix(face_mul[0], 0.62 + t[0] * 0.34, 0.45),
                        mix(face_mul[1], 0.62 + t[1] * 0.34, 0.45),
                        mix(face_mul[2], 0.62 + t[2] * 0.34, 0.45),
                    ];
                    face_add = [
                        mix(face_add[0], t[0] * 0.16, 0.45),
                        mix(face_add[1], t[1] * 0.16, 0.45),
                        mix(face_add[2], t[2] * 0.16, 0.45),
                    ];
                }
            }
            Self {
                blur_radius: if active { 6.0 } else { 4.5 },
                refract_inner: if active { 13.5 * boost } else { 10.5 * boost },
                refract_outer: if active { 20.0 * boost } else { 16.0 * boost },
                aberrate_amount: if active { 0.90 * boost } else { 0.75 * boost },
                saturation: 1.10,
                base_rgb: [0.16, 0.17, 0.20],
                face_mul,
                face_add,
                face_opacity: if active { 0.44 } else { 0.22 },
                key_height: 2.3,
                key_spread: 0.10,
                key_amount: 0.50,
                key_bias: if active { 0.72 } else { 0.64 } + 0.16 * lift,
                shadow_rgb: [0.0, 0.0, 0.0],
                shadow_opacity: if active { 0.28 } else { 0.18 },
                shadow_radius: 7.0,
                shadow_offset_y: 3.0,
                ring_opacity: 0.20,
                ring_radius: 2.0,
                ring_stroke: 1.6,
                bleed_amount: 0.22,
                bleed_rgb: None,
                refract_scale: 1.0,
                light: false,
            }
        }
    }
}

/// 5-level premultiplied mipmap pyramid over a bounded DIB region for real backdrop blur & refraction.
pub struct BackdropPyramid {
    ox: i32,
    oy: i32,
    scale: f32,
    levels: [Vec<u8>; 5],
    sizes: [(i32, i32); 5],
    level_count: usize,
}

impl Default for BackdropPyramid {
    fn default() -> Self {
        Self {
            ox: 0,
            oy: 0,
            scale: 1.0,
            levels: Default::default(),
            sizes: [(0, 0); 5],
            level_count: 0,
        }
    }
}

impl BackdropPyramid {
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        bits: *const u8,
        surf_w: i32,
        surf_h: i32,
        scale: f32,
        bounds: RectF,
        pad_px: i32,
        max_levels: usize,
    ) -> bool {
        if bits.is_null() || surf_w <= 0 || surf_h <= 0 {
            return false;
        }
        let x0 = ((bounds.x * scale).floor() as i32 - pad_px).clamp(0, surf_w - 1);
        let y0 = ((bounds.y * scale).floor() as i32 - pad_px).clamp(0, surf_h - 1);
        let x1 = ((bounds.x + bounds.w) * scale).ceil() as i32 + pad_px;
        let y1 = ((bounds.y + bounds.h) * scale).ceil() as i32 + pad_px;
        let x1 = x1.clamp(x0 + 1, surf_w);
        let y1 = y1.clamp(y0 + 1, surf_h);
        let w = x1 - x0;
        let h = y1 - y0;

        self.ox = x0;
        self.oy = y0;
        self.scale = scale.max(0.5);
        self.sizes[0] = (w, h);

        let len0 = (w * h * 4) as usize;
        self.levels[0].resize(len0, 0);
        let src_stride = (surf_w * 4) as usize;
        let dst_stride = (w * 4) as usize;
        unsafe {
            for row in 0..h {
                let s_off = ((y0 + row) as usize) * src_stride + (x0 as usize) * 4;
                let d_off = (row as usize) * dst_stride;
                std::ptr::copy_nonoverlapping(
                    bits.add(s_off),
                    self.levels[0].as_mut_ptr().add(d_off),
                    dst_stride,
                );
            }
        }

        let cap = max_levels.clamp(1, 5);
        let mut count = 1usize;
        for lvl in 1..cap {
            let (pw, ph) = self.sizes[lvl - 1];
            if pw <= 2 && ph <= 2 {
                break;
            }
            let nw = (pw / 2).max(1);
            let nh = (ph / 2).max(1);
            self.sizes[lvl] = (nw, nh);
            let nlen = (nw * nh * 4) as usize;
            let (prev_slice, rest) = self.levels.split_at_mut(lvl);
            let src = &prev_slice[lvl - 1];
            let dst = &mut rest[0];
            dst.resize(nlen, 0);
            for y in 0..nh {
                let sy0 = (y * 2).min(ph - 1);
                let sy1 = (sy0 + 1).min(ph - 1);
                for x in 0..nw {
                    let sx0 = (x * 2).min(pw - 1);
                    let sx1 = (sx0 + 1).min(pw - 1);
                    let i00 = ((sy0 * pw + sx0) * 4) as usize;
                    let i10 = ((sy0 * pw + sx1) * 4) as usize;
                    let i01 = ((sy1 * pw + sx0) * 4) as usize;
                    let i11 = ((sy1 * pw + sx1) * 4) as usize;
                    let di = ((y * nw + x) * 4) as usize;
                    for c in 0..4 {
                        let sum = src[i00 + c] as u32
                            + src[i10 + c] as u32
                            + src[i01 + c] as u32
                            + src[i11 + c] as u32
                            + 2;
                        dst[di + c] = (sum >> 2) as u8;
                    }
                }
            }
            count = lvl + 1;
        }
        self.level_count = count;
        true
    }

    #[inline]
    fn sample_level(&self, lvl: usize, lx: f32, ly: f32) -> [f32; 4] {
        let (w, h) = self.sizes[lvl];
        let div = (1 << lvl) as f32;
        let fx = ((lx * self.scale - self.ox as f32) / div - 0.5).clamp(0.0, (w - 1).max(0) as f32);
        let fy = ((ly * self.scale - self.oy as f32) / div - 0.5).clamp(0.0, (h - 1).max(0) as f32);
        let x0 = fx.floor() as i32;
        let y0 = fy.floor() as i32;
        let x1 = (x0 + 1).min(w - 1);
        let y1 = (y0 + 1).min(h - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        let buf = &self.levels[lvl];
        let i00 = ((y0 * w + x0) * 4) as usize;
        let i10 = ((y0 * w + x1) * 4) as usize;
        let i01 = ((y1 * w + x0) * 4) as usize;
        let i11 = ((y1 * w + x1) * 4) as usize;

        let w00 = (1.0 - tx) * (1.0 - ty);
        let w10 = tx * (1.0 - ty);
        let w01 = (1.0 - tx) * ty;
        let w11 = tx * ty;

        let inv = 1.0 / 255.0;
        let pb = (buf[i00] as f32 * w00
            + buf[i10] as f32 * w10
            + buf[i01] as f32 * w01
            + buf[i11] as f32 * w11)
            * inv;
        let pg = (buf[i00 + 1] as f32 * w00
            + buf[i10 + 1] as f32 * w10
            + buf[i01 + 1] as f32 * w01
            + buf[i11 + 1] as f32 * w11)
            * inv;
        let pr = (buf[i00 + 2] as f32 * w00
            + buf[i10 + 2] as f32 * w10
            + buf[i01 + 2] as f32 * w01
            + buf[i11 + 2] as f32 * w11)
            * inv;
        let pa = (buf[i00 + 3] as f32 * w00
            + buf[i10 + 3] as f32 * w10
            + buf[i01 + 3] as f32 * w01
            + buf[i11 + 3] as f32 * w11)
            * inv;
        if pa >= 0.996 {
            [pr.min(1.0), pg.min(1.0), pb.min(1.0), 1.0]
        } else if pa > 0.002 {
            [
                (pr / pa).min(1.0),
                (pg / pa).min(1.0),
                (pb / pa).min(1.0),
                pa.min(1.0),
            ]
        } else {
            [0.0, 0.0, 0.0, 0.0]
        }
    }

    #[inline]
    pub fn sample_srgb(&self, lx: f32, ly: f32, lod: f32) -> [f32; 4] {
        let max_l = self.level_count.saturating_sub(1);
        let clamped = lod.clamp(0.0, max_l as f32);
        let l0 = clamped.floor() as usize;
        let l1 = (l0 + 1).min(max_l);
        let frac = clamped - l0 as f32;
        if frac < 0.02 || l0 == l1 {
            self.sample_level(l0, lx, ly)
        } else {
            let a = self.sample_level(l0, lx, ly);
            let b = self.sample_level(l1, lx, ly);
            [
                mix(a[0], b[0], frac),
                mix(a[1], b[1], frac),
                mix(a[2], b[2], frac),
                mix(a[3], b[3], frac),
            ]
        }
    }

    /// 3-tap RGB chromatic dispersion prism (physical R/G/B wavelength shift).
    #[inline]
    pub fn aberrate_srgb(&self, lx: f32, ly: f32, direction: [f32; 2], lod: f32) -> [f32; 4] {
        if direction[0].abs() + direction[1].abs() < 0.08 {
            return self.sample_srgb(lx, ly, lod);
        }
        let c_pos = self.sample_srgb(lx + direction[0], ly + direction[1], lod);
        let c_mid = self.sample_srgb(lx, ly, lod);
        let c_neg = self.sample_srgb(lx - direction[0], ly - direction[1], lod);
        [
            c_pos[0].clamp(0.0, 1.0),
            c_mid[1].clamp(0.0, 1.0),
            c_neg[2].clamp(0.0, 1.0),
            ((c_pos[3] + c_mid[3] + c_neg[3]) * (1.0 / 3.0)).clamp(0.0, 1.0),
        ]
    }
}

#[inline]
fn saturate_rgb(c: [f32; 3], amount: f32) -> [f32; 3] {
    let luma = dot3(c, [0.2126, 0.7152, 0.0722]);
    [
        mix(luma, c[0], amount).clamp(0.0, 1.0),
        mix(luma, c[1], amount).clamp(0.0, 1.0),
        mix(luma, c[2], amount).clamp(0.0, 1.0),
    ]
}

#[inline]
fn write_pargb(px: &mut [u8], rgb: [f32; 3], alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    let ai = (a * 255.0).round() as u8;
    let pr = (rgb[0].clamp(0.0, 1.0) * a * 255.0).round().min(ai as f32) as u8;
    let pg = (rgb[1].clamp(0.0, 1.0) * a * 255.0).round().min(ai as f32) as u8;
    let pb = (rgb[2].clamp(0.0, 1.0) * a * 255.0).round().min(ai as f32) as u8;
    px[0] = pb;
    px[1] = pg;
    px[2] = pr;
    px[3] = ai;
}

#[inline]
fn composite_over_pargb(px: &mut [u8], src_rgb: [f32; 3], src_a: f32) {
    let sa = src_a.clamp(0.0, 1.0);
    if sa <= 0.001 {
        return;
    }
    if sa >= 0.998 && px[3] >= 254 {
        write_pargb(px, src_rgb, 1.0);
        return;
    }
    let inv = 1.0 / 255.0;
    let da = px[3] as f32 * inv;
    let db = px[0] as f32 * inv;
    let dg = px[1] as f32 * inv;
    let dr = px[2] as f32 * inv;
    let inv_sa = 1.0 - sa;
    let out_a = (sa + da * inv_sa).clamp(0.0, 1.0);
    let out_ai = (out_a * 255.0).round() as u8;
    let out_r = ((src_rgb[0].clamp(0.0, 1.0) * sa + dr * inv_sa) * 255.0)
        .round()
        .clamp(0.0, out_ai as f32) as u8;
    let out_g = ((src_rgb[1].clamp(0.0, 1.0) * sa + dg * inv_sa) * 255.0)
        .round()
        .clamp(0.0, out_ai as f32) as u8;
    let out_b = ((src_rgb[2].clamp(0.0, 1.0) * sa + db * inv_sa) * 255.0)
        .round()
        .clamp(0.0, out_ai as f32) as u8;
    px[0] = out_b;
    px[1] = out_g;
    px[2] = out_r;
    px[3] = out_ai;
}

/// Shades a rounded-rectangle Liquid Glass element directly onto a top-down `PixelFormat32bppPARGB` DIB.
#[allow(clippy::too_many_arguments)]
pub fn shade_rounded_glass(
    bits: *mut u8,
    surf_w: i32,
    surf_h: i32,
    scale: f32,
    rect: RectF,
    radius: f32,
    spec: &GlassSpec,
    pyramid: Option<&BackdropPyramid>,
    clip_rect: Option<(RectF, f32)>,
    alpha: f32,
) {
    if bits.is_null()
        || surf_w <= 0
        || surf_h <= 0
        || rect.w <= 1.0
        || rect.h <= 1.0
        || alpha <= 0.01
    {
        return;
    }
    let scale = scale.max(0.5);
    let half = [rect.w * 0.5, rect.h * 0.5];
    let center = [rect.x + half[0], rect.y + half[1]];
    let r = radius.min(half[0]).min(half[1]).max(0.5);
    let aa = 1.0 / scale;

    let pad_l = (spec.shadow_radius * 1.3).ceil() + 3.0;
    let pad_t = (spec.shadow_radius * 0.9).ceil() + 3.0;
    let pad_b = (spec.shadow_radius * 1.4 + spec.shadow_offset_y).ceil() + 3.0;

    let mut x0 = ((rect.x - pad_l) * scale).floor() as i32;
    let mut y0 = ((rect.y - pad_t) * scale).floor() as i32;
    let mut x1 = ((rect.x + rect.w + pad_l) * scale).ceil() as i32;
    let mut y1 = ((rect.y + rect.h + pad_b) * scale).ceil() as i32;
    if let Some((cr, _)) = clip_rect {
        x0 = x0.max((cr.x * scale).floor() as i32);
        y0 = y0.max((cr.y * scale).floor() as i32);
        x1 = x1.min(((cr.x + cr.w) * scale).ceil() as i32);
        y1 = y1.min(((cr.y + cr.h) * scale).ceil() as i32);
    }
    x0 = x0.clamp(0, surf_w);
    y0 = y0.clamp(0, surf_h);
    x1 = x1.clamp(x0, surf_w);
    y1 = y1.clamp(y0, surf_h);

    let clip_geom = clip_rect.map(|(cr, cr_r)| {
        let ch = [cr.w * 0.5, cr.h * 0.5];
        ([cr.x + ch[0], cr.y + ch[1]], ch, cr_r.min(ch[0]).min(ch[1]))
    });

    let bevel_depth = 34.0f32;
    let flat_lod = pyramid
        .map(|_| blur_lod(spec.blur_radius * 0.25 * scale))
        .unwrap_or(0.0);
    let base_face = spec.base_rgb;

    let stride = (surf_w * 4) as usize;
    unsafe {
        for py in y0..y1 {
            let ly = (py as f32 + 0.5) / scale;
            let row_ptr = bits.add((py as usize) * stride);
            for px_x in x0..x1 {
                let lx = (px_x as f32 + 0.5) / scale;

                let clip_cov = if let Some((cc, ch, cr_r)) = clip_geom {
                    let cd = rounded_box_sdf([lx - cc[0], ly - cc[1]], ch, cr_r);
                    if cd >= aa * 0.6 {
                        continue;
                    }
                    coverage(cd, aa)
                } else {
                    1.0
                };

                let q = [lx - center[0], ly - center[1]];
                let d = rounded_box_sdf(q, half, r);
                let mask = coverage(d, aa) * clip_cov;

                let pixel = std::slice::from_raw_parts_mut(row_ptr.add((px_x as usize) * 4), 4);

                // Exterior drop shadow & contact ring occlusion (QuartzCore %117..203)
                if d > -aa {
                    let shadow_cov = (1.0 - mask) * clip_cov * alpha;
                    if shadow_cov > 0.002 {
                        let ds = rounded_box_sdf([q[0], q[1] - spec.shadow_offset_y], half, r);
                        let sh =
                            sat(shadow_cdf(ds / spec.shadow_radius.max(1.0))) * spec.shadow_opacity;
                        let rg =
                            ring_shadow(ds, spec.ring_radius, spec.ring_stroke, spec.ring_opacity);
                        let shadow_a = sat(sh + rg * (1.0 - sh)) * shadow_cov;
                        if shadow_a > 0.003 {
                            composite_over_pargb(pixel, spec.shadow_rgb, shadow_a);
                        }
                    }
                }

                if mask <= 0.002 {
                    continue;
                }

                // Interior glass body: fast flat core path when deep inside the rounded box.
                if d < -bevel_depth {
                    let (face, final_a) = if let Some(pyr) = pyramid {
                        let sample = pyr.sample_srgb(lx, ly, flat_lod);
                        let src_rgb = mix3(base_face, [sample[0], sample[1], sample[2]], sample[3]);
                        let sat_c = saturate_rgb(src_rgb, spec.saturation);
                        let treated = [
                            sat_c[0] * spec.face_mul[0] + spec.face_add[0],
                            sat_c[1] * spec.face_mul[1] + spec.face_add[1],
                            sat_c[2] * spec.face_mul[2] + spec.face_add[2],
                        ];
                        let out = mix3(src_rgb, treated, spec.face_opacity);
                        let body_a = mix(
                            spec.face_opacity,
                            (spec.face_opacity + 0.36).min(0.92),
                            sample[3] * 0.85,
                        );
                        (out, (mask * alpha * body_a).clamp(0.0, 1.0))
                    } else {
                        (
                            base_face,
                            (mask * alpha * spec.face_opacity).clamp(0.0, 1.0),
                        )
                    };
                    composite_over_pargb(pixel, face, final_a);
                    continue;
                }

                // Bevel & lens refraction zone
                let n = rounded_box_normal(q, half, r);
                let disp_inner = lens(d, spec.refract_inner * spec.refract_scale, 1.0 / 24.0);
                let shifted = d + disp_inner;
                let r0 = ((shifted + 40.0) / 22.0).clamp(0.0, 1.0);
                let r1 = ((shifted + 18.0) / 15.0).clamp(0.0, 1.0);
                let r2 = ((shifted + 3.0) / 15.0).clamp(0.0, 1.0);
                let blur_r = spec.blur_radius * (1.0 - (r0 * 0.20 + r1 * 0.30 + r2 * 0.35));
                let lod = blur_lod(blur_r * scale);

                let (mut face, back_a) = if let Some(pyr) = pyramid {
                    let rx = lx + n[0] * disp_inner;
                    let ry = ly + n[1] * disp_inner;
                    let ab_amp = lens(d, spec.aberrate_amount * spec.refract_scale, 1.0 / 16.0);
                    let ab = pyr.aberrate_srgb(rx, ry, [n[0] * ab_amp, n[1] * ab_amp], lod);
                    let mut c = mix3(base_face, [ab[0], ab[1], ab[2]], ab[3]);
                    let outer_mix = 0.30 * sat((d + 16.0) / 16.0) * spec.refract_scale;
                    if outer_mix > 0.005 {
                        let disp_outer =
                            lens(d, spec.refract_outer * spec.refract_scale, 1.0 / 18.0);
                        let out =
                            pyr.sample_srgb(lx + n[0] * disp_outer, ly + n[1] * disp_outer, lod);
                        let out_rgb = mix3(base_face, [out[0], out[1], out[2]], out[3]);
                        c = mix3(c, out_rgb, outer_mix);
                    }
                    let sat_c = saturate_rgb(c, spec.saturation);
                    let treated = [
                        sat_c[0] * spec.face_mul[0] + spec.face_add[0],
                        sat_c[1] * spec.face_mul[1] + spec.face_add[1],
                        sat_c[2] * spec.face_mul[2] + spec.face_add[2],
                    ];
                    (mix3(c, treated, spec.face_opacity), ab[3])
                } else {
                    let mut c = base_face;
                    if d >= -22.0 {
                        let meniscus = lens(d, 1.0, 1.0 / 20.0);
                        let top_catch = (-n[1]).max(0.0);
                        c = mix3(
                            c,
                            [1.0, 1.0, 1.0],
                            meniscus * if spec.light { 0.09 } else { 0.06 } * top_catch,
                        );
                    }
                    (c, 1.0)
                };

                if spec.bleed_amount > 0.005 {
                    let ramp = sat((d + 10.0) / 10.0);
                    let darken = 0.7 * sat(dot3(face, [0.2125, 0.7154, 0.0721])) + 0.3;
                    let bleed_w = sq(sq(darken) * ramp) * spec.bleed_amount;
                    if bleed_w > 0.004 {
                        if let Some(rgb) = spec.bleed_rgb {
                            face = mix3(face, rgb, bleed_w);
                        } else if let Some(pyr) = pyramid {
                            let bdisp = lens(d, 34.0 * spec.refract_scale, 1.0 / 12.0);
                            let bs = pyr.sample_srgb(
                                lx + n[0] * bdisp,
                                ly + n[1] * bdisp,
                                blur_lod(2.0 * scale),
                            );
                            face = mix3(face, [bs[0], bs[1], bs[2]], bleed_w);
                        }
                    }
                }

                let h = key_fill(
                    d,
                    n,
                    KEY_LIGHT,
                    spec.key_height,
                    spec.key_spread,
                    spec.key_amount,
                    aa,
                );
                face = highlight_color(face, h, spec.key_bias);
                // Decoupled Fresnel rim & directional specular catch so even translucent glass keeps crisp 3D bevels.
                let rim_cov = coverage((d + 0.45).abs() - 0.48, aa);
                let mut dir_shine = 0.0f32;
                if rim_cov > 0.01 {
                    dir_shine = sq(dot2(n, RIM_LIGHT).max(0.0));
                    if spec.light {
                        let back_shade = sq((-dot2(n, RIM_LIGHT)).max(0.0));
                        face = mix3(face, [0.12, 0.16, 0.24], rim_cov * 0.10 * back_shade);
                    }
                }
                let body_a = if pyramid.is_some() {
                    mix(
                        spec.face_opacity,
                        (spec.face_opacity + 0.36).min(0.92),
                        back_a * 0.85,
                    )
                } else {
                    spec.face_opacity
                };
                let spec_reflect = if pyramid.is_some() {
                    sat(h * spec.key_bias * 0.55 + rim_cov * (0.18 + 0.44 * dir_shine))
                } else {
                    sat(h * spec.key_bias * 0.40 + rim_cov * (0.06 + 0.26 * dir_shine))
                };
                if spec_reflect > 0.005 {
                    let spec_w = (spec_reflect * (1.0 - body_a * 0.5)).clamp(0.0, 0.85);
                    face = mix3(face, [1.0, 1.0, 1.0], spec_w);
                }

                let final_a =
                    (mask * alpha * sat(body_a + spec_reflect * (1.0 - body_a))).clamp(0.0, 1.0);
                composite_over_pargb(pixel, face, final_a);
            }
        }
    }
}

/// Shades the persistent refractive Liquid Glass droplet lens (exact `glass.wgsl::navigation` optics).
#[allow(clippy::too_many_arguments)]
pub fn shade_liquid_droplet(
    bits: *mut u8,
    surf_w: i32,
    surf_h: i32,
    scale: f32,
    bar_bounds: RectF,
    center: [f32; 2],
    half_size: [f32; 2],
    corner_r: f32,
    shown: f32,
    velocity: [f32; 2],
    pyramid: &BackdropPyramid,
    light: bool,
    alpha: f32,
) {
    if bits.is_null()
        || surf_w <= 0
        || surf_h <= 0
        || alpha <= 0.01
        || half_size[0] <= 1.0
        || half_size[1] <= 1.0
    {
        return;
    }
    let scale = scale.max(0.5);
    let aa = 1.0 / scale;
    let lift = shown.clamp(0.0, 1.0);
    let speed = len2(velocity);
    let stretch_pad = (speed * 0.025).min(42.0) + 28.0;
    let x0 = ((center[0] - half_size[0] - stretch_pad).max(bar_bounds.x - 16.0) * scale)
        .floor()
        .max(0.0) as i32;
    let y0 = ((center[1] - half_size[1] - 16.0).max(bar_bounds.y - 14.0) * scale)
        .floor()
        .max(0.0) as i32;
    let x1 = ((center[0] + half_size[0] + stretch_pad).min(bar_bounds.x + bar_bounds.w + 16.0)
        * scale)
        .ceil()
        .min(surf_w as f32) as i32;
    let y1 = ((center[1] + half_size[1] + 18.0).min(bar_bounds.y + bar_bounds.h + 16.0) * scale)
        .ceil()
        .min(surf_h as f32) as i32;

    let (tint, tint_mix) = if light {
        ([0.99, 0.995, 1.0], 0.30)
    } else {
        ([0.78, 0.80, 0.85], 0.18)
    };
    let shadow_rgb = if light {
        [0.05, 0.08, 0.15]
    } else {
        [0.0, 0.0, 0.0]
    };
    let lod = blur_lod((1.4 + 0.6 * lift) * scale);
    let geom = DropletGeom::new(center, half_size, corner_r, lift, velocity);
    let stride = (surf_w * 4) as usize;
    unsafe {
        for py in y0..y1 {
            let ly = (py as f32 + 0.5) / scale;
            let row_ptr = bits.add((py as usize) * stride);
            for px_x in x0..x1 {
                let lx = (px_x as f32 + 0.5) / scale;
                let bd = geom.sdf([lx, ly]);
                if bd > 12.0 {
                    continue;
                }
                let bm = coverage(bd, aa) * alpha;
                let pixel = std::slice::from_raw_parts_mut(row_ptr.add((px_x as usize) * 4), 4);

                // Under-droplet occlusion + contact ring shadow
                if bd > -aa {
                    let sd = geom.sdf([lx, ly - (2.2 + 1.8 * lift)]);
                    let sh = (0.13 + 0.08 * lift)
                        * alpha
                        * sat(shadow_cdf(sd / (4.5 + 1.5 * lift)))
                        * (1.0 - coverage(bd, aa));
                    let rg = ring_shadow(
                        bd,
                        2.0,
                        1.5,
                        if light {
                            0.16 + 0.04 * lift
                        } else {
                            0.19 + 0.04 * lift
                        },
                    ) * alpha
                        * (1.0 - coverage(bd, aa));
                    let shadow_a = sat(sh + rg * (1.0 - sh));
                    if shadow_a > 0.003 {
                        composite_over_pargb(pixel, shadow_rgb, shadow_a);
                    }
                }

                if bd >= aa {
                    continue;
                }

                let n = geom.normal([lx, ly]);
                let disp = lens(bd, 12.5 + 4.5 * lift, 1.0 / 16.0);
                let rx = lx + n[0] * disp;
                let ry = ly + n[1] * disp;
                let ab = 0.85 + 0.40 * lift;
                let refracted = pyramid.aberrate_srgb(rx, ry, [n[0] * ab, n[1] * ab], lod);
                let mut glass = mix3([refracted[0], refracted[1], refracted[2]], tint, tint_mix);
                let h = key_fill(bd, n, KEY_LIGHT, 2.4, 0.06, 0.45, aa);
                glass = highlight_color(glass, h, if light { 0.56 } else { 0.66 });

                let rim = coverage((bd + 0.65).abs() - 0.68, aa);
                let mut rim_shine = 0.0f32;
                if rim > 0.01 {
                    let shine = 0.28 + 0.62 * sq(dot2(n, RIM_LIGHT).max(0.0));
                    rim_shine = rim * shine;
                    if light {
                        let back_shade = sq((-dot2(n, RIM_LIGHT)).max(0.0));
                        glass = mix3(glass, [0.14, 0.18, 0.26], rim * 0.12 * back_shade);
                        glass = mix3(glass, [1.0, 1.0, 1.0], rim * (0.18 + 0.50 * shine));
                    } else {
                        glass = mix3(glass, [1.0, 1.0, 1.0], rim * shine);
                    }
                }

                let body_a = mix(if light { 0.52 } else { 0.48 }, 0.88, refracted[3] * 0.85);
                let edge_a = sat(h * 0.45 + rim_shine);
                let final_a = (bm * sat(body_a + edge_a * (1.0 - body_a))).clamp(0.0, 1.0);
                composite_over_pargb(pixel, glass, final_a);
            }
        }
    }
}

/// Authored press-and-slide spring navigation controller (adapted from `liquid-glass-rs/src/navigation.rs`).
pub struct LiquidNav {
    pub selected: usize,
    pub pressed: bool,
    slots: Vec<(f32, f32)>, // (center_x, width)
    center_y: f32,
    half_h: f32,
    min_x: f32,
    max_x: f32,
    position: [f32; 2],
    velocity: [f32; 2],
    target: [f32; 2],
    shown: f32,
    linger: f32,
}

impl LiquidNav {
    pub fn new(slots: &[(f32, f32)], center_y: f32, half_h: f32, initial: usize) -> Self {
        let mut nav = Self {
            selected: 0,
            pressed: false,
            slots: Vec::new(),
            center_y,
            half_h,
            min_x: 0.0,
            max_x: 0.0,
            position: [0.0, center_y],
            velocity: [0.0; 2],
            target: [0.0, center_y],
            shown: 0.0,
            linger: 0.0,
        };
        nav.set_layout(slots, center_y, half_h);
        let idx = initial.min(nav.slots.len().saturating_sub(1));
        nav.selected = idx;
        let cx = nav.slot_x(idx);
        nav.position = [cx, center_y];
        nav.target = [cx, center_y];
        nav
    }

    pub fn set_layout(&mut self, slots: &[(f32, f32)], center_y: f32, half_h: f32) {
        self.slots.clear();
        self.slots.extend_from_slice(slots);
        self.center_y = center_y;
        self.half_h = half_h.max(8.0);
        if let (Some(first), Some(last)) = (self.slots.first(), self.slots.last()) {
            self.min_x = first.0 - first.1 * 0.25;
            self.max_x = last.0 + last.1 * 0.25;
        }
        if !self.slots.is_empty() {
            self.selected = self.selected.min(self.slots.len() - 1);
            let cx = self.slot_x(self.selected);
            if !self.pressed {
                self.target = [cx, center_y];
                if self.shown == 0.0 && self.velocity[0].abs() < 0.1 {
                    self.position = [cx, center_y];
                }
            }
        }
    }

    pub fn sync_layout(
        &mut self,
        slots: &[(f32, f32)],
        center_y: f32,
        half_h: f32,
        active_rel: usize,
    ) {
        let was_empty = self.slots.is_empty();
        self.set_layout(slots, center_y, half_h);
        self.position[1] = center_y;
        self.target[1] = center_y;
        if was_empty && !self.slots.is_empty() {
            let idx = active_rel.min(self.slots.len() - 1);
            self.selected = idx;
            let cx = self.slot_x(idx);
            self.position = [cx, center_y];
            self.target = [cx, center_y];
        } else if !self.pressed && !self.slots.is_empty() {
            let idx = active_rel.min(self.slots.len() - 1);
            if idx != self.selected {
                self.select(idx, true);
            }
        }
    }

    pub fn slot_x(&self, idx: usize) -> f32 {
        self.slots.get(idx).map(|s| s.0).unwrap_or(self.position[0])
    }

    pub fn slot_half_w(&self, idx: usize) -> f32 {
        self.slots.get(idx).map(|s| s.1 * 0.5).unwrap_or(28.0)
    }

    pub fn nearest(&self, x: f32) -> usize {
        self.slots
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.0 - x)
                    .abs()
                    .partial_cmp(&(b.0 - x).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    pub fn press(&mut self, p: [f32; 2]) -> bool {
        if self.slots.is_empty() {
            return false;
        }
        let left = self.slots.first().map(|s| s.0 - s.1 * 0.5).unwrap_or(0.0);
        let right = self.slots.last().map(|s| s.0 + s.1 * 0.5).unwrap_or(0.0);
        if p[0] < left - 4.0
            || p[0] > right + 4.0
            || (p[1] - self.center_y).abs() > self.half_h + 4.0
        {
            return false;
        }
        self.pressed = true;
        self.move_to(p);
        self.shown = self.shown.max(0.18);
        true
    }

    pub fn move_to(&mut self, p: [f32; 2]) {
        if self.pressed && !self.slots.is_empty() {
            self.target = [p[0].clamp(self.min_x, self.max_x), self.center_y];
        }
    }

    pub fn release(&mut self) -> Option<usize> {
        if !self.pressed {
            return None;
        }
        let target_idx = self.nearest(self.target[0]);
        self.select(target_idx, true);
        Some(target_idx)
    }

    pub fn select(&mut self, idx: usize, raise_droplet: bool) {
        if self.slots.is_empty() {
            return;
        }
        self.selected = idx.min(self.slots.len() - 1);
        self.pressed = false;
        self.target = [self.slot_x(self.selected), self.center_y];
        if raise_droplet {
            self.linger = 0.24;
            self.shown = self.shown.max(0.35);
        }
    }

    pub fn cancel(&mut self) {
        self.pressed = false;
        self.target = [self.slot_x(self.selected), self.center_y];
        self.linger = 0.0;
    }

    pub fn update(&mut self, elapsed: f32) {
        let dt = elapsed.clamp(0.0, 0.05);
        if dt <= 0.0 {
            return;
        }
        let steps = (dt * 240.0).ceil().max(1.0) as u32;
        let h = dt / steps as f32;
        for _ in 0..steps {
            for axis in 0..2 {
                self.velocity[axis] += ((self.target[axis] - self.position[axis]) * 620.0
                    - self.velocity[axis] * 34.0)
                    * h;
                self.position[axis] += self.velocity[axis] * h;
            }
            self.linger = (self.linger - h).max(0.0);
            let visible = if self.pressed || self.linger > 0.0 {
                1.0
            } else {
                0.0
            };
            self.shown += (visible - self.shown) * (1.0 - (-h * 22.0).exp());
        }
        if !self.pressed
            && self.linger == 0.0
            && self.shown < 0.002
            && (self.position[0] - self.target[0]).abs() < 0.15
            && len2(self.velocity) < 1.5
        {
            self.shown = 0.0;
            self.position = self.target;
            self.velocity = [0.0; 2];
        }
    }

    pub fn animating(&self) -> bool {
        self.pressed
            || self.linger > 0.0
            || self.shown > 0.0
            || (self.position[0] - self.target[0]).abs() > 0.15
            || len2(self.velocity) > 1.5
    }

    /// Returns `(center, half_width, shown, velocity, nearest_idx)`.
    pub fn state(&self) -> ([f32; 2], f32, f32, [f32; 2], usize) {
        let nearest_idx = self.nearest(self.target[0]);
        let hw = if !self.slots.is_empty() {
            // Interpolate slot width smoothly under the capsule center
            let cx = self.position[0];
            let mut hw = self.slot_half_w(nearest_idx);
            for w in self.slots.windows(2) {
                let (x0, w0) = w[0];
                let (x1, w1) = w[1];
                if cx >= x0 && cx <= x1 && (x1 - x0).abs() > 0.5 {
                    let t = (cx - x0) / (x1 - x0);
                    hw = mix(w0, w1, t) * 0.5;
                    break;
                }
            }
            hw
        } else {
            28.0
        };
        (self.position, hw, self.shown, self.velocity, nearest_idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Subset + full 75 samples from QuartzCore `glass_background_all_lpf` (research/native-probe.json).
    // (mode: 0=refraction, 1=shadow, 2=highlight; distance; native_apple_output)
    const NATIVE_PROBE_75: &[(u8, f32, f32)] = &[
        (0, -25.5, 0.400_390_63),
        (0, -24.5, 0.404_296_88),
        (0, -23.5, 0.408_218_38),
        (0, -22.5, 0.412_292_48),
        (0, -21.5, 0.416_519_17),
        (0, -20.5, 0.420_928_96),
        (0, -19.5, 0.425_491_33),
        (0, -18.5, 0.430_236_82),
        (0, -17.5, 0.435_150_15),
        (0, -16.5, 0.440_246_58),
        (0, -15.5, 0.445_526_12),
        (0, -14.5, 0.451_019_3),
        (0, -13.5, 0.456_710_82),
        (0, -12.5, 0.462_631_23),
        (0, -11.5, 0.468_795_78),
        (0, -10.5, 0.475_219_73),
        (0, -9.5, 0.481_933_6),
        (0, -8.5, 0.488_967_9),
        (0, -7.5, 0.496_368_4),
        (0, -6.5, 0.504_196_17),
        (0, -5.5, 0.512_542_7),
        (0, -4.5, 0.521_514_9),
        (0, -3.5, 0.531_326_3),
        (0, -2.5, 0.542_327_9),
        (0, -1.5, 0.555_267_33),
        (0, -0.5, 0.572_753_9),
        (1, 0.5, 0.238_525_39),
        (1, 1.5, 0.215_087_89),
        (1, 2.5, 0.192_138_67),
        (1, 3.5, 0.170_532_23),
        (1, 4.5, 0.149_169_92),
        (1, 5.5, 0.129_150_39),
        (1, 6.5, 0.110_900_88),
        (1, 7.5, 0.093_75),
        (1, 8.5, 0.078_308_105),
        (1, 9.5, 0.064_880_37),
        (1, 10.5, 0.052_825_928),
        (1, 11.5, 0.042_572_02),
        (1, 12.5, 0.033_813_477),
        (1, 13.5, 0.026_840_21),
        (1, 14.5, 0.021_026_611),
        (1, 15.5, 0.016_098_022),
        (1, 16.5, 0.012_329_102),
        (1, 17.5, 0.009_353_638),
        (1, 18.5, 0.006_652_832),
        (1, 19.5, 0.004_425_049),
        (1, 20.5, 0.002_655_029_3),
        (1, 21.5, 0.000_927_925_1),
        (1, 22.5, 0.000_061_035_156),
        (2, -25.5, 0.400_390_63),
        (2, -24.5, 0.404_296_88),
        (2, -23.5, 0.408_203_13),
        (2, -22.5, 0.412_109_38),
        (2, -21.5, 0.416_015_63),
        (2, -20.5, 0.419_921_88),
        (2, -19.5, 0.423_828_13),
        (2, -18.5, 0.427_734_38),
        (2, -17.5, 0.431_640_63),
        (2, -16.5, 0.435_546_88),
        (2, -15.5, 0.439_453_13),
        (2, -14.5, 0.443_359_38),
        (2, -13.5, 0.447_265_63),
        (2, -12.5, 0.451_171_88),
        (2, -11.5, 0.455_078_13),
        (2, -10.5, 0.458_984_38),
        (2, -9.5, 0.462_890_63),
        (2, -8.5, 0.466_796_88),
        (2, -7.5, 0.605_650_96),
        (2, -6.5, 0.661_171_26),
        (2, -5.5, 0.722_071_95),
        (2, -4.5, 0.789_161_56),
        (2, -3.5, 0.863_421_2),
        (2, -2.5, 0.946_053_1),
        (2, -1.5, 1.038_547),
        (2, -0.5, 1.142_771_1),
    ];

    #[test]
    fn equations_match_75_native_quartzcore_samples() {
        assert_eq!(NATIVE_PROBE_75.len(), 75);
        for &(mode, d, apple) in NATIVE_PROBE_75 {
            let answer = match mode {
                0 => (d + 128.0 + lens(d, 24.0, 1.0 / 24.0)) / 256.0,
                1 => 0.5 * shadow_cdf(d / 12.0),
                _ => {
                    let h = key_fill(d, [1.0, 0.0], [1.0, 0.0], 8.0, 0.12, 0.7, 1.0);
                    let base = (d + 128.0) / 256.0;
                    highlight_color([base, base, base], h, 0.7)[0]
                }
            };
            let err = (answer - apple).abs();
            let tol = if mode == 1 { 0.001 } else { 0.0001 };
            assert!(
                answer.is_finite() && err < tol,
                "mode {mode} d={d}: got {answer}, apple {apple}, err {err}"
            );
        }
    }

    #[test]
    fn liquid_nav_press_drag_release_and_cancel_settle_across_fps() {
        let slots = [(428.0, 94.0), (560.0, 94.0), (692.0, 94.0)];
        for fps in [30, 60, 120] {
            let mut nav = LiquidNav::new(&slots, 632.0, 28.0, 0);
            assert!(!nav.press([320.0, 632.0]));
            assert!(nav.press([428.0, 632.0]));
            for _ in 0..fps / 4 {
                nav.update(1.0 / fps as f32);
            }
            let (_, _, shown, _, _) = nav.state();
            assert!(shown > 0.98, "shown={shown}");

            nav.move_to([692.0, 670.0]);
            nav.update(1.0 / fps as f32);
            let (pos, _, _, vel, _) = nav.state();
            assert!(vel[0] > 0.0 && pos[0] > 428.0 && pos[0] < 692.0);
            assert_eq!(pos[1], 632.0);

            assert_eq!(nav.release(), Some(2));
            assert_eq!(nav.selected, 2);
            for _ in 0..fps * 2 {
                nav.update(1.0 / fps as f32);
            }
            assert!(!nav.animating());
            let (pos_end, _, shown_end, _, _) = nav.state();
            assert_eq!(pos_end, [692.0, 632.0]);
            assert_eq!(shown_end, 0.0);

            assert!(nav.press([560.0, 632.0]));
            nav.move_to([-1000.0, -1000.0]);
            nav.cancel();
            for _ in 0..fps * 2 {
                nav.update(1.0 / fps as f32);
            }
            assert_eq!(nav.selected, 2);
            assert!(!nav.pressed && !nav.animating());
        }
    }

    #[test]
    fn backdrop_pyramid_and_shaders_produce_valid_pargb() {
        let (w, h) = (160, 96);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                buf[i] = ((x * 255 / w) as u8).max(40);
                buf[i + 1] = ((y * 255 / h) as u8).max(50);
                buf[i + 2] = 90;
                buf[i + 3] = 255;
            }
        }
        let mut pyr = BackdropPyramid::default();
        assert!(pyr.capture(
            buf.as_ptr(),
            w,
            h,
            1.0,
            RectF::new(16.0, 12.0, 128.0, 72.0),
            14,
            5,
        ));
        shade_rounded_glass(
            buf.as_mut_ptr(),
            w,
            h,
            1.0,
            RectF::new(16.0, 12.0, 128.0, 72.0),
            18.0,
            &GlassSpec::panel(false),
            Some(&pyr),
            None,
            1.0,
        );
        assert!(pyr.capture(
            buf.as_ptr(),
            w,
            h,
            1.0,
            RectF::new(16.0, 12.0, 128.0, 72.0),
            16,
            3,
        ));
        shade_liquid_droplet(
            buf.as_mut_ptr(),
            w,
            h,
            1.0,
            RectF::new(16.0, 12.0, 128.0, 72.0),
            [80.0, 48.0],
            [28.0, 18.0],
            18.0,
            0.9,
            [320.0, 0.0],
            &pyr,
            false,
            1.0,
        );
        for px in buf.as_chunks::<4>().0 {
            assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3]);
        }
    }

    #[test]
    fn frosted_presets_are_neutral_and_light_shell_is_bright() {
        let (w, h) = (120, 64);
        let center_idx = ((32 * w + 60) * 4) as usize;

        // Light mode shell without backdrop pyramid must render bright neutral Frosted Alabaster
        let mut light_shell = vec![0u8; (w * h * 4) as usize];
        shade_rounded_glass(
            light_shell.as_mut_ptr(),
            w,
            h,
            1.0,
            RectF::new(10.0, 8.0, 100.0, 48.0),
            18.0,
            &GlassSpec::shell(true, None),
            None,
            None,
            1.0,
        );
        let la = light_shell[center_idx + 3].max(1) as i32;
        let lr = light_shell[center_idx + 2] as i32 * 255 / la;
        let lg = light_shell[center_idx + 1] as i32 * 255 / la;
        let lb = light_shell[center_idx] as i32 * 255 / la;
        assert!(
            lr > 220 && lg > 220 && lb > 220,
            "light shell too dark: ({lr},{lg},{lb})"
        );
        assert!(
            (lg - lr).abs() <= 6 && (lb - lg).abs() <= 12,
            "light shell not neutral: ({lr},{lg},{lb})"
        );

        // Dark mode shell without pyramid must render deep neutral Graphite
        let mut dark_shell = vec![0u8; (w * h * 4) as usize];
        shade_rounded_glass(
            dark_shell.as_mut_ptr(),
            w,
            h,
            1.0,
            RectF::new(10.0, 8.0, 100.0, 48.0),
            18.0,
            &GlassSpec::shell(false, None),
            None,
            None,
            1.0,
        );
        let da = dark_shell[center_idx + 3].max(1) as i32;
        let dr = dark_shell[center_idx + 2] as i32 * 255 / da;
        let dg = dark_shell[center_idx + 1] as i32 * 255 / da;
        let db = dark_shell[center_idx] as i32 * 255 / da;
        assert!(
            dr < 55 && dg < 55 && db < 55,
            "dark shell too bright: ({dr},{dg},{db})"
        );
        assert!(
            (dg - dr).abs() <= 6 && (db - dg).abs() <= 12,
            "dark shell not neutral: ({dr},{dg},{db})"
        );
    }

    #[test]
    fn translucent_pill_and_persistent_droplet_produce_valid_pargb() {
        let (w, h) = (160, 72);
        let center_idx = ((36 * w + 80) * 4) as usize;
        for light in [true, false] {
            let mut buf = vec![0u8; (w * h * 4) as usize];
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * w + x) * 4) as usize;
                    let stripe = if (x / 12) % 2 == 0 { 16 } else { -12 };
                    if light {
                        let base = (232i32 + stripe).clamp(0, 255) as u8;
                        buf[i] = base;
                        buf[i + 1] = base;
                        buf[i + 2] = base;
                        buf[i + 3] = 255;
                    } else {
                        let base = (32i32 + stripe).clamp(0, 255) as u8;
                        buf[i] = base;
                        buf[i + 1] = base;
                        buf[i + 2] = base;
                        buf[i + 3] = 255;
                    }
                }
            }
            let mut pyr = BackdropPyramid::default();
            let rect = RectF::new(16.0, 12.0, 128.0, 48.0);
            assert!(pyr.capture(buf.as_ptr(), w, h, 1.0, rect, 14, 3));
            let spec = GlassSpec::pill(light, None, true, 0.25);
            shade_rounded_glass(
                buf.as_mut_ptr(),
                w,
                h,
                1.0,
                rect,
                24.0,
                &spec,
                Some(&pyr),
                None,
                1.0,
            );
            assert!(pyr.capture(buf.as_ptr(), w, h, 1.0, rect, 16, 3));
            // Both resting (shown=0.0) and gliding (shown=0.9) droplets must render cleanly
            for shown in [0.0, 0.9] {
                shade_liquid_droplet(
                    buf.as_mut_ptr(),
                    w,
                    h,
                    1.0,
                    rect,
                    [80.0, 36.0],
                    [34.0, 18.0],
                    18.0,
                    shown,
                    if shown > 0.1 {
                        [280.0, 0.0]
                    } else {
                        [0.0, 0.0]
                    },
                    &pyr,
                    light,
                    1.0,
                );
            }
            for px in buf.as_chunks::<4>().0 {
                assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3]);
            }
            let a = buf[center_idx + 3].max(1) as i32;
            let r = buf[center_idx + 2] as i32 * 255 / a;
            let g = buf[center_idx + 1] as i32 * 255 / a;
            let b = buf[center_idx] as i32 * 255 / a;
            assert!(
                (g - r).abs() <= 10 && (b - g).abs() <= 14,
                "droplet/pill not neutral (light={light}): ({r},{g},{b})"
            );
            if light {
                assert!(r > 195, "light pill+droplet too dark: {r}");
            } else {
                assert!(r < 110, "dark pill+droplet too bright: {r}");
            }
        }
    }
}
