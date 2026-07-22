//! Shared handle to YARGLE's own SQLite database (`yargle.db` in the app data
//! dir). Used by the RhythmVerse download tracker and the folder-scan cache.

use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub(crate) fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("No app data dir: {}", e))?;
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create data dir: {}", e))?;
    Ok(dir.join("yargle.db"))
}
