use moxcms::{ColorProfile, Layout, TransformOptions};

#[derive(Debug, Clone)]
pub struct DisplayProfile {
    pub name: String,
    pub icc_data: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct DisplayLut3d {
    pub size: u32,
    /// RGBA16-float values with X varying fastest, then Y, then Z.
    pub rgba: Vec<u16>,
}

/// Resolve the ICC profile assigned to the foreground Halation window's monitor.
pub fn discover_display_profile() -> DisplayProfile {
    if let Some(profile) = profile_from_environment() {
        return profile;
    }
    static LAST_PROFILE: std::sync::OnceLock<std::sync::Mutex<Option<DisplayProfile>>> =
        std::sync::OnceLock::new();
    let cache = LAST_PROFILE.get_or_init(|| std::sync::Mutex::new(None));
    if let Some(profile) = windows_display_profile() {
        if let Ok(mut previous) = cache.lock() {
            *previous = Some(profile.clone());
        }
        return profile;
    }
    if let Ok(previous) = cache.lock() {
        if let Some(profile) = previous.clone() {
            return profile;
        }
    }
    DisplayProfile {
        name: "sRGB fallback".into(),
        icc_data: None,
    }
}

fn profile_from_environment() -> Option<DisplayProfile> {
    let path = std::env::var_os("HALATION_DISPLAY_ICC")?;
    let path = std::path::PathBuf::from(path);
    let data = std::fs::read(&path).ok()?;
    Some(DisplayProfile {
        name: path.to_string_lossy().into_owned(),
        icc_data: Some(data),
    })
}

pub fn build_display_lut(profile: &DisplayProfile, size: u32) -> Result<DisplayLut3d, String> {
    if !(2..=65).contains(&size) {
        return Err("3D LUT size must be between 2 and 65".into());
    }
    let source = ColorProfile::new_srgb();
    let destination = match profile.icc_data.as_deref() {
        Some(bytes) => ColorProfile::new_from_slice(bytes)
            .map_err(|e| format!("Invalid display ICC profile: {e}"))?,
        None => ColorProfile::new_srgb(),
    };
    let transform = source
        .create_transform_f32(
            Layout::Rgb,
            &destination,
            Layout::Rgb,
            TransformOptions::default(),
        )
        .map_err(|e| format!("Could not create display color transform: {e}"))?;
    let sample_count = (size as usize).pow(3);
    let mut input = Vec::with_capacity(sample_count * 3);
    for z in 0..size {
        for y in 0..size {
            for x in 0..size {
                input.extend([
                    x as f32 / (size - 1) as f32,
                    y as f32 / (size - 1) as f32,
                    z as f32 / (size - 1) as f32,
                ]);
            }
        }
    }
    let mut converted = vec![0.0_f32; input.len()];
    transform
        .transform(&input, &mut converted)
        .map_err(|e| format!("Display LUT transform failed: {e}"))?;
    let mut rgba = Vec::with_capacity(sample_count * 4);
    for rgb in converted.chunks_exact(3) {
        rgba.extend([
            f32_to_f16(rgb[0].clamp(0.0, 1.0)),
            f32_to_f16(rgb[1].clamp(0.0, 1.0)),
            f32_to_f16(rgb[2].clamp(0.0, 1.0)),
            f32_to_f16(1.0),
        ]);
    }
    Ok(DisplayLut3d { size, rgba })
}

fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x7f_ffff;
    if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        let significand = mantissa | 0x80_0000;
        let shift = (14 - exponent) as u32;
        return sign | ((significand + (1 << (shift - 1))) >> shift) as u16;
    }
    if exponent >= 31 {
        return sign | 0x7bff;
    }
    sign | ((exponent as u16) << 10) | (((mantissa + 0x1000) >> 13) as u16)
}

fn windows_display_profile() -> Option<DisplayProfile> {
    use std::path::PathBuf;
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::HWND,
            Graphics::Gdi::{GetDC, ReleaseDC},
            UI::{
                ColorSystem::GetICMProfileW,
                WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
            },
        },
    };
    unsafe {
        let window = GetForegroundWindow();
        let window = if window.0.is_null() {
            HWND::default()
        } else {
            window
        };
        if !window.0.is_null() {
            let mut process_id = 0_u32;
            GetWindowThreadProcessId(window, Some(&mut process_id));
            if process_id != std::process::id() {
                return None;
            }
        }
        let dc = GetDC(window);
        if dc.0.is_null() {
            return None;
        }
        let mut length = 0_u32;
        let _ = GetICMProfileW(dc, &mut length, PWSTR(std::ptr::null_mut()));
        if length == 0 || length > 32768 {
            let _ = ReleaseDC(window, dc);
            return None;
        }
        let mut buffer = vec![0_u16; length as usize];
        let ok = GetICMProfileW(dc, &mut length, PWSTR(buffer.as_mut_ptr())).as_bool();
        let _ = ReleaseDC(window, dc);
        if !ok {
            return None;
        }
        buffer.truncate(length.saturating_sub(1) as usize);
        let path = PathBuf::from(String::from_utf16_lossy(&buffer));
        static PROFILE_CACHE: std::sync::OnceLock<
            std::sync::Mutex<Option<(PathBuf, Option<std::time::SystemTime>, DisplayProfile)>>,
        > = std::sync::OnceLock::new();
        let cache = PROFILE_CACHE.get_or_init(|| std::sync::Mutex::new(None));
        let modified = std::fs::metadata(&path).ok()?.modified().ok();
        if let Ok(guard) = cache.lock() {
            if let Some((cached_path, cached_modified, profile)) = guard.as_ref() {
                if cached_path == &path && cached_modified == &modified {
                    return Some(profile.clone());
                }
            }
        }
        let icc_data = std::fs::read(&path).ok()?;
        let profile = DisplayProfile {
            name: path.file_name()?.to_string_lossy().into_owned(),
            icc_data: Some(icc_data),
        };
        if let Ok(mut guard) = cache.lock() {
            *guard = Some((path, modified, profile.clone()));
        }
        Some(profile)
    }
}
