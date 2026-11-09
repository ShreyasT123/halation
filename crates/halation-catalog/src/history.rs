//! Persistent branching history (Sprint 7) and volume maintenance (Sprint 1) for [`Catalog`].
//!
//! `develop_history` is a self-referencing table (`parent_history_id`); `develop_states.active_history_id`
//! points at the head. Committing from a historical node simply inserts a child of that node, so no
//! branch is ever truncated.

use super::{volume, Catalog};
use halation_core::DevelopParameters;
use rusqlite::{params, OptionalExtension};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct StoredNode {
    pub db_id: i64,
    pub parent_db_id: Option<i64>,
    pub label: String,
    pub detail: String,
    pub params: DevelopParameters,
}

impl Catalog {
    /// Every history node of `asset_id` (oldest first) and the active head's database id.
    pub fn history_load(&self, asset_id: u64) -> Option<(Vec<StoredNode>, i64)> {
        let conn = self.conn.lock().ok()?;
        let mut stmt = conn
            .prepare("SELECT id,parent_history_id,action_label,detail,params_json FROM develop_history WHERE asset_id=?1 ORDER BY id")
            .ok()?;
        let nodes: Vec<StoredNode> = stmt
            .query_map(params![asset_id as i64], |r| {
                let json: String = r.get(4)?;
                Ok(StoredNode {
                    db_id: r.get(0)?,
                    parent_db_id: r.get(1)?,
                    label: r.get(2)?,
                    detail: r.get(3)?,
                    params: serde_json::from_str(&json).unwrap_or_default(),
                })
            })
            .ok()?
            .filter_map(Result::ok)
            .collect();
        let last = nodes.last()?.db_id;
        let active: Option<i64> = conn
            .query_row("SELECT active_history_id FROM develop_states WHERE asset_id=?1", params![asset_id as i64], |r| r.get(0))
            .optional()
            .ok()?
            .flatten();
        let active = active.filter(|a| nodes.iter().any(|n| n.db_id == *a)).unwrap_or(last);
        Some((nodes, active))
    }

    /// Insert a node under `parent` and make it the active head (one transaction).
    pub fn history_append(
        &self,
        asset_id: u64,
        parent: Option<i64>,
        label: &str,
        detail: &str,
        params_value: &DevelopParameters,
    ) -> Result<i64, String> {
        let json = serde_json::to_string(params_value).map_err(|e| e.to_string())?;
        let mut conn = self.conn.lock().map_err(|_| "catalog lock poisoned".to_owned())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO develop_history(asset_id,parent_history_id,action_label,detail,params_json) VALUES(?1,?2,?3,?4,?5)",
            params![asset_id as i64, parent, label, detail, json],
        )
        .map_err(|e| e.to_string())?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO develop_states(asset_id,active_history_id,params_json) VALUES(?1,?2,?3) \
             ON CONFLICT(asset_id) DO UPDATE SET active_history_id=excluded.active_history_id,params_json=excluded.params_json,updated_at=strftime('%s','now')",
            params![asset_id as i64, id, json],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Move the head to an existing node (undo / redo / history click) and store its parameters.
    pub fn history_set_active(&self, asset_id: u64, db_id: i64, params_value: &DevelopParameters) -> Result<(), String> {
        let json = serde_json::to_string(params_value).map_err(|e| e.to_string())?;
        self.conn
            .lock()
            .map_err(|_| "catalog lock poisoned".to_owned())?
            .execute(
                "UPDATE develop_states SET active_history_id=?1,params_json=?2,updated_at=strftime('%s','now') WHERE asset_id=?3",
                params![db_id, json, asset_id as i64],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn update_color_label(&self, id: u64, label: &str) {
        if ["Red", "Yellow", "Green", "Blue", "Purple", "None"].contains(&label) {
            let _ = self.conn.lock().unwrap().execute("UPDATE assets SET color_label=?1 WHERE id=?2", params![label, id as i64]);
        }
    }

    /// Re-point assets whose file vanished (changed drive letter / mount point) at the same
    /// `relative_path` on the volume with the same uuid. Returns how many were relinked.
    pub fn relink_missing(&self) -> usize {
        let roots = volume::mount_roots();
        let Ok(conn) = self.conn.lock() else { return 0 };
        let Ok(mut stmt) = conn.prepare("SELECT id,file_path,volume_uuid,relative_path FROM assets WHERE volume_uuid<>'' AND relative_path<>''") else {
            return 0;
        };
        let rows: Vec<(i64, String, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default();
        drop(stmt);
        let mut relinked = 0;
        for (id, path, uuid, relative) in rows {
            if PathBuf::from(&path).exists() {
                continue;
            }
            if let Some(found) = volume::resolve(&uuid, &relative, &roots) {
                if conn.execute("UPDATE assets SET file_path=?1 WHERE id=?2", params![found.to_string_lossy(), id]).is_ok() {
                    relinked += 1;
                }
            }
        }
        relinked
    }
}
