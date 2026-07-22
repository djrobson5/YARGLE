//! RhythmVerse (rhythmverse.co) integration — browse/search customs from within YARGLE.
//!
//! The site is a thin front-end over a JSON API. The rich browse endpoint is
//! `POST /api/{game}/songfiles/search/live` with `data_type=full`, returning
//! `{ status, data: { songs: [{ data, file }], records, pagination } }`.
//! `song.data` holds song metadata, `song.file` holds the file record whose
//! `file_id` (32-hex) maps to the download URL `/download/{file_id}`.
//!
//! The API is a scraper of ~190k heterogeneous records: individual fields are
//! frequently null/missing and inconsistently typed (numbers arrive as strings
//! and vice-versa), so we deserialize `data`/`file` as loose `serde_json::Value`
//! and coerce each field rather than binding rigid typed structs.

use reqwest::Client;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const BASE: &str = "https://rhythmverse.co";
const USER_AGENT: &str = concat!("YARGLE/", env!("CARGO_PKG_VERSION"), " (song browser)");

/// One row in the browse results — a specific chart file for a game.
#[derive(Debug, Clone, Serialize)]
pub struct RvSongFile {
    pub file_id: String,
    pub song_id: Option<i64>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub subgenre: String,
    pub year: Option<i64>,
    pub decade: String,
    pub song_length_sec: Option<i64>,
    pub album_art_url: String,
    pub charter: String,
    pub gameformat: String,
    pub gamesource: String,
    pub size_bytes: Option<i64>,
    pub downloads: Option<i64>,
    pub uploader: String,
    pub uploaded: String,
    pub file_name: String,
    pub detail_url: String,
    pub download_url: String,
    // Non-empty when the file is hosted off-site (Google Drive, Mediafire, …)
    // rather than on RhythmVerse. These can't be auto-downloaded reliably, so
    // the UI opens them in the browser instead.
    pub external_url: String,
    // Per-instrument difficulty tiers. >=1 means charted at that tier;
    // 0 / -1 / null means the instrument isn't present.
    pub diff_guitar: Option<i64>,
    pub diff_bass: Option<i64>,
    pub diff_drums: Option<i64>,
    pub diff_vocals: Option<i64>,
    pub diff_keys: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RvBrowseResult {
    pub songs: Vec<RvSongFile>,
    pub total_available: i64,
    pub total_filtered: i64,
    pub returned: i64,
    pub page: u32,
}

// --- Wire types (only the envelope is rigid; per-song payloads stay loose) ---

#[derive(Deserialize)]
struct RvResponse {
    status: String,
    #[serde(default)]
    data: Option<RvData>,
    #[serde(default)]
    error: Option<RvError>,
}

#[derive(Deserialize)]
struct RvError {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize, Default)]
struct RvData {
    // The API returns `songs: false` (not `[]`) when nothing matches, so parse
    // leniently: anything that isn't an array becomes an empty list.
    #[serde(default, deserialize_with = "de_songs_lenient")]
    songs: Vec<RvSongRaw>,
    #[serde(default)]
    records: RvCounts,
}

fn de_songs_lenient<'de, D>(deserializer: D) -> Result<Vec<RvSongRaw>, D::Error>
where
    D: Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::Array(items) => Ok(items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect()),
        _ => Ok(Vec::new()),
    }
}

#[derive(Deserialize, Default)]
struct RvCounts {
    #[serde(default)]
    total_available: i64,
    #[serde(default)]
    total_filtered: i64,
    #[serde(default)]
    returned: i64,
}

#[derive(Deserialize)]
struct RvSongRaw {
    #[serde(default)]
    data: Value,
    #[serde(default)]
    file: Value,
}

// --- Coercion helpers (the API is inconsistent about string vs number) ---

