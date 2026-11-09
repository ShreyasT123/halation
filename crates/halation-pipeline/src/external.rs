//! External editor hand-off (Sprint 8): "Open in External Editor" (Ctrl/Cmd+Alt+E).
//!
//! 1. Develop the photo at full resolution into a 16-bit **ProPhoto RGB** TIFF (profile embedded) in a
//!    per-photo scratch directory.
//! 2. Launch the configured executable (Affinity Photo, GIMP, …) with the file as its argument.
//! 3. Poll the file's modification time; when the editor saves, report the path so the UI can
//!    import the result next to the original.

use crate::export::{export_one, ExportFormat, ExportJob, ExportSettings};
use halation_color::OutputSpace;
use halation_core::{DevelopParameters, MaskBuffer};
use std::{
    io,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEditor {
    pub name: String,
    pub executable: PathBuf,
    pub extra_args: Vec<String>,
}

/// Where the hand-off TIFF lives: `<scratch>/halation-external/<stem>-<asset id>.tif`.
pub fn handoff_path(scratch: &Path, source: &Path, asset_id: u64) -> PathBuf {
    let stem = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".into());
    scratch.join("halation-external").join(format!("{stem}-{asset_id}.tif"))
}

/// Render the 16-bit ProPhoto TIFF for `source` with the current develop settings.
pub fn write_handoff_tiff(
    source: &Path,
    dest: &Path,
    params: &DevelopParameters,
    mask: Option<Arc<MaskBuffer>>,
) -> Result<(), String> {
    let job = ExportJob { src: source.to_path_buf(), dest: dest.to_path_buf(), params: params.clone(), mask };
    let settings = ExportSettings {
        format: ExportFormat::Tiff { sixteen_bit: true },
        space: OutputSpace::ProPhoto,
        max_long_edge: None,
        embed_profile: true,
        watermark: None,
    };
    export_one(&job, &settings)
}

pub fn launch(editor: &ExternalEditor, file: &Path) -> io::Result<Child> {
    Command::new(&editor.executable).args(&editor.extra_args).arg(file).spawn()
}

fn modified(path: &Path) -> Option<SystemTime> {
    path.metadata().ok()?.modified().ok()
}

/// Poll `path` every `poll` and call `on_saved` whenever its modification time advances.
/// The watcher stops when `stop` is set. A short settle delay avoids reading half-written files.
pub fn watch_for_saves(
    path: PathBuf,
    poll: Duration,
    stop: Arc<AtomicBool>,
    on_saved: impl Fn(PathBuf) + Send + 'static,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut last = modified(&path);
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(poll);
            let now = modified(&path);
            if now.is_some() && now != last {
                thread::sleep(Duration::from_millis(400));
                if modified(&path) == now {
                    last = now;
                    on_saved(path.clone());
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn handoff_path_is_stable_and_scoped() {
        let p = handoff_path(Path::new("/tmp"), Path::new("/photos/IMG_0042.ARW"), 7);
        assert!(p.ends_with("halation-external/IMG_0042-7.tif"));
    }

    #[test]
    fn watcher_reports_saves_and_stops() {
        let dir = std::env::temp_dir().join(format!("halation-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.tif");
        std::fs::write(&file, b"a").unwrap();
        let seen = Arc::new(Mutex::new(0usize));
        let stop = Arc::new(AtomicBool::new(false));
        let counter = seen.clone();
        let handle = watch_for_saves(file.clone(), Duration::from_millis(30), stop.clone(), move |_| *counter.lock().unwrap() += 1);
        thread::sleep(Duration::from_millis(120));
        std::fs::write(&file, b"changed").unwrap();
        let t = SystemTime::now() + Duration::from_secs(5);
        let _ = std::fs::File::options().write(true).open(&file).and_then(|f| f.set_modified(t));
        thread::sleep(Duration::from_millis(900));
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        assert!(*seen.lock().unwrap() >= 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
