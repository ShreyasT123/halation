//! Volume identity (Sprint 1): a photo is identified by `(volume_uuid, relative_path)` instead of an
//! absolute path, so catalogs on external drives survive drive-letter and mount-point changes.
//!
//! * Windows: volume serial from `GetVolumeInformationW`.
//! * macOS / Linux: `statfs.f_fsid`.
//!
//! Cargo: `libc = "0.2"` (unix), `windows = { version = "0.58", features = ["Win32_Storage_FileSystem"] }` (windows).

use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeLocation {
    pub uuid: String,
    pub root: PathBuf,
    /// Path below `root`, always with `/` separators.
    pub relative: String,
}

fn relative_to(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

// ------------------------------------------------------------------------------------------------
// Windows
// ------------------------------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn windows_root(path: &Path) -> Option<PathBuf> {
    use std::path::{Component, Prefix};
    match path.components().next()? {
        Component::Prefix(p) => match p.kind() {
            Prefix::Disk(l) | Prefix::VerbatimDisk(l) => Some(PathBuf::from(format!("{}:\\", l as char))),
            _ => Some(PathBuf::from(format!("{}\\", p.as_os_str().to_string_lossy()))),
        },
        _ => None,
    }
}

#[cfg(target_os = "windows")]
pub fn volume_uuid(path: &Path) -> io::Result<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;
    let root = windows_root(path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing volume root"))?;
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut serial = 0u32;
    // SAFETY: `wide` is NUL-terminated and outlives the call; `serial` is a valid out pointer.
    unsafe { GetVolumeInformationW(PCWSTR(wide.as_ptr()), None, Some(&mut serial), None, None, None) }
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(format!("WIN-{serial:08X}"))
}

#[cfg(target_os = "windows")]
pub fn mount_roots() -> Vec<(String, PathBuf)> {
    (b'A'..=b'Z')
        .filter_map(|l| {
            let root = PathBuf::from(format!("{}:\\", l as char));
            root.exists().then(|| volume_uuid(&root).ok().map(|u| (u, root))).flatten()
        })
        .collect()
}

#[cfg(target_os = "windows")]
pub fn locate(path: &Path) -> VolumeLocation {
    let root = windows_root(path).unwrap_or_else(|| PathBuf::from("\\"));
    VolumeLocation { uuid: volume_uuid(path).unwrap_or_else(|_| "UNKNOWN".into()), relative: relative_to(path, &root), root }
}

#[cfg(target_os = "windows")]
pub fn locate_in(path: &Path, _roots: &[(String, PathBuf)]) -> VolumeLocation {
    locate(path)
}

// ------------------------------------------------------------------------------------------------
// macOS / Linux
// ------------------------------------------------------------------------------------------------

#[cfg(unix)]
pub fn volume_uuid(path: &Path) -> io::Result<String> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // SAFETY: zeroed `statfs` is a valid out buffer; `c_path` is NUL-terminated.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c_path.as_ptr(), &mut stat) } != 0 {
        return Err(io::Error::last_os_error());
    }
    #[cfg(target_os = "macos")]
    let (a, b, tag) = (stat.f_fsid.val[0] as u32, stat.f_fsid.val[1] as u32, "MAC");
    #[cfg(not(target_os = "macos"))]
    let (a, b, tag) = (stat.f_fsid.__val[0] as u32, stat.f_fsid.__val[1] as u32, "LNX");
    Ok(format!("{tag}-{a:08X}{b:08X}"))
}

#[cfg(target_os = "macos")]
pub fn mount_roots() -> Vec<(String, PathBuf)> {
    let mut roots = vec![PathBuf::from("/")];
    roots.extend(std::fs::read_dir("/Volumes").into_iter().flatten().flatten().map(|e| e.path()));
    roots.into_iter().filter_map(|r| volume_uuid(&r).ok().map(|u| (u, r))).collect()
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn mount_roots() -> Vec<(String, PathBuf)> {
    const SKIP: [&str; 5] = ["/proc", "/sys", "/dev", "/run", "/snap"];
    let unescape = |s: &str| {
        let (mut out, bytes) = (String::new(), s.as_bytes());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 3 < bytes.len() + 0 && bytes[i + 1..i + 4].iter().all(u8::is_ascii_digit) {
                let v = u8::from_str_radix(&s[i + 1..i + 4], 8).unwrap_or(b'?');
                out.push(v as char);
                i += 4;
            } else {
                out.push(bytes[i] as char);
                i += 1;
            }
        }
        out
    };
    let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let mut roots: Vec<PathBuf> = text.lines().filter_map(|l| l.split_whitespace().nth(4)).map(|p| PathBuf::from(unescape(p))).collect();
    roots.retain(|r| !SKIP.iter().any(|s| r.starts_with(s)));
    roots.sort();
    roots.dedup();
    roots.into_iter().filter_map(|r| volume_uuid(&r).ok().map(|u| (u, r))).collect()
}

#[cfg(unix)]
pub fn locate(path: &Path) -> VolumeLocation {
    locate_in(path, &mount_roots())
}

/// Like [`locate`] with mount roots supplied by the caller (bulk backfills query them once).
#[cfg(unix)]
pub fn locate_in(path: &Path, roots: &[(String, PathBuf)]) -> VolumeLocation {
    let best = roots.iter().filter(|(_, root)| path.starts_with(root)).max_by_key(|(_, root)| root.components().count()).cloned();
    match best {
        Some((uuid, root)) => VolumeLocation { uuid, relative: relative_to(path, &root), root },
        None => VolumeLocation {
            uuid: volume_uuid(path).unwrap_or_else(|_| "UNKNOWN".into()),
            root: PathBuf::from("/"),
            relative: relative_to(path, Path::new("/")),
        },
    }
}

/// Where `relative` of volume `uuid` currently lives, if that volume is mounted and the file exists.
pub fn resolve(uuid: &str, relative: &str, roots: &[(String, PathBuf)]) -> Option<PathBuf> {
    roots.iter().filter(|(u, _)| u == uuid).map(|(_, root)| root.join(relative)).find(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_use_forward_slashes() {
        let root = Path::new("/mnt/photos");
        assert_eq!(relative_to(Path::new("/mnt/photos/2026/trip/IMG_1.ARW"), root), "2026/trip/IMG_1.ARW");
    }

    #[test]
    fn current_directory_has_a_volume() {
        let here = std::env::current_dir().unwrap();
        let loc = locate(&here);
        assert!(!loc.uuid.is_empty());
        assert_eq!(resolve(&loc.uuid, &loc.relative, &[(loc.uuid.clone(), loc.root.clone())]), Some(here));
    }
}
