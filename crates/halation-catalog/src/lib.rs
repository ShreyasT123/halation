//! halation-catalog — SQLite (WAL) photo catalog.
//!
//! Schema v2 (migrated in place from the Sprint-1 prototype schema):
//! * `assets` — identity by `(volume_uuid, relative_path)` next to the absolute `file_path`, BLAKE3
//!   `file_hash`, EXIF columns, rating / flag / colour label.
//! * `develop_states` — current parameters + `active_history_id`.
//! * `develop_history` — branching edit DAG (see `history.rs`).
//!
//! Cargo: `rusqlite` (bundled), `blake3`, `serde_json`, `halation-core`, `halation-raw`, `libc` (unix),
//! `windows` (windows, see `volume.rs`).

use halation_core::DevelopParameters;
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::UNIX_EPOCH,
};

mod history;
pub mod volume;
pub mod xmp;

pub use history::StoredNode;

#[derive(Debug, Clone)]
pub struct CatalogItem {
    pub id: u64,
    pub path: PathBuf,
    pub filename: String,
    pub file_size: u64,
    pub modified_at: Option<i64>,
    pub rating: u8,
    pub flag: String,
    /// BLAKE3 of the file contents (thumbnail-cache key).
    pub hash: [u8; 32],
    pub volume_uuid: String,
    pub relative_path: String,
    pub color_label: String,
    pub params: DevelopParameters,
}

