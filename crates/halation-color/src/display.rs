//! Display colour management (Sprint 5): monitor ICC discovery, an in-house matrix/TRC profile parser
//! and the 33×33×33 working-space → display LUT with tetrahedral interpolation.
//!
//! LUT axis: `linear ProPhoto ^ (1 / SHAPER_GAMMA)`. Without a shaper a 33-point linear axis would put
//! only ~2 samples under 3 % reflectance and band the shadows. The LUT stores *display-encoded* RGB,
//! so the sRGB OETF is part of it.
//!
//! Matrix/TRC profiles (virtually every calibrated monitor profile) are parsed here with no extra
//! crates; LUT-only (A2B) profiles are reported as unsupported and the caller falls back to sRGB.
//!
//! Platform discovery (best effort, `None` ⇒ use sRGB):
//! * Windows — `windows` crate (features `Win32_Foundation`, `Win32_Graphics_Gdi`, `Win32_UI_ColorSystem`):
//!   monitor from point → device DC → `GetICMProfileW`.
//! * macOS — CoreGraphics (`CGDisplayCopyColorSpace` → `CGColorSpaceCopyICCData`), no crate needed.
//! * Linux/X11 — `x11rb` crate, root-window `_ICC_PROFILE` atom. Wayland (`color-management-v1`) is not
//!   exposed by winit yet and falls back to sRGB.

use super::{bradford_cat, prophoto_to_xyz_d50, srgb_eotf, srgb_oetf, srgb_to_xyz_d65, D50, D65, REC709_LUMA};
use glam::{Mat3, Vec3};

pub const DISPLAY_LUT_SIZE: usize = 33;
pub const SHAPER_GAMMA: f32 = 2.4;

// ------------------------------------------------------------------------------------------------
// Tone response curves
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Trc {
    Gamma(f32),
    Srgb,
    Table(Vec<f32>),
    Para { kind: u16, p: [f32; 7] }, // g a b c d e f
}

