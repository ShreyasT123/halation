use halation_core::DevelopParameters;
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::UNIX_EPOCH,
};

#[derive(Debug, Clone)]
pub struct CatalogItem {
    pub id: u64,
    pub path: PathBuf,
    pub filename: String,
    pub file_size: u64,
    pub modified_at: Option<i64>,
    pub rating: u8,
    pub flag: String,
    pub color_label: String,
    pub params: DevelopParameters,
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub label: String,
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
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
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
                params_json TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                active_history_id INTEGER
            );
            CREATE TABLE IF NOT EXISTS develop_history (
                id INTEGER PRIMARY KEY, asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                parent_history_id INTEGER REFERENCES develop_history(id), action_label TEXT NOT NULL,
                params_json TEXT NOT NULL, created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );
            CREATE INDEX IF NOT EXISTS idx_history_asset ON develop_history(asset_id,id);")?;
        // Existing catalogs predate the history head pointer.
        let _ = conn.execute(
            "ALTER TABLE develop_states ADD COLUMN active_history_id INTEGER",
            [],
        );
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn load_items(&self) -> Vec<CatalogItem> {
        let conn = self.conn.lock().unwrap();
        let Ok(mut stmt) = conn.prepare("SELECT a.id,a.file_path,a.file_size,a.modified_at,a.rating,a.flag,a.color_label,d.params_json FROM assets a LEFT JOIN develop_states d ON d.asset_id=a.id ORDER BY a.id DESC") else { return Vec::new() };
        let Ok(rows) = stmt.query_map([], |r| {
            let path: String = r.get(1)?;
            let json: Option<String> = r.get(7)?;
            Ok(CatalogItem {
                id: r.get::<_, i64>(0)? as u64,
                filename: Path::new(&path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&path)
                    .to_owned(),
                path: PathBuf::from(path),
                file_size: r.get::<_, i64>(2)? as u64,
                modified_at: r.get(3)?,
                rating: r.get::<_, i64>(4)? as u8,
                flag: r.get(5)?,
                color_label: r.get(6)?,
                params: json
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
            })
        }) else {
            return Vec::new();
        };
        rows.filter_map(Result::ok).collect()
    }

    pub fn import_asset(
        &self,
        path: &Path,
        metadata: &halation_raw::RawMetadata,
    ) -> Result<u64, String> {
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        let file_metadata = std::fs::metadata(&canonical).map_err(|e| e.to_string())?;
        let size = file_metadata.len();
        let modified = file_metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        let mut file = std::fs::File::open(&canonical).map_err(|e| e.to_string())?;
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
        let hash = hasher.finalize();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        if let Some(id) = tx
            .query_row(
                "SELECT id FROM assets WHERE file_hash=?1",
                params![hash.as_bytes()],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
        {
            return Ok(id as u64);
        }
        let path_text = canonical.to_string_lossy();
        tx.execute("INSERT INTO assets(file_path,file_hash,file_size,modified_at,camera_make,lens_model,aperture,shutter_speed,iso,capture_time) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![path_text,hash.as_bytes(),size as i64,modified,metadata.camera,metadata.lens,metadata.aperture,metadata.shutter,metadata.iso,metadata.captured_at]).map_err(|e|e.to_string())?;
        let id = tx.last_insert_rowid();
        let defaults =
            serde_json::to_string(&DevelopParameters::default()).map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO develop_states(asset_id,params_json) VALUES(?1,?2)",
            params![id, defaults],
        )
        .map_err(|e| e.to_string())?;
        let baseline = tx
            .query_row(
                "INSERT INTO develop_history(asset_id,parent_history_id,action_label,params_json) VALUES(?1,NULL,'Import baseline',?2) RETURNING id",
                params![id, defaults],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE develop_states SET active_history_id=?1 WHERE asset_id=?2",
            params![baseline, id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id as u64)
    }

    pub fn save_parameters(&self, id: u64, params_value: &DevelopParameters) {
        if let Ok(json) = serde_json::to_string(params_value) {
            let _=self.conn.lock().unwrap().execute("INSERT INTO develop_states(asset_id,params_json) VALUES(?1,?2) ON CONFLICT(asset_id) DO UPDATE SET params_json=excluded.params_json,updated_at=strftime('%s','now')",params![id as i64,json]);
        }
    }

    /// Append a history node and move the active head. Existing descendants are
    /// preserved, so editing after undo creates a new branch.
    pub fn commit_history(
        &self,
        id: u64,
        label: &str,
        value: &DevelopParameters,
    ) -> Result<i64, String> {
        let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let parent: Option<i64> = tx
            .query_row(
                "SELECT active_history_id FROM develop_states WHERE asset_id=?1",
                [id as i64],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .flatten();
        let mut action = label.to_owned();
        if let Some(parent_id) = parent {
            let previous_json: Option<String> = tx
                .query_row(
                    "SELECT params_json FROM develop_history WHERE id=?1 AND asset_id=?2",
                    params![parent_id, id as i64],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if let Some(previous) =
                previous_json.and_then(|json| serde_json::from_str::<DevelopParameters>(&json).ok())
            {
                if previous == *value {
                    return Ok(parent_id);
                }
                if let Some(delta) = history_label(&previous, value) {
                    action = delta;
                }
            }
        }
        tx.execute("INSERT INTO develop_history(asset_id,parent_history_id,action_label,params_json) VALUES(?1,?2,?3,?4)", params![id as i64,parent,action,json]).map_err(|e|e.to_string())?;
        let node = tx.last_insert_rowid();
        tx.execute("INSERT INTO develop_states(asset_id,params_json,active_history_id) VALUES(?1,?2,?3) ON CONFLICT(asset_id) DO UPDATE SET params_json=excluded.params_json,active_history_id=excluded.active_history_id,updated_at=strftime('%s','now')", params![id as i64,json,node]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(node)
    }

    pub fn history(&self, asset_id: u64) -> Vec<HistoryEntry> {
        let conn = self.conn.lock().unwrap();
        conn.prepare("SELECT id,parent_history_id,action_label,params_json FROM develop_history WHERE asset_id=?1 ORDER BY id")
            .and_then(|mut stmt| stmt.query_map([asset_id as i64], |r| {
                let json: String = r.get(3)?;
                Ok(HistoryEntry { id:r.get(0)?, parent_id:r.get(1)?, label:r.get(2)?, params:serde_json::from_str(&json).unwrap_or_default() })
            }).map(|rows| rows.filter_map(Result::ok).collect()))
            .unwrap_or_default()
    }

    pub fn activate_history(&self, asset_id: u64, history_id: i64) -> Option<DevelopParameters> {
        let conn = self.conn.lock().unwrap();
        let params_json: String = conn
            .query_row(
                "SELECT params_json FROM develop_history WHERE id=?1 AND asset_id=?2",
                params![history_id, asset_id as i64],
                |r| r.get(0),
            )
            .ok()?;
        conn.execute("UPDATE develop_states SET active_history_id=?1,params_json=?2,updated_at=strftime('%s','now') WHERE asset_id=?3", params![history_id,params_json,asset_id as i64]).ok()?;
        serde_json::from_str(&params_json).ok()
    }

    pub fn active_history_id(&self, asset_id: u64) -> Option<i64> {
        self.conn
            .lock()
            .ok()?
            .query_row(
                "SELECT active_history_id FROM develop_states WHERE asset_id=?1",
                [asset_id as i64],
                |r| r.get(0),
            )
            .ok()
            .flatten()
    }

    /// Write the common Lightroom develop fields as an Adobe XMP sidecar.
    pub fn write_xmp_sidecar(
        &self,
        asset_id: u64,
        value: &DevelopParameters,
    ) -> Result<(), String> {
        let path: String = self
            .conn
            .lock()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT file_path FROM assets WHERE id=?1",
                [asset_id as i64],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut sidecar = PathBuf::from(path);
        sidecar.set_extension("xmp");
        let xml = format!(
            "<?xpacket begin='﻿' id='W5M0MpCehiHzreSzNTczkc9d'?>\n<x:xmpmeta xmlns:x='adobe:ns:meta/'><rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description xmlns:crs='http://ns.adobe.com/camera-raw-settings/1.0/' crs:Version='15.0' crs:ProcessVersion='11.0' crs:Exposure2012='{:.4}' crs:Contrast2012='{:.4}' crs:Highlights2012='{:.4}' crs:Shadows2012='{:.4}' crs:Whites2012='{:.4}' crs:Blacks2012='{:.4}' crs:Temperature='{:.1}' crs:Tint='{:.4}'/></rdf:RDF></x:xmpmeta>\n<?xpacket end='w'?>\n",
            value.exposure, value.contrast, value.highlights, value.shadows, value.whites, value.blacks, value.temperature, value.tint
        );
        std::fs::write(sidecar, xml).map_err(|e| e.to_string())
    }

    /// Read core Lightroom slider fields from an existing XMP sidecar.
    pub fn read_xmp_sidecar(path: &Path) -> Result<DevelopParameters, String> {
        let mut sidecar = path.to_path_buf();
        sidecar.set_extension("xmp");
        let xml = std::fs::read_to_string(sidecar).map_err(|e| e.to_string())?;
        let mut value = DevelopParameters::default();
        for (key, target) in [
            ("Exposure2012", &mut value.exposure),
            ("Contrast2012", &mut value.contrast),
            ("Highlights2012", &mut value.highlights),
            ("Shadows2012", &mut value.shadows),
            ("Whites2012", &mut value.whites),
            ("Blacks2012", &mut value.blacks),
            ("Temperature", &mut value.temperature),
            ("Tint", &mut value.tint),
        ] {
            if let Some(raw) = xmp_attribute(&xml, key) {
                *target = raw
                    .parse::<f32>()
                    .map_err(|e| format!("invalid XMP {key}: {e}"))?;
            }
        }
        Ok(value)
    }
    pub fn update_rating(&self, id: u64, rating: u8) {
        let _ = self.conn.lock().unwrap().execute(
            "UPDATE assets SET rating=?1 WHERE id=?2",
            params![rating.min(5), id as i64],
        );
    }
    pub fn update_flag(&self, id: u64, flag: &str) {
        if ["Pick", "Reject", "None"].contains(&flag) {
            let _ = self.conn.lock().unwrap().execute(
                "UPDATE assets SET flag=?1 WHERE id=?2",
                params![flag, id as i64],
            );
        }
    }
    pub fn update_color_label(&self, id: u64, label: &str) {
        if ["None", "Red", "Yellow", "Green", "Blue"].contains(&label) {
            let _ = self.conn.lock().unwrap().execute(
                "UPDATE assets SET color_label=?1 WHERE id=?2",
                params![label, id as i64],
            );
        }
    }
}

fn xmp_attribute(xml: &str, key: &str) -> Option<String> {
    let marker = format!("crs:{key}=");
    let start = xml.find(&marker)? + marker.len();
    let quote = xml.as_bytes().get(start).copied()? as char;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let value_start = start + 1;
    let end = xml[value_start..].find(quote)? + value_start;
    Some(xml[value_start..end].to_owned())
}

fn history_label(old: &DevelopParameters, new: &DevelopParameters) -> Option<String> {
    let mut changes = Vec::new();
    for (name, before, after, suffix) in [
        ("Exposure", old.exposure, new.exposure, " EV"),
        ("Contrast", old.contrast, new.contrast, ""),
        ("Highlights", old.highlights, new.highlights, ""),
        ("Shadows", old.shadows, new.shadows, ""),
        ("Whites", old.whites, new.whites, ""),
        ("Blacks", old.blacks, new.blacks, ""),
        ("Temperature", old.temperature, new.temperature, " K"),
        ("Tint", old.tint, new.tint, ""),
    ] {
        if (before - after).abs() > 0.0001 {
            changes.push(format!("{name} {after:+.2}{suffix}"));
        }
    }
    (!changes.is_empty()).then(|| changes.join(" | "))
}