#[derive(Clone)]
pub struct Catalog {
    conn: Arc<Mutex<Connection>>,
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        Self::initialize(conn)
    }

    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(conn: Connection) -> Result<Self, rusqlite::Error> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS assets (
                id INTEGER PRIMARY KEY, file_path TEXT NOT NULL UNIQUE, volume_serial TEXT NOT NULL DEFAULT '',
                file_hash BLOB NOT NULL, file_size INTEGER NOT NULL, modified_at INTEGER,
                capture_time TEXT, camera_make TEXT, camera_model TEXT, lens_model TEXT,
                focal_length REAL, aperture TEXT, shutter_speed TEXT, iso INTEGER,
                rating INTEGER NOT NULL DEFAULT 0 CHECK(rating BETWEEN 0 AND 5),
                flag TEXT NOT NULL DEFAULT 'None' CHECK(flag IN ('Pick','Reject','None')),
                color_label TEXT NOT NULL DEFAULT 'None', created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );
            CREATE INDEX IF NOT EXISTS idx_assets_hash ON assets(file_hash);
            CREATE TABLE IF NOT EXISTS develop_states (
                asset_id INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
                params_json TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );",
        )?;
        Self::migrate(&conn)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
        conn.prepare(&format!("PRAGMA table_info({table})"))
            .and_then(|mut s| {
                let names: Vec<String> = s.query_map([], |r| r.get::<_, String>(1))?.filter_map(Result::ok).collect();
                Ok(names.iter().any(|n| n == column))
            })
            .unwrap_or(false)
    }

    /// Idempotent upgrade to schema v2.
    fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
        if !Self::has_column(conn, "assets", "volume_uuid") {
            conn.execute_batch(
                "ALTER TABLE assets ADD COLUMN volume_uuid TEXT NOT NULL DEFAULT '';
                 ALTER TABLE assets ADD COLUMN relative_path TEXT NOT NULL DEFAULT '';",
            )?;
        }
        if !Self::has_column(conn, "develop_states", "active_history_id") {
            conn.execute_batch("ALTER TABLE develop_states ADD COLUMN active_history_id INTEGER;")?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS develop_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                parent_history_id INTEGER REFERENCES develop_history(id) ON DELETE SET NULL,
                action_label TEXT NOT NULL,
                detail TEXT NOT NULL DEFAULT '',
                params_json TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );
            CREATE INDEX IF NOT EXISTS idx_history_asset ON develop_history(asset_id);
            CREATE INDEX IF NOT EXISTS idx_assets_lookup ON assets(volume_uuid, relative_path);
            CREATE INDEX IF NOT EXISTS idx_assets_rating ON assets(rating);
            CREATE INDEX IF NOT EXISTS idx_assets_flag ON assets(flag);",
        )?;
        Self::backfill_volumes(conn)
    }

    /// Fill `volume_uuid` / `relative_path` for rows written by the prototype schema.
    fn backfill_volumes(conn: &Connection) -> Result<(), rusqlite::Error> {
        let rows: Vec<(i64, String)> = {
            let mut stmt = conn.prepare("SELECT id,file_path FROM assets WHERE volume_uuid=''")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(Result::ok).collect();
            rows
        };
        if rows.is_empty() {
            return Ok(());
        }
        let roots = volume::mount_roots();
        for (id, path) in rows {
            let loc = volume::locate_in(Path::new(&path), &roots);
            conn.execute(
                "UPDATE assets SET volume_uuid=?1,relative_path=?2 WHERE id=?3",
                params![loc.uuid, loc.relative, id],
            )?;
        }
        Ok(())
    }

    pub fn load_items(&self) -> Vec<CatalogItem> {
        let conn = self.conn.lock().unwrap();
        let Ok(mut stmt) = conn.prepare(
            "SELECT a.id,a.file_path,a.file_size,a.modified_at,a.rating,a.flag,d.params_json,a.file_hash,a.volume_uuid,a.relative_path,a.color_label \
             FROM assets a LEFT JOIN develop_states d ON d.asset_id=a.id ORDER BY a.id DESC",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([], |r| {
            let path: String = r.get(1)?;
            let json: Option<String> = r.get(6)?;
            let blob: Vec<u8> = r.get(7)?;
            let mut hash = [0u8; 32];
            if blob.len() == 32 {
                hash.copy_from_slice(&blob);
            }
            Ok(CatalogItem {
                id: r.get::<_, i64>(0)? as u64,
                filename: Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or(&path).to_owned(),
                path: PathBuf::from(path),
                file_size: r.get::<_, i64>(2)? as u64,
                modified_at: r.get(3)?,
                rating: r.get::<_, i64>(4)? as u8,
                flag: r.get(5)?,
                hash,
                volume_uuid: r.get(8)?,
                relative_path: r.get(9)?,
                color_label: r.get(10)?,
                params: json.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default(),
            })
        }) else {
            return Vec::new();
        };
        rows.filter_map(Result::ok).collect()
    }

    pub fn import_asset(&self, path: &Path, metadata: &halation_raw::RawMetadata) -> Result<u64, String> {
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        let file_metadata = std::fs::metadata(&canonical).map_err(|e| e.to_string())?;
        let size = file_metadata.len();
        let modified = file_metadata.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
        let mut file = std::fs::File::open(&canonical).map_err(|e| e.to_string())?;
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
        let hash = hasher.finalize();
        let location = volume::locate(&canonical);

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        if let Some((id, old_path)) = tx
            .query_row("SELECT id,file_path FROM assets WHERE file_hash=?1", params![hash.as_bytes()], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .optional()
            .map_err(|e| e.to_string())?
        {
            // Same content seen again: if the old location vanished (moved / renamed volume), follow it.
            if Path::new(&old_path) != canonical && !Path::new(&old_path).exists() {
                tx.execute(
                    "UPDATE assets SET file_path=?1,volume_uuid=?2,relative_path=?3 WHERE id=?4",
                    params![canonical.to_string_lossy(), location.uuid, location.relative, id],
                )
                .map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())?;
            }
            return Ok(id as u64);
        }
        let path_text = canonical.to_string_lossy();
        tx.execute(
            "INSERT INTO assets(file_path,file_hash,file_size,modified_at,camera_make,lens_model,aperture,shutter_speed,iso,capture_time,volume_uuid,relative_path) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                path_text,
                hash.as_bytes(),
                size as i64,
                modified,
                metadata.camera,
                metadata.lens,
                metadata.aperture,
                metadata.shutter,
                metadata.iso,
                metadata.captured_at,
                location.uuid,
                location.relative
            ],
        )
        .map_err(|e| e.to_string())?;
        let id = tx.last_insert_rowid();
        let defaults = serde_json::to_string(&DevelopParameters::default()).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO develop_states(asset_id,params_json) VALUES(?1,?2)", params![id, defaults]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id as u64)
    }

    pub fn save_parameters(&self, id: u64, params_value: &DevelopParameters) {
        if let Ok(json) = serde_json::to_string(params_value) {
            let _ = self.conn.lock().unwrap().execute(
                "INSERT INTO develop_states(asset_id,params_json) VALUES(?1,?2) \
                 ON CONFLICT(asset_id) DO UPDATE SET params_json=excluded.params_json,updated_at=strftime('%s','now')",
                params![id as i64, json],
            );
        }
    }

    pub fn update_rating(&self, id: u64, rating: u8) {
        let _ = self.conn.lock().unwrap().execute("UPDATE assets SET rating=?1 WHERE id=?2", params![rating.min(5), id as i64]);
    }

    pub fn update_flag(&self, id: u64, flag: &str) {
        if ["Pick", "Reject", "None"].contains(&flag) {
            let _ = self.conn.lock().unwrap().execute("UPDATE assets SET flag=?1 WHERE id=?2", params![flag, id as i64]);
        }
    }

    /// Write the XMP sidecar for `id` from the catalog's current state (rating, flag, parameters).
    pub fn write_sidecar(&self, item_path: &Path, rating: u8, flag: &str, params_value: &DevelopParameters) -> std::io::Result<PathBuf> {
        xmp::write_sidecar(item_path, &xmp::XmpState { params: params_value.clone(), rating, flag: flag.to_owned() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_catalog_has_v2_schema_and_history_roundtrips() {
        let catalog = Catalog::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("halation-cat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.jpg");
        std::fs::write(&file, b"not really a jpeg").unwrap();
        let id = catalog.import_asset(&file, &halation_raw::RawMetadata::default()).unwrap();
        assert_eq!(catalog.import_asset(&file, &halation_raw::RawMetadata::default()).unwrap(), id);

        let mut p = DevelopParameters::default();
        p.exposure = 0.5;
        let root = catalog.history_append(id, None, "Imported", "", &DevelopParameters::default()).unwrap();
        let child = catalog.history_append(id, Some(root), "Exposure +0.50 EV", "0.00 → +0.50 EV", &p).unwrap();
        catalog.history_set_active(id, root, &DevelopParameters::default()).unwrap();
        let (nodes, active) = catalog.history_load(id).unwrap();
        assert_eq!((nodes.len(), active), (2, root));
        assert_eq!(nodes[1].parent_db_id, Some(root));
        assert_eq!(nodes[1].db_id, child);

        let item = catalog.load_items().remove(0);
        assert_eq!(item.hash.len(), 32);
        assert!(!item.volume_uuid.is_empty() && !item.relative_path.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
