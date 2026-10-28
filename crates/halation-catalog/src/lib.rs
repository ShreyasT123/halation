use halation_core::DevelopParameters;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct CatalogItem {
    pub id: u64,
    pub filename: String,
    pub rating: u8,
    pub flag: String, // "Pick", "Reject", "None"
    pub params: DevelopParameters,
}

#[derive(Clone)]
pub struct Catalog {
    conn: Arc<Mutex<Connection>>,
}

impl Catalog {
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        conn.execute(
            "CREATE TABLE photos (
                id INTEGER PRIMARY KEY,
                filename TEXT NOT NULL,
                rating INTEGER NOT NULL,
                flag TEXT NOT NULL,
                params_json TEXT NOT NULL
            )",
            [],
        )?;

        // Prepopulate with demo assets
        let default_json = serde_json::to_string(&DevelopParameters::default()).unwrap();
        conn.execute("INSERT INTO photos VALUES (1, 'DSC_0042.RAW', 5, 'Pick', ?1)", params![default_json])?;
        conn.execute("INSERT INTO photos VALUES (2, 'DSC_0043.RAW', 4, 'None', ?1)", params![default_json])?;
        conn.execute("INSERT INTO photos VALUES (3, 'DSC_0044.RAW', 0, 'Reject', ?1)", params![default_json])?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn load_items(&self) -> Vec<CatalogItem> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, filename, rating, flag, params_json FROM photos ORDER BY id").unwrap();
        let rows = stmt.query_map([], |row| {
            let json_str: String = row.get(4)?;
            let params: DevelopParameters = serde_json::from_str(&json_str).unwrap_or_default();
            Ok(CatalogItem {
                id: row.get(0)?,
                filename: row.get(1)?,
                rating: row.get(2)?,
                flag: row.get(3)?,
                params,
            })
        }).unwrap();

        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn save_parameters(&self, id: u64, params: &DevelopParameters) {
        let conn = self.conn.lock().unwrap();
        let json_str = serde_json::to_string(params).unwrap();
        let _ = conn.execute("UPDATE photos SET params_json = ?1 WHERE id = ?2", params![json_str, id]);
    }

    pub fn update_rating(&self, id: u64, rating: u8) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("UPDATE photos SET rating = ?1 WHERE id = ?2", params![rating, id]);
    }

    pub fn update_flag(&self, id: u64, flag: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("UPDATE photos SET flag = ?1 WHERE id = ?2", params![flag, id]);
    }
}
