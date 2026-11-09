//! Two-tier thumbnail cache (Sprint 6).
//!
//! * Tier 1 — memory: an LRU of decoded RGBA thumbnails (500 items by default).
//! * Tier 2 — disk: `<dir>/<aa>/<blake3-hex>.<size>.thm`, LZ4-compressed RGBA behind a 12-byte header.
//!   Keyed by the file's BLAKE3 hash, so moved / renamed files keep their cache.
//!
//! Misses are decoded in parallel on the Rayon pool, so a virtualized grid can ask for a screenful at
//! once without stalling scrolling.
//!
//! Cargo: `lru = "0.12"`, `lz4_flex = "0.11"`.

use halation_raw::decode_thumbnail;
use lru::LruCache;
use rayon::prelude::*;
use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub type ThumbKey = [u8; 32];

const MAGIC: &[u8; 4] = b"HTH1";

#[derive(Debug)]
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ThumbJob {
    pub id: u64,
    pub hash: ThumbKey,
    pub path: PathBuf,
}

pub struct ThumbnailCache {
    memory: Mutex<LruCache<ThumbKey, Arc<Thumb>>>,
    dir: PathBuf,
    max_dim: u32,
}

pub fn hex(key: &ThumbKey) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn encode_thumb(t: &Thumb) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + t.rgba.len() / 2);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&t.width.to_le_bytes());
    out.extend_from_slice(&t.height.to_le_bytes());
    out.extend_from_slice(&lz4_flex::compress_prepend_size(&t.rgba));
    out
}

pub fn decode_thumb(bytes: &[u8]) -> Option<Thumb> {
    if bytes.get(0..4)? != MAGIC {
        return None;
    }
    let width = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?);
    let height = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?);
    let rgba = lz4_flex::decompress_size_prepended(bytes.get(12..)?).ok()?;
    (rgba.len() == width as usize * height as usize * 4).then_some(Thumb {
        width,
        height,
        rgba,
    })
}

impl ThumbnailCache {
    pub fn new(dir: impl Into<PathBuf>, max_dim: u32, memory_items: usize) -> Self {
        Self {
            memory: Mutex::new(LruCache::new(
                NonZeroUsize::new(memory_items.max(1)).unwrap_or(NonZeroUsize::MIN),
            )),
            dir: dir.into(),
            max_dim,
        }
    }

    fn disk_path(&self, key: &ThumbKey) -> PathBuf {
        let h = hex(key);
        self.dir
            .join(&h[..2])
            .join(format!("{h}.{}.thm", self.max_dim))
    }

    /// Memory tier only.
    pub fn peek(&self, key: &ThumbKey) -> Option<Arc<Thumb>> {
        self.memory.lock().ok()?.get(key).cloned()
    }

    fn remember(&self, key: ThumbKey, thumb: Arc<Thumb>) {
        if let Ok(mut m) = self.memory.lock() {
            m.put(key, thumb);
        }
    }

    /// Memory → disk → decode (and populate both tiers).
    pub fn get(&self, job: &ThumbJob) -> Option<Arc<Thumb>> {
        if let Some(hit) = self.peek(&job.hash) {
            return Some(hit);
        }
        let disk = self.disk_path(&job.hash);
        if let Some(thumb) = fs::read(&disk).ok().and_then(|b| decode_thumb(&b)) {
            let thumb = Arc::new(thumb);
            self.remember(job.hash, thumb.clone());
            return Some(thumb);
        }
        let decoded = decode_thumbnail(&job.path, self.max_dim).ok()?;
        let thumb = Arc::new(Thumb {
            width: decoded.width,
            height: decoded.height,
            rgba: decoded.rgba,
        });
        self.write_disk(&disk, &thumb);
        self.remember(job.hash, thumb.clone());
        Some(thumb)
    }

    fn write_disk(&self, path: &Path, thumb: &Thumb) {
        let Some(parent) = path.parent() else { return };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let tmp = path.with_extension("tmp");
        if fs::write(&tmp, encode_thumb(thumb)).is_ok() {
            let _ = fs::rename(&tmp, path);
        }
    }

    /// Resolve many thumbnails in parallel; result order matches `jobs`.
    pub fn load_many(&self, jobs: &[ThumbJob]) -> Vec<(u64, Option<Arc<Thumb>>)> {
        jobs.par_iter().map(|job| (job.id, self.get(job))).collect()
    }

    pub fn memory_len(&self) -> usize {
        self.memory.lock().map(|m| m.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_format_roundtrips() {
        let t = Thumb {
            width: 3,
            height: 2,
            rgba: (0..24).collect(),
        };
        let back = decode_thumb(&encode_thumb(&t)).unwrap();
        assert_eq!((back.width, back.height, back.rgba), (3, 2, t.rgba));
        assert!(decode_thumb(b"nope").is_none());
    }

    #[test]
    fn memory_tier_evicts_least_recently_used() {
        let cache = ThumbnailCache::new(std::env::temp_dir().join("halation-thumb-test"), 64, 2);
        for i in 0..3u8 {
            cache.remember(
                [i; 32],
                Arc::new(Thumb {
                    width: 1,
                    height: 1,
                    rgba: vec![0; 4],
                }),
            );
        }
        assert!(cache.peek(&[0; 32]).is_none() && cache.peek(&[2; 32]).is_some());
        assert_eq!(cache.memory_len(), 2);
    }
}