impl Trc {
    /// Encoded → linear.
    fn eval(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Trc::Gamma(g) => x.powf(*g),
            Trc::Srgb => srgb_eotf(x),
            Trc::Table(t) => {
                let pos = x * (t.len() - 1) as f32;
                let i = (pos as usize).min(t.len() - 2);
                t[i] + (t[i + 1] - t[i]) * (pos - i as f32)
            }
            Trc::Para { kind, p } => {
                let [g, a, b, c, d, e, f] = *p;
                let y = match kind {
                    0 => x.powf(g),
                    1 => if a != 0.0 && x >= -b / a { (a * x + b).max(0.0).powf(g) } else { 0.0 },
                    2 => if a != 0.0 && x >= -b / a { (a * x + b).max(0.0).powf(g) + c } else { c },
                    3 => if x >= d { (a * x + b).max(0.0).powf(g) } else { c * x },
                    _ => if x >= d { (a * x + b).max(0.0).powf(g) + e } else { c * x + f },
                };
                y.clamp(0.0, 1.0)
            }
        }
    }

    /// Linear → encoded (analytic where possible, bisection otherwise; all TRCs are monotone).
    fn inverse(&self, y: f32) -> f32 {
        let y = y.clamp(0.0, 1.0);
        match self {
            Trc::Gamma(g) => y.powf(1.0 / g.max(1.0e-3)),
            Trc::Srgb => srgb_oetf(y),
            _ => {
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..28 {
                    let mid = 0.5 * (lo + hi);
                    if self.eval(mid) < y {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                0.5 * (lo + hi)
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------
// ICC matrix / TRC profile
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DisplayProfile {
    pub description: String,
    to_pcs: Mat3, // device linear RGB → XYZ (D50)
    from_pcs: Mat3,
    trc: [Trc; 3],
}

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn s15f16(b: &[u8], at: usize) -> Option<f32> {
    be_u32(b, at).map(|v| v as i32 as f32 / 65536.0)
}

impl DisplayProfile {
    pub fn srgb() -> Self {
        let m = (bradford_cat(D65, D50) * srgb_to_xyz_d65()).as_mat3();
        Self { description: "sRGB (built-in)".into(), to_pcs: m, from_pcs: m.inverse(), trc: [Trc::Srgb, Trc::Srgb, Trc::Srgb] }
    }

    pub fn from_icc(icc: &[u8]) -> Result<Self, String> {
        if icc.len() < 132 || icc.get(36..40) != Some(b"acsp") {
            return Err("not an ICC profile".into());
        }
        if icc.get(16..20) != Some(b"RGB ") || icc.get(20..24) != Some(b"XYZ ") {
            return Err("only RGB profiles with an XYZ connection space are supported".into());
        }
        let count = be_u32(icc, 128).ok_or("truncated tag table")? as usize;
        let tag = |sig: &[u8; 4]| -> Option<&[u8]> {
            (0..count.min(256)).find_map(|i| {
                let e = 132 + i * 12;
                if icc.get(e..e + 4)? != sig {
                    return None;
                }
                let (off, len) = (be_u32(icc, e + 4)? as usize, be_u32(icc, e + 8)? as usize);
                icc.get(off..off.checked_add(len)?)
            })
        };
        let xyz = |sig: &[u8; 4]| -> Option<Vec3> {
            let t = tag(sig)?;
            (t.get(0..4)? == b"XYZ ").then_some(())?;
            Some(Vec3::new(s15f16(t, 8)?, s15f16(t, 12)?, s15f16(t, 16)?))
        };
        let (r, g, b) = (xyz(b"rXYZ"), xyz(b"gXYZ"), xyz(b"bXYZ"));
        let (Some(r), Some(g), Some(b)) = (r, g, b) else {
            return Err("profile has no matrix columns (LUT-based profiles are unsupported)".into());
        };
        let trc = |sig: &[u8; 4]| tag(sig).and_then(parse_trc).ok_or_else(|| "missing or unsupported TRC".to_owned());
        let to_pcs = Mat3::from_cols(r, g, b);
        if to_pcs.determinant().abs() < 1.0e-6 {
            return Err("degenerate primaries".into());
        }
        Ok(Self {
            description: tag(b"desc").and_then(parse_description).unwrap_or_else(|| "Display profile".into()),
            to_pcs,
            from_pcs: to_pcs.inverse(),
            trc: [trc(b"rTRC")?, trc(b"gTRC")?, trc(b"bTRC")?],
        })
    }

    /// Linear ProPhoto → display-encoded device RGB in [0, 1] (out-of-gamut colours are desaturated
    /// towards their luminance, then clamped).
    pub fn from_working(&self, rgb: Vec3) -> Vec3 {
        let xyz = prophoto_to_xyz_d50().as_mat3() * rgb;
        let mut d = self.from_pcs * xyz;
        let l = d.dot(Vec3::from_array(REC709_LUMA));
        let min = d.min_element();
        if min < 0.0 && l > 0.0 {
            d = Vec3::splat(l) + (d - Vec3::splat(l)) * (l / (l - min));
        }
        let d = d.clamp(Vec3::ZERO, Vec3::ONE);
        Vec3::new(self.trc[0].inverse(d.x), self.trc[1].inverse(d.y), self.trc[2].inverse(d.z))
    }
}

fn parse_trc(t: &[u8]) -> Option<Trc> {
    match t.get(0..4)? {
        b"curv" => {
            let n = be_u32(t, 8)? as usize;
            match n {
                0 => Some(Trc::Gamma(1.0)),
                1 => Some(Trc::Gamma(u16::from_be_bytes([*t.get(12)?, *t.get(13)?]) as f32 / 256.0)),
                _ => {
                    let table: Option<Vec<f32>> = (0..n.min(65536))
                        .map(|i| t.get(12 + i * 2..14 + i * 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as f32 / 65535.0))
                        .collect();
                    table.map(Trc::Table)
                }
            }
        }
        b"para" => {
            let kind = u16::from_be_bytes([*t.get(8)?, *t.get(9)?]);
            let n = match kind {
                0 => 1,
                1 => 3,
                2 => 4,
                3 => 5,
                4 => 7,
                _ => return None,
            };
            let mut p = [0.0f32; 7];
            for (i, slot) in p.iter_mut().enumerate().take(n) {
                *slot = s15f16(t, 12 + i * 4)?;
            }
            Some(Trc::Para { kind, p })
        }
        _ => None,
    }
}

fn parse_description(t: &[u8]) -> Option<String> {
    match t.get(0..4)? {
        b"desc" => {
            let n = be_u32(t, 8)? as usize;
            let s = t.get(12..12 + n.saturating_sub(1))?;
            Some(String::from_utf8_lossy(s).into_owned())
        }
        b"mluc" => {
            let (off, len) = (be_u32(t, 20)? as usize, be_u32(t, 24)? as usize);
            let s = t.get(off..off + len)?;
            let units: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

// ------------------------------------------------------------------------------------------------
// 3D LUT
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DisplayLut {
    pub description: String,
    pub size: usize,
    /// `size³` display-encoded RGB triples, index `r + size·(g + size·b)`.
    pub data: Vec<[f32; 3]>,
}

impl DisplayLut {
    pub fn build(profile: &DisplayProfile) -> Self {
        let n = DISPLAY_LUT_SIZE;
        let mut data = Vec::with_capacity(n * n * n);
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let lin = |i: usize| (i as f32 / (n - 1) as f32).powf(SHAPER_GAMMA);
                    data.push(profile.from_working(Vec3::new(lin(r), lin(g), lin(b))).to_array());
                }
            }
        }
        Self { description: profile.description.clone(), size: n, data }
    }

    pub fn srgb() -> Self {
        Self::build(&DisplayProfile::srgb())
    }

    pub fn from_icc(icc: &[u8]) -> Result<Self, String> {
        DisplayProfile::from_icc(icc).map(|p| Self::build(&p))
    }

    /// RGBA32F texels in LUT order, ready for a `texture_3d<f32>` upload.
    pub fn rgba32f(&self) -> Vec<f32> {
        self.data.iter().flat_map(|c| [c[0], c[1], c[2], 1.0]).collect()
    }

    fn at(&self, r: usize, g: usize, b: usize) -> Vec3 {
        Vec3::from_array(self.data[r + self.size * (g + self.size * b)])
    }

    /// Tetrahedral interpolation of linear working-space RGB → display-encoded RGB.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let u = Vec3::from_array(rgb).clamp(Vec3::ZERO, Vec3::ONE).powf(1.0 / SHAPER_GAMMA) * (n - 1) as f32;
        let (i0, f) = (
            [(u.x as usize).min(n - 2), (u.y as usize).min(n - 2), (u.z as usize).min(n - 2)],
            Vec3::new(u.x - (u.x as usize).min(n - 2) as f32, u.y - (u.y as usize).min(n - 2) as f32, u.z - (u.z as usize).min(n - 2) as f32),
        );
        let c = |dr: usize, dg: usize, db: usize| self.at(i0[0] + dr, i0[1] + dg, i0[2] + db);
        let (c000, c111) = (c(0, 0, 0), c(1, 1, 1));
        let out = if f.x >= f.y {
            if f.y >= f.z {
                c000 + (c(1, 0, 0) - c000) * f.x + (c(1, 1, 0) - c(1, 0, 0)) * f.y + (c111 - c(1, 1, 0)) * f.z
            } else if f.x >= f.z {
                c000 + (c(1, 0, 0) - c000) * f.x + (c(1, 0, 1) - c(1, 0, 0)) * f.z + (c111 - c(1, 0, 1)) * f.y
            } else {
                c000 + (c(0, 0, 1) - c000) * f.z + (c(1, 0, 1) - c(0, 0, 1)) * f.x + (c111 - c(1, 0, 1)) * f.y
            }
        } else if f.z >= f.y {
            c000 + (c(0, 0, 1) - c000) * f.z + (c(0, 1, 1) - c(0, 0, 1)) * f.y + (c111 - c(0, 1, 1)) * f.x
        } else if f.z >= f.x {
            c000 + (c(0, 1, 0) - c000) * f.y + (c(0, 1, 1) - c(0, 1, 0)) * f.z + (c111 - c(0, 1, 1)) * f.x
        } else {
            c000 + (c(0, 1, 0) - c000) * f.y + (c(1, 1, 0) - c(0, 1, 0)) * f.x + (c111 - c(1, 1, 0)) * f.z
        };
        out.clamp(Vec3::ZERO, Vec3::ONE).to_array()
    }
}

// ------------------------------------------------------------------------------------------------
// Platform discovery
// ------------------------------------------------------------------------------------------------

impl DisplayProfile {
    /// Raw ICC bytes of the display under `window_center` (global screen coordinates), if the OS exposes one.
    pub fn query_icc(window_center: Option<(i32, i32)>) -> Option<Vec<u8>> {
        platform_icc(window_center)
    }

    /// Discover + parse; `None` when the OS reports no profile or it is not a matrix/TRC profile.
    pub fn discover(window_center: Option<(i32, i32)>) -> Option<Self> {
        Self::from_icc(&Self::query_icc(window_center)?).ok()
    }
}

#[cfg(target_os = "windows")]
fn platform_icc(pos: Option<(i32, i32)>) -> Option<Vec<u8>> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{CreateDCW, DeleteDC, GetMonitorInfoW, MonitorFromPoint, MONITORINFOEXW, MONITOR_DEFAULTTOPRIMARY};
    use windows::Win32::UI::ColorSystem::GetICMProfileW;
    let (x, y) = pos.unwrap_or((0, 0));
    // SAFETY: plain Win32 calls with correctly sized, zero-initialised out parameters.
    let path = unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(monitor, &mut info as *mut MONITORINFOEXW as *mut _).as_bool() {
            return None;
        }
        let device = PCWSTR(info.szDevice.as_ptr());
        let hdc = CreateDCW(device, device, PCWSTR::null(), None);
        if hdc.is_invalid() {
            return None;
        }
        let mut len = 0u32;
        let _ = GetICMProfileW(hdc, &mut len, PWSTR::null());
        let mut buf = vec![0u16; len.max(1) as usize];
        let ok = GetICMProfileW(hdc, &mut len, PWSTR(buf.as_mut_ptr())).as_bool();
        let _ = DeleteDC(hdc);
        if !ok {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    };
    std::fs::read(path).ok()
}

#[cfg(target_os = "macos")]
fn platform_icc(pos: Option<(i32, i32)>) -> Option<Vec<u8>> {
    use std::ffi::c_void;
    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGGetDisplaysWithPoint(point: CGPoint, max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayCopyColorSpace(display: u32) -> *const c_void;
        fn CGColorSpaceCopyICCData(space: *const c_void) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDataGetLength(data: *const c_void) -> isize;
        fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
        fn CFRelease(object: *const c_void);
    }
    // SAFETY: CoreGraphics / CoreFoundation calls follow their documented ownership rules (Copy ⇒ CFRelease).
    unsafe {
        let mut display = CGMainDisplayID();
        if let Some((x, y)) = pos {
            let (mut id, mut count) = (0u32, 0u32);
            if CGGetDisplaysWithPoint(CGPoint { x: x as f64, y: y as f64 }, 1, &mut id, &mut count) == 0 && count > 0 {
                display = id;
            }
        }
        let space = CGDisplayCopyColorSpace(display);
        if space.is_null() {
            return None;
        }
        let data = CGColorSpaceCopyICCData(space);
        CFRelease(space);
        if data.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data).max(0) as usize).to_vec();
        CFRelease(data);
        Some(bytes)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_icc(_pos: Option<(i32, i32)>) -> Option<Vec<u8>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let atom = conn.intern_atom(false, b"_ICC_PROFILE").ok()?.reply().ok()?.atom;
    let reply = conn.get_property(false, root, atom, AtomEnum::CARDINAL, 0, u32::MAX / 4).ok()?.reply().ok()?;
    (!reply.value.is_empty()).then_some(reply.value)
}

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
fn platform_icc(_pos: Option<(i32, i32)>) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_lut_reproduces_the_analytic_transform() {
        let lut = DisplayLut::srgb();
        let to_srgb = super::super::working_to_srgb();
        for rgb in [[0.18f32; 3], [0.5, 0.2, 0.1], [0.02, 0.4, 0.9], [1.0; 3], [0.0; 3], [0.001; 3]] {
            let got = lut.apply(rgb);
            let lin = (to_srgb * Vec3::from_array(rgb)).clamp(Vec3::ZERO, Vec3::ONE);
            for k in 0..3 {
                assert!((got[k] - srgb_oetf(lin[k])).abs() < 0.006, "{rgb:?}: {got:?}");
            }
        }
    }

    #[test]
    fn lut_is_monotone_on_the_neutral_axis() {
        let lut = DisplayLut::srgb();
        let mut prev = -1.0;
        for i in 0..=200 {
            let v = lut.apply([i as f32 / 200.0; 3])[1];
            assert!(v >= prev - 1.0e-6);
            prev = v;
        }
    }

    #[test]
    fn rejects_non_icc_data() {
        assert!(DisplayProfile::from_icc(&[0u8; 200]).is_err());
    }
}