/// Extract a field as a String, coercing numbers/bools to their text form.
fn cs(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Extract a field as an i64, parsing numeric strings and truncating floats.
fn ci(v: &Value, key: &str) -> Option<i64> {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                t.parse::<i64>()
                    .ok()
                    .or_else(|| t.parse::<f64>().ok().map(|f| f as i64))
            }
        }
        _ => None,
    }
}

/// Turn a possibly-relative URL/path into an absolute rhythmverse.co URL.
fn absolutize(u: &str) -> String {
    let u = u.trim();
    if u.is_empty() {
        String::new()
    } else if u.starts_with("http://") || u.starts_with("https://") {
        u.to_string()
    } else if let Some(rest) = u.strip_prefix('/') {
        format!("{}/{}", BASE, rest)
    } else {
        format!("{}/{}", BASE, u)
    }
}

fn build_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

fn map_song(raw: &RvSongRaw) -> Option<RvSongFile> {
    let d = &raw.data;
    let f = &raw.file;

    // A row is only actionable if it carries a file_id we can download.
    let file_id = cs(f, "file_id");
    if file_id.is_empty() {
        return None;
    }

    // Charter/author naming varies between records.
    let charter = {
        let c = cs(f, "charter");
        if c.is_empty() {
            cs(f, "author")
        } else {
            c
        }
    };

    Some(RvSongFile {
        song_id: ci(d, "song_id"),
        title: cs(d, "title"),
        artist: cs(d, "artist"),
        album: cs(d, "album"),
        genre: cs(d, "genre"),
        subgenre: cs(d, "subgenre"),
        year: ci(d, "year"),
        decade: cs(d, "decade"),
        song_length_sec: ci(d, "song_length"),
        album_art_url: absolutize(&cs(d, "album_art")),
        charter,
        gameformat: cs(f, "gameformat"),
        gamesource: cs(f, "gamesource"),
        size_bytes: ci(f, "size"),
        downloads: ci(f, "downloads").or_else(|| ci(d, "downloads")),
        uploader: cs(f, "user"),
        uploaded: {
            // record_updated is usually 0000-00-00; upload_date is the real one.
            let u = cs(f, "upload_date");
            if u.is_empty() || u.starts_with("0000") {
                cs(f, "record_created")
            } else {
                u
            }
        },
        file_name: cs(f, "file_name"),
        detail_url: format!("{}/songfile/{}", BASE, file_id),
        download_url: format!("{}/download/{}", BASE, file_id),
        external_url: cs(f, "external_url").trim().to_string(),
        // Read per-instrument difficulty from the FILE (only the instruments
        // actually in this chart), NOT `data` (the song-level aggregate across
        // all versions/formats, which would falsely add e.g. vocals).
        diff_guitar: ci(f, "diff_guitar"),
        diff_bass: ci(f, "diff_bass"),
        diff_drums: ci(f, "diff_drums"),
        diff_vocals: ci(f, "diff_vocals"),
        diff_keys: ci(f, "diff_keys"),
        file_id,
    })
}

