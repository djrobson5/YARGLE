//! Persistent folder-scan cache, YARG-songcache style.
//!
//! Parsing a CON file for the list view costs three file opens plus a dozen
//! seeks (magic check, header read, DTA block-chain walk). On a 20k-song
//! library on an external drive that's minutes of wall time — almost all of it
//! re-deriving results that haven't changed. This cache keys each entry's
//! parsed `SongSummary` by (path, mtime, size); on rescan, entries whose
//! signature matches are served without opening the file at all, so a warm
//! scan is just directory enumeration + stats.
//!
//! Known non-song files are cached too (`summary = NULL`) so we don't re-open
//! them for a magic check on every scan. The summary is stored as JSON: if
//! `SongSummary` gains fields later, old rows fail to deserialize and are
//! simply treated as misses (self-invalidating, no schema migration needed).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use tauri::AppHandle;

use crate::dta::types::SongSummary;
use crate::local_db;

pub struct CacheRow {
    pub mtime: i64,
    pub size: i64,
    /// `None` = known non-song file (failed the STFS magic check).
    pub summary: Option<SongSummary>,
}

fn open_db(db: &Path) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(db)
        .map_err(|e| format!("SQLite open failed: {}", e))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS scan_cache (
            path TEXT PRIMARY KEY,
            mtime INTEGER NOT NULL,
            size INTEGER NOT NULL,
            summary TEXT
        )",
        [],
    )
    .map_err(|e| format!("SQLite init failed: {}", e))?;
    Ok(conn)
}

/// Load every cached row under `root` into memory (a 20k-row library is only a
/// few MB; filtering in Rust avoids LIKE-escaping issues with `_` in paths).
pub fn load(app: &AppHandle, root: &str) -> HashMap<String, CacheRow> {
    match local_db::db_path(app) {
        Ok(db) => load_at(&db, root),
        Err(_) => HashMap::new(),
    }
}

fn load_at(db: &Path, root: &str) -> HashMap<String, CacheRow> {
    let mut map = HashMap::new();
    let Ok(conn) = open_db(db) else {
        return map;
    };
    let Ok(mut stmt) = conn.prepare("SELECT path, mtime, size, summary FROM scan_cache") else {
        return map;
    };
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    });
    if let Ok(rows) = rows {
        for (path, mtime, size, summary_json) in rows.flatten() {
            if !path.starts_with(root) {
                continue;
            }
            let summary = match summary_json {
                Some(json) => match serde_json::from_str::<SongSummary>(&json) {
                    Ok(s) => Some(s),
                    // Stale schema — drop the row so the entry re-parses.
                    Err(_) => continue,
                },
                None => None,
            };
            map.insert(path, CacheRow { mtime, size, summary });
        }
    }
    map
}

/// Persist newly parsed entries and prune rows under `root` that no longer
/// exist on disk. One transaction — ~20k upserts complete in well under a second.
pub fn store(
    app: &AppHandle,
    root: &str,
    seen: &HashSet<String>,
    cached: &HashMap<String, CacheRow>,
    new_rows: &[(String, i64, i64, Option<SongSummary>)],
) -> Result<(), String> {
    store_at(&local_db::db_path(app)?, root, seen, cached, new_rows)
}

fn store_at(
    db: &Path,
    root: &str,
    seen: &HashSet<String>,
    cached: &HashMap<String, CacheRow>,
    new_rows: &[(String, i64, i64, Option<SongSummary>)],
) -> Result<(), String> {
    let mut conn = open_db(db)?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("SQLite transaction failed: {}", e))?;
    {
        let mut upsert = tx
            .prepare(
                "INSERT OR REPLACE INTO scan_cache (path, mtime, size, summary)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|e| e.to_string())?;
        for (path, mtime, size, summary) in new_rows {
            let json = summary
                .as_ref()
                .and_then(|s| serde_json::to_string(s).ok());
            upsert
                .execute(rusqlite::params![path, mtime, size, json])
                .map_err(|e| e.to_string())?;
        }

        let mut delete = tx
            .prepare("DELETE FROM scan_cache WHERE path = ?1")
            .map_err(|e| e.to_string())?;
        for path in cached.keys() {
            if path.starts_with(root) && !seen.contains(path) {
                let _ = delete.execute([path]);
            }
        }
    }
    tx.commit().map_err(|e| format!("SQLite commit failed: {}", e))
}