/// Fetch a page of browse/search results for a game (default `yarg`).
///
/// `text` is the free-text query (empty = browse everything). `sort_by`
/// defaults to `update_date` (other values seen: `title`, `artist`,
/// `downloads`); `sort_order` is `DESC`/`ASC`.
#[tauri::command]
pub async fn rv_browse(
    game: Option<String>,
    text: Option<String>,
    page: Option<u32>,
    records: Option<u32>,
    sort_by: Option<String>,
    sort_order: Option<String>,
) -> Result<RvBrowseResult, String> {
    let game = game.filter(|g| !g.is_empty()).unwrap_or_else(|| "yarg".into());
    let text = text.unwrap_or_default();
    let page = page.unwrap_or(1).max(1);
    let records = records.unwrap_or(25).clamp(1, 100);
    let sort_by = sort_by
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "update_date".into());
    let sort_order = sort_order
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "DESC".into());

    // The search endpoint requires a query of >=3 chars; with no (or too
    // short) a query we hit the plain `list` endpoint instead, which honors
    // the same sort and returns the same rich shape — i.e. the default
    // "browse the most recent uploads" view.
    let query = text.trim();
    let is_search = query.chars().count() >= 3;
    let endpoint = if is_search { "search/live" } else { "list" };
    let url = format!("{}/api/{}/songfiles/{}", BASE, game, endpoint);

    // `sort` is a nested-array param; reqwest's .form() percent-encodes the
    // bracket keys to the `sort%5B0%5D%5Bsort_by%5D` shape the server expects.
    let mut params: Vec<(&str, String)> = vec![
        ("data_type", "full".into()),
        ("page", page.to_string()),
        ("records", records.to_string()),
        ("sort[0][sort_by]", sort_by),
        ("sort[0][sort_order]", sort_order),
    ];
    if is_search {
        params.push(("text", query.to_string()));
    }

    let client = build_client()?;
    let resp = client
        .post(&url)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Accept", "application/json, text/javascript, */*; q=0.01")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("RhythmVerse request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("RhythmVerse HTTP {}", resp.status()));
    }

    let parsed: RvResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse RhythmVerse response: {}", e))?;

    if parsed.status != "success" {
        let msg = parsed
            .error
            .map(|e| e.message)
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| "RhythmVerse returned an error".into());
        return Err(msg);
    }

    let data = parsed.data.unwrap_or_default();
    let songs: Vec<RvSongFile> = data.songs.iter().filter_map(map_song).collect();

    Ok(RvBrowseResult {
        songs,
        total_available: data.records.total_available,
        total_filtered: data.records.total_filtered,
        returned: data.records.returned,
        page,
    })
}

// ===== Download + extract + local tracking =====

#[derive(Debug, Clone, Serialize)]
pub struct RvDownloadResult {
    pub file_id: String,
    pub extracted_to: String,
    pub entries: usize,
}

fn emit_progress(app: &AppHandle, file_id: &str, phase: &str, received: u64, total: u64, message: &str) {
    let _ = app.emit(
        "rv-download-progress",
        serde_json::json!({
            "file_id": file_id,
            "phase": phase,
            "received": received,
            "total": total,
            "message": message,
        }),
    );
}

/// First single/double-quoted string appearing before the next `;`.
fn first_quoted(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let quote = bytes[i];
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] != quote {
                    j += 1;
                }
                return if j < bytes.len() {
                    Some(s[start..j].to_string())
                } else {
                    None
                };
            }
            b';' => return None,
            _ => i += 1,
        }
    }
    None
}

/// Pull the `window.location = "<url>"` assignment on the interstitial page
/// that points at the real download (a zip or a loose file under
/// `download_file/…`); skips `.replace()`/`.reload()`/comparisons.
fn extract_download_url(html: &str) -> Option<String> {
    let marker = "window.location";
    let mut from = 0usize;
    while let Some(rel) = html[from..].find(marker) {
        let pos = from + rel;
        from = pos + marker.len();
        let rest = &html[from..];
        let Some(idx) = rest.find(|c| c == '=' || c == ';' || c == '(') else {
            continue;
        };
        if rest.as_bytes()[idx] != b'=' {
            continue;
        }
        if let Some(url) = first_quoted(&rest[idx + 1..]) {
            let url = url.replace("\\/", "/");
            if url.contains("download_file") || url.ends_with(".zip") {
                return Some(url);
            }
        }
    }
    None
}

fn build_download_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(USER_AGENT)
        .cookie_store(true)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Extract a zip's bytes into `dest`, guarding against zip-slip. Returns the
/// top-level folder the song landed in and the number of files written.
fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(PathBuf, usize), String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("Invalid zip: {}", e))?;
    let mut count = 0usize;
    let mut top_level: Option<PathBuf> = None;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Zip read error: {}", e))?;
        // enclosed_name() returns None for unsafe (path-traversal) entries.
        let rel = match file.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        if top_level.is_none() {
            if let Some(std::path::Component::Normal(first)) = rel.components().next() {
                top_level = Some(dest.join(first));
            }
        }
        let outpath = dest.join(&rel);
        if file.is_dir() {
            fs::create_dir_all(&outpath).map_err(|e| format!("mkdir failed: {}", e))?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {}", e))?;
            }
            let mut out = File::create(&outpath).map_err(|e| format!("write failed: {}", e))?;
            io::copy(&mut file, &mut out).map_err(|e| format!("extract failed: {}", e))?;
            count += 1;
        }
    }

    Ok((top_level.unwrap_or_else(|| dest.to_path_buf()), count))
}

fn open_db(app: &AppHandle) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(crate::local_db::db_path(app)?)
        .map_err(|e| format!("SQLite open failed: {}", e))?;
    // rv_downloads = files fetched + extracted into the library (exact "have").
    conn.execute(
        "CREATE TABLE IF NOT EXISTS rv_downloads (
            file_id TEXT PRIMARY KEY,
            song_id INTEGER,
            artist TEXT,
            title TEXT,
            file_name TEXT,
            dest_path TEXT,
            downloaded_at TEXT,
            rv_upload_date TEXT
        )",
        [],
    )
    .map_err(|e| format!("SQLite init failed: {}", e))?;
    // rv_upload_date = RhythmVerse's OWN upload timestamp for the version we
    // hold, captured at download time. Update detection compares it against the
    // site's *current* upload_date (same clock → no skew), instead of the old
    // approach of comparing the site's clock against our local "downloaded_at"
    // wall-clock, which false-positived on freshly-uploaded files. Added via
    // ALTER for DBs created before this column existed (ignore "duplicate").
    let _ = conn.execute("ALTER TABLE rv_downloads ADD COLUMN rv_upload_date TEXT", []);
    // rv_opened = off-site links the user opened in the browser (NOT "have";
    // we can't see whether the manual download succeeded — just a visited flag).
    conn.execute(
        "CREATE TABLE IF NOT EXISTS rv_opened (
            file_id TEXT PRIMARY KEY,
            artist TEXT,
            title TEXT,
            external_url TEXT,
            opened_at TEXT
        )",
        [],
    )
    .map_err(|e| format!("SQLite init failed: {}", e))?;
    Ok(conn)
}

#[derive(Debug, Clone, Serialize)]
pub struct RvDownloadRecord {
    pub file_id: String,
    pub downloaded_at: String,
    // RhythmVerse's upload_date for the version we hold. Empty for records made
    // before this was tracked (or editor links, which carry no RV data) — the
    // UI treats an empty baseline as "don't flag updates" to avoid false
    // positives, and backfills it the next time the song appears in a browse.
    pub rv_upload_date: String,
}

/// RhythmVerse files held locally, each with the site's upload_date for the
/// version we have. The browse UI compares that baseline against the site's
/// *current* upload_date to flag charts revised since we grabbed them.
#[tauri::command]
pub fn rv_download_records(app: AppHandle) -> Result<Vec<RvDownloadRecord>, String> {
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare("SELECT file_id, downloaded_at, rv_upload_date FROM rv_downloads")
        .map_err(|e| e.to_string())?;
    let records = stmt
        .query_map([], |row| {
            Ok(RvDownloadRecord {
                file_id: row.get(0)?,
                downloaded_at: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                rv_upload_date: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(records)
}

/// Where a previous download of this file landed, if any.
fn previous_dest(app: &AppHandle, file_id: &str) -> Option<String> {
    let conn = open_db(app).ok()?;
    conn.query_row(
        "SELECT dest_path FROM rv_downloads WHERE file_id = ?1",
        [file_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

/// The set of file_ids whose off-site link the user has opened in the browser.
#[tauri::command]
pub fn rv_opened_ids(app: AppHandle) -> Result<Vec<String>, String> {
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare("SELECT file_id FROM rv_opened")
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// Record that the user opened an off-site download link (for the "Opened" flag).
#[tauri::command]
pub fn rv_mark_opened(
    app: AppHandle,
    file_id: String,
    artist: Option<String>,
    title: Option<String>,
    external_url: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&app)?;
    conn.execute(
        "INSERT OR REPLACE INTO rv_opened (file_id, artist, title, external_url, opened_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            file_id,
            artist.unwrap_or_default(),
            title.unwrap_or_default(),
            external_url.unwrap_or_default(),
            chrono::Utc::now().to_rfc3339()
        ],
    )
    .map_err(|e| format!("Failed to record opened: {}", e))?;
    Ok(())
}

/// Mark a RhythmVerse file as present in the library WITHOUT YARGLE having
/// fetched it — e.g. the user grabbed an off-site (Google Drive) download by
/// hand and dropped it in. Keyed on the exact `file_id`, so the "In library"
/// badge is precise no matter how the chart's own metadata is spelled (the
/// artist/title heuristic can't be trusted for version tags, `&`/`and`,
/// accents, etc.). `dest_path` is left empty — we don't know which folder it
/// landed in — and `downloaded_at` = now, so the update check treats the user
/// as holding the current version. Because `rv_download` never succeeds for
/// external files, any `rv_downloads` row for one is necessarily a manual mark,
/// which is what lets the UI offer an undo (see `rv_unmark_downloaded`).
#[tauri::command]
pub fn rv_mark_downloaded(
    app: AppHandle,
    file_id: String,
    song_id: Option<i64>,
    artist: Option<String>,
    title: Option<String>,
    file_name: Option<String>,
    uploaded: Option<String>,
) -> Result<(), String> {
    record_download(
        &app,
        &file_id,
        song_id,
        &artist.unwrap_or_default(),
        &title.unwrap_or_default(),
        &file_name.unwrap_or_default(),
        "", // destination unknown for a manual/external placement
        &uploaded.unwrap_or_default(), // RV upload_date = version baseline
    )
}

/// Undo a manual "Got it" mark. Only removes rows with no recorded destination
/// path, so a real YARGLE-performed download (which always records where it
/// extracted) can never be wiped by an accidental undo click.
#[tauri::command]
pub fn rv_unmark_downloaded(app: AppHandle, file_id: String) -> Result<(), String> {
    let conn = open_db(&app)?;
    conn.execute(
        "DELETE FROM rv_downloads WHERE file_id = ?1 AND (dest_path IS NULL OR dest_path = '')",
        [file_id],
    )
    .map_err(|e| format!("Failed to unmark: {}", e))?;
    Ok(())
}

/// Link an on-disk song (`dest_path`) to a RhythmVerse `file_id` from the
/// editor. Unlike "Got it", this captures the real folder path, so the browser
/// can both badge it "In library" (exact) and flag updates, and — for
/// self-hosted files — replace-in-place on re-download. Enforces one link per
/// path: any prior link for this folder is dropped first so re-linking replaces
/// rather than leaving a stale "in library" row behind.
#[tauri::command]
pub fn rv_link_song(
    app: AppHandle,
    file_id: String,
    dest_path: String,
    song_id: Option<i64>,
    artist: Option<String>,
    title: Option<String>,
    file_name: Option<String>,
    uploaded: Option<String>,
) -> Result<(), String> {
    let file_id = file_id.trim().to_string();
    if file_id.is_empty() {
        return Err("Empty RhythmVerse file id".into());
    }
    let conn = open_db(&app)?;
    if !dest_path.is_empty() {
        conn.execute("DELETE FROM rv_downloads WHERE dest_path = ?1", [&dest_path])
            .map_err(|e| format!("Failed to clear previous link: {}", e))?;
    }
    // The editor has no RV data, so `uploaded` is normally empty here — the
    // browse UI backfills the version baseline the next time the song appears.
    conn.execute(
        "INSERT OR REPLACE INTO rv_downloads
            (file_id, song_id, artist, title, file_name, dest_path, downloaded_at, rv_upload_date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            file_id,
            song_id,
            artist.unwrap_or_default(),
            title.unwrap_or_default(),
            file_name.unwrap_or_default(),
            dest_path,
            chrono::Utc::now().to_rfc3339(),
            uploaded.unwrap_or_default()
        ],
    )
    .map_err(|e| format!("Failed to link: {}", e))?;
    Ok(())
}

/// Remove the RhythmVerse link for an on-disk song (by its folder/file path).
#[tauri::command]
pub fn rv_unlink_song(app: AppHandle, dest_path: String) -> Result<(), String> {
    let conn = open_db(&app)?;
    conn.execute("DELETE FROM rv_downloads WHERE dest_path = ?1", [dest_path])
        .map_err(|e| format!("Failed to unlink: {}", e))?;
    Ok(())
}

/// The RhythmVerse file_id an on-disk song is linked to, if any — so the editor
/// can show the current link for the selected song.
#[tauri::command]
pub fn rv_linked_file_id(app: AppHandle, path: String) -> Result<Option<String>, String> {
    let conn = open_db(&app)?;
    match conn.query_row(
        "SELECT file_id FROM rv_downloads WHERE dest_path = ?1 LIMIT 1",
        [path],
        |row| row.get::<_, String>(0),
    ) {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Refresh a download record's timestamp to now, keeping its path. Used when
/// the user re-opens an external file's link to grab an update: it clears the
/// "Update" flag without wiping the linked folder path (unlike a fresh mark).
#[tauri::command]
pub fn rv_touch_downloaded(app: AppHandle, file_id: String) -> Result<(), String> {
    let conn = open_db(&app)?;
    conn.execute(
        "UPDATE rv_downloads SET downloaded_at = ?2 WHERE file_id = ?1",
        rusqlite::params![file_id, chrono::Utc::now().to_rfc3339()],
    )
    .map_err(|e| format!("Failed to update timestamp: {}", e))?;
    Ok(())
}

/// Fetch the interstitial, resolve the real download URL, wait out the
/// countdown, and stream the file bytes — calling `on_progress(received,
/// total)` as it goes. Returns the bytes plus a suggested filename (from the
/// Content-Disposition header, else the URL's last path segment). The network
/// core of `rv_download`, factored out so it needs no `AppHandle`. Note the
/// payload may be a zip OR a loose file (raw CON/STFS, `.sng`, etc.).
async fn fetch_download(
    client: &Client,
    file_id: &str,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<(Vec<u8>, String), String> {
    // 1) Interstitial page (also establishes the download cookie).
    let interstitial_url = format!("{}/download/{}", BASE, file_id);
    let html = client
        .get(&interstitial_url)
        .send()
        .await
        .map_err(|e| format!("Download page request failed: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Failed to read download page: {}", e))?;

    let file_url = extract_download_url(&html)
        .map(|u| absolutize(&u))
        .ok_or("Could not find the download link on the RhythmVerse page")?;

    // 2) Honor the interstitial's short countdown — be a polite client.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // 3) Stream the file, reporting progress against content-length.
    let mut resp = client
        .get(&file_url)
        .header("Referer", &interstitial_url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Download failed: HTTP {}", resp.status()));
    }

    // Resolve the filename before consuming the body.
    let cd_name = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(filename_from_disposition);
    let url_name = resp
        .url()
        .path_segments()
        .and_then(|segs| segs.filter(|s| !s.is_empty()).last().map(percent_decode));
    let filename = sanitize_filename(cd_name.or(url_name), file_id);

    let total = resp.content_length().unwrap_or(0);
    let mut bytes: Vec<u8> = Vec::with_capacity(total as usize);
    let mut last_emit: u64 = 0;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("Download interrupted: {}", e))?
    {
        bytes.extend_from_slice(&chunk);
        let received = bytes.len() as u64;
        if received - last_emit >= 256 * 1024 || (total > 0 && received >= total) {
            last_emit = received;
            on_progress(received, total);
        }
    }

    Ok((bytes, filename))
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Minimal percent-decoder for filenames pulled out of URLs/headers.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Parse a filename out of a Content-Disposition header value.
fn filename_from_disposition(cd: &str) -> Option<String> {
    let lower = cd.to_ascii_lowercase();
    if let Some(pos) = lower.find("filename*=") {
        let val = cd[pos + "filename*=".len()..]
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"');
        // RFC 5987: charset'lang'value — take the part after the last ''
        let encoded = val.rsplit("''").next().unwrap_or(val);
        let decoded = percent_decode(encoded);
        if !decoded.is_empty() {
            return Some(decoded);
        }
    }
    if let Some(pos) = lower.find("filename=") {
        let val = cd[pos + "filename=".len()..]
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"');
        if !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

/// Reduce a suggested name to a safe basename, falling back to the file_id.
fn sanitize_filename(name: Option<String>, file_id: &str) -> String {
    let raw = name.unwrap_or_default();
    // Keep only the final path component and drop anything unsafe.
    let base = raw
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or("")
        .trim();
    let cleaned: String = base
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\0'))
        .collect();
    let cleaned = cleaned.trim_matches('.').trim();
    if cleaned.is_empty() {
        format!("{}.bin", file_id)
    } else {
        cleaned.to_string()
    }
}

/// Heuristic: does this payload look like an HTML page (e.g. a sign-in wall)
/// rather than a real file? Zips start with `PK`, CON/STFS with `CON `/`LIVE`/
/// `PIRS`, so a leading `<` plus an html-ish tag is a strong signal.
fn looks_like_html(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(512)];
    if head.iter().copied().find(|b| !b.is_ascii_whitespace()) != Some(b'<') {
        return false;
    }
    let s = String::from_utf8_lossy(head).to_ascii_lowercase();
    s.contains("<!doctype") || s.contains("<html") || s.contains("<head") || s.contains("<body")
}

/// Download one file from RhythmVerse and extract it into `dest_folder`.
///
/// Flow: GET the `/download/{file_id}` interstitial (which also sets the
/// download cookie) → parse the real zip URL out of its redirect script →
/// wait out the countdown politely → stream the zip → extract → record the
/// file_id locally so the library badge stays exact.
#[tauri::command]
pub async fn rv_download(
    app: AppHandle,
    file_id: String,
    dest_folder: String,
    song_id: Option<i64>,
    artist: Option<String>,
    title: Option<String>,
    file_name: Option<String>,
    uploaded: Option<String>,
) -> Result<RvDownloadResult, String> {
    let artist = artist.unwrap_or_default();
    let title = title.unwrap_or_default();
    let file_name = file_name.unwrap_or_default();
    let uploaded = uploaded.unwrap_or_default();

    let dest = PathBuf::from(&dest_folder);
    if !dest.is_dir() {
        return Err(format!("Destination folder does not exist: {}", dest_folder));
    }

    emit_progress(&app, &file_id, "starting", 0, 0, "Contacting RhythmVerse…");
    let client = build_download_client()?;

    emit_progress(&app, &file_id, "downloading", 0, 0, "Downloading…");
    let (bytes, filename) = fetch_download(&client, &file_id, &|received, total| {
        emit_progress(&app, &file_id, "downloading", received, total, "Downloading…");
    })
    .await?;
    let total = bytes.len() as u64;

    // Decide what we got: a zip to extract, or a loose file to save as-is.
    // RhythmVerse serves both — many YARG/RB customs are raw CON/STFS packages.
    let (extracted_to, entries) = if bytes.starts_with(b"PK") {
        emit_progress(&app, &file_id, "extracting", total, total, "Extracting…");
        extract_zip(&bytes, &dest)?
    } else if looks_like_html(&bytes) {
        return Err(
            "RhythmVerse returned a web page instead of a file — this download may require signing in."
                .into(),
        );
    } else {
        // Raw package (CON/STFS, .sng, etc.). YARGLE detects these by magic
        // bytes, so writing the file into the library folder is enough.
        emit_progress(&app, &file_id, "saving", total, total, "Saving…");
        let out_path = dest.join(&filename);
        fs::write(&out_path, &bytes).map_err(|e| format!("Failed to save file: {}", e))?;
        (out_path, 1)
    };

    // If an earlier download of this same file landed at a different path
    // (e.g. the charter renamed the folder in an update), remove the stale
    // copy so a re-download replaces the song instead of duplicating it.
    if let Some(old) = previous_dest(&app, &file_id) {
        let old_path = PathBuf::from(&old);
        if !old.is_empty() && old_path != extracted_to && old_path.exists() {
            let removed = if old_path.is_dir() {
                fs::remove_dir_all(&old_path)
            } else {
                fs::remove_file(&old_path)
            };
            if let Err(e) = removed {
                eprintln!("failed to remove previous version {}: {}", old, e);
            }
        }
    }

    // 5) Record it so the "in library" badge is exact next time.
    let _ = record_download(
        &app,
        &file_id,
        song_id,
        &artist,
        &title,
        &file_name,
        extracted_to.to_string_lossy().as_ref(),
        &uploaded,
    );

    emit_progress(&app, &file_id, "done", total, total, "Done");

    Ok(RvDownloadResult {
        file_id,
        extracted_to: extracted_to.to_string_lossy().to_string(),
        entries,
    })
}

fn record_download(
    app: &AppHandle,
    file_id: &str,
    song_id: Option<i64>,
    artist: &str,
    title: &str,
    file_name: &str,
    dest: &str,
    rv_upload_date: &str,
) -> Result<(), String> {
    let conn = open_db(app)?;
    conn.execute(
        "INSERT OR REPLACE INTO rv_downloads
            (file_id, song_id, artist, title, file_name, dest_path, downloaded_at, rv_upload_date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            file_id,
            song_id,
            artist,
            title,
            file_name,
            dest,
            chrono::Utc::now().to_rfc3339(),
            rv_upload_date
        ],
    )
    .map_err(|e| format!("Failed to record download: {}", e))?;
    Ok(())
}

/// Backfill the version baseline for a record that has none — used by the
/// browse UI when it encounters a linked/held song (e.g. an editor link, which
/// carries no RV data) whose `rv_upload_date` is empty. Records the site's
/// current upload_date as "the version you have" so future revisions are
/// detectable. Only fills an EMPTY baseline, so it never clobbers a real one.
#[tauri::command]
pub fn rv_set_upload_baseline(
    app: AppHandle,
    file_id: String,
    uploaded: String,
) -> Result<(), String> {
    let conn = open_db(&app)?;
    conn.execute(
        "UPDATE rv_downloads SET rv_upload_date = ?2
         WHERE file_id = ?1 AND (rv_upload_date IS NULL OR rv_upload_date = '')",
        rusqlite::params![file_id, uploaded],
    )
    .map_err(|e| format!("Failed to set baseline: {}", e))?;
    Ok(())
}

/// Open an off-site download link (Google Drive, Mediafire, …) in the user's
/// default browser. These hosts can't be scraped reliably, so the user grabs
/// the file manually. Restricted to http(s) so we never launch odd schemes
/// from user-uploaded song data.
#[tauri::command]
pub fn rv_open_external(url: String) -> Result<(), String> {
    let u = url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err("Refusing to open a non-web link".into());
    }

    #[cfg(target_os = "windows")]
    let spawned = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", u])
        .spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(u).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(u).spawn();

    spawned
        .map(|_| ())
        .map_err(|e| format!("Failed to open link: {}", e))
}
