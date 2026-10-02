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
use crate::download::{self, archive, hosts, TempFile};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub(crate) const BASE: &str = "https://rhythmverse.co";
pub(crate) const USER_AGENT: &str = concat!("YARGLE/", env!("CARGO_PKG_VERSION"), " (song browser)");

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
    // rather than on RhythmVerse.
    pub external_url: String,
    // True when `rv_download` can fetch that off-site link itself (Drive,
    // Mediafire, Dropbox, shorteners, direct archive links). False means the
    // UI only offers "Open ↗" (MEGA, Ko-fi, other web pages).
    pub external_auto: bool,
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
pub(crate) struct RvData {
    // The API returns `songs: false` (not `[]`) when nothing matches, so parse
    // leniently: anything that isn't an array becomes an empty list.
    #[serde(default, deserialize_with = "de_songs_lenient")]
    pub(crate) songs: Vec<RvSongRaw>,
    #[serde(default)]
    pub(crate) records: RvCounts,
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
pub(crate) struct RvCounts {
    #[serde(default)]
    pub(crate) total_available: i64,
    #[serde(default)]
    pub(crate) total_filtered: i64,
    #[serde(default)]
    pub(crate) returned: i64,
}

#[derive(Deserialize)]
pub(crate) struct RvSongRaw {
    #[serde(default)]
    pub(crate) data: Value,
    #[serde(default)]
    pub(crate) file: Value,
}

// --- Coercion helpers (the API is inconsistent about string vs number) ---

/// Extract a field as a String, coercing numbers/bools to their text form.
pub(crate) fn cs(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Extract a field as an i64, parsing numeric strings and truncating floats.
pub(crate) fn ci(v: &Value, key: &str) -> Option<i64> {
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
pub(crate) fn absolutize(u: &str) -> String {
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

pub(crate) fn map_song(raw: &RvSongRaw) -> Option<RvSongFile> {
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

    let external_url = cs(f, "external_url").trim().to_string();

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
        external_auto: !external_url.is_empty() && hosts::classify(&external_url).is_some(),
        external_url,
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

/// POST one `songfiles` request and unwrap the `{status, data}` envelope.
pub(crate) async fn post_songfiles(
    client: &Client,
    url: &str,
    params: &[(&str, String)],
) -> Result<RvData, String> {
    let resp = client
        .post(url)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Accept", "application/json, text/javascript, */*; q=0.01")
        .form(params)
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

    Ok(parsed.data.unwrap_or_default())
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
    let data = post_songfiles(&client, &url, &params).await?;
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

/// Progress sink for one download: `(phase, received, total, message)`.
/// `rv_download` points it at `emit_progress`; keeping it a closure lets the
/// install steps run without an `AppHandle`.
type Report<'a> = &'a (dyn Fn(&str, u64, u64, &str) + Send + Sync);

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

pub(crate) fn open_db(app: &AppHandle) -> Result<rusqlite::Connection, String> {
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
    // A "Got it" mark with no on-disk path (the user placed the file by hand).
    // Only these can be undone from the browser.
    pub manual: bool,
}

/// RhythmVerse files held locally, each with the site's upload_date for the
/// version we have. The browse UI compares that baseline against the site's
/// *current* upload_date to flag charts revised since we grabbed them.
#[tauri::command]
pub fn rv_download_records(app: AppHandle) -> Result<Vec<RvDownloadRecord>, String> {
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare("SELECT file_id, downloaded_at, rv_upload_date, dest_path FROM rv_downloads")
        .map_err(|e| e.to_string())?;
    let records = stmt
        .query_map([], |row| {
            Ok(RvDownloadRecord {
                file_id: row.get(0)?,
                downloaded_at: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                rv_upload_date: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                manual: row.get::<_, Option<String>>(3)?.unwrap_or_default().is_empty(),
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

/// Resolve a RhythmVerse file to its direct URL: GET the `/download/{id}`
/// interstitial (which also sets the download cookie), pull the real file URL
/// out of its redirect script, and wait out its countdown politely. Returns
/// the file URL plus the interstitial URL (sent as the Referer).
async fn rv_file_url(client: &Client, file_id: &str) -> Result<(String, String), String> {
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

    // Honor the interstitial's short countdown — be a polite client.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    Ok((file_url, interstitial_url))
}

/// Fetch an off-site file into `out`. Google Drive sometimes answers with its
/// "can't scan for viruses" page instead of the file; follow that page's
/// confirm form once, and report quota/private files clearly.
async fn fetch_external_file(
    client: &Client,
    url: &str,
    out: &Path,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<download::Fetched, String> {
    let headers = [("User-Agent", download::BROWSER_UA)];
    let fetched = download::fetch_to_file(client, url, &headers, out, on_progress).await?;
    if !hosts::is_drive_url(url) || !fetched.content_type.starts_with("text/html") {
        return Ok(fetched);
    }
    let html = fs::read_to_string(out).unwrap_or_default();
    let confirm = hosts::drive_confirm_url(&html).ok_or(hosts::DRIVE_UNAVAILABLE)?;
    let fetched = download::fetch_to_file(client, &confirm, &headers, out, on_progress).await?;
    if fetched.content_type.starts_with("text/html") {
        return Err(hosts::DRIVE_UNAVAILABLE.into());
    }
    Ok(fetched)
}

/// Put a downloaded payload into the library: extract it if it's an archive,
/// otherwise save it under `filename` (raw CON/STFS packages, `.sng`, …;
/// YARGLE detects those by magic bytes). Returns the song's path and the
/// number of files written.
fn install_payload(
    report: Report,
    temp: &Path,
    filename: &str,
    dest: &Path,
    size: u64,
    html_error: &str,
) -> Result<(PathBuf, usize), String> {
    let head = download::read_head(temp, 512);
    if download::looks_like_html(&head) {
        return Err(html_error.into());
    }
    if let Some(kind) = archive::detect(&head) {
        report("extracting", size, size, "Extracting…");
        return archive::extract(kind, temp, dest, filename);
    }
    report("saving", size, size, "Saving…");
    let out_path = dest.join(filename);
    download::place_file(temp, &out_path)?;
    Ok((out_path, 1))
}

/// Most files a shared Drive folder may hold before we refuse it — a song is
/// a dozen files; hundreds means someone shared a whole library.
const DRIVE_FOLDER_MAX_FILES: usize = 100;

/// Download a public Google Drive folder into `dest/<folder name>/`, keeping
/// its subfolders. A folder holding a single subfolder is unwrapped first, and
/// one holding a single file (often a zip or CON) is handled like a file link.
async fn download_drive_folder(
    report: Report<'_>,
    client: &Client,
    file_id: &str,
    folder_id: &str,
    dest: &Path,
) -> Result<(PathBuf, usize), String> {
    let (mut title, mut items) = hosts::list_drive_folder(client, folder_id).await?;
    for _ in 0..3 {
        if items.len() == 1 && items[0].is_folder {
            let only = items.remove(0);
            title = only.name;
            items = hosts::list_drive_folder(client, &only.id).await?.1;
        } else {
            break;
        }
    }
    if items.is_empty() {
        return Err(
            "This Google Drive folder is empty (or private). Use Open ↗ to check it in your browser.".into(),
        );
    }
    if items.len() == 1 {
        let item = &items[0];
        let temp = TempFile::new(file_id)?;
        let fetched = fetch_external_file(client, &hosts::drive_file_url(&item.id), temp.path(), &|r, t| {
            report("downloading", r, t, "Downloading…");
        })
        .await?;
        let name = download::sanitize_filename(Some(item.name.clone()), file_id);
        return install_payload(report, temp.path(), &name, dest, fetched.size, hosts::DRIVE_UNAVAILABLE);
    }

    // Walk subfolders (a few levels deep) into a flat list of files.
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut pending: Vec<(PathBuf, Vec<hosts::DriveItem>, usize)> = vec![(PathBuf::new(), items, 0)];
    while let Some((rel, items, depth)) = pending.pop() {
        for item in items {
            let path = rel.join(download::sanitize_filename(Some(item.name), &item.id));
            if !item.is_folder {
                files.push((path, item.id));
            } else if depth < 3 {
                let (_, sub) = hosts::list_drive_folder(client, &item.id).await?;
                pending.push((path, sub, depth + 1));
            }
            if files.len() > DRIVE_FOLDER_MAX_FILES {
                return Err(format!(
                    "This Google Drive folder holds more than {} files, so it's probably not a single song. \
                     Use Open ↗ to pick what you need in your browser.",
                    DRIVE_FOLDER_MAX_FILES
                ));
            }
        }
    }

    let target = dest.join(download::sanitize_filename(Some(title), "Google Drive folder"));
    let created = !target.exists();
    let total = files.len() as u64;
    let result: Result<(), String> = async {
        for (i, (rel, id)) in files.iter().enumerate() {
            let msg = format!("Downloading file {} of {}…", i + 1, total);
            report("downloading", i as u64, total, &msg);
            let temp = TempFile::new(file_id)?;
            fetch_external_file(client, &hosts::drive_file_url(id), temp.path(), &|_, _| {}).await?;
            if download::looks_like_html(&download::read_head(temp.path(), 512)) {
                return Err(hosts::DRIVE_UNAVAILABLE.to_string());
            }
            let out = target.join(rel);
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("Failed to create folder: {}", e))?;
            }
            download::place_file(temp.path(), &out)?;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        if created {
            let _ = fs::remove_dir_all(&target);
        }
        return Err(e);
    }
    Ok((target, files.len()))
}

/// Download one file and install it into `dest_folder`.
///
/// Self-hosted files go through RhythmVerse's `/download/{file_id}`
/// interstitial; off-site ones (`external_url`) are resolved per host (Google
/// Drive file or folder, Mediafire, Dropbox, shorteners). Either way the bytes
/// stream to a temp file (checked against Content-Length), then get extracted
/// (zip / 7z / RAR) or saved as a loose package, and the file_id is recorded
/// locally so the library badge stays exact.
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
    external_url: Option<String>,
) -> Result<RvDownloadResult, String> {
    let artist = artist.unwrap_or_default();
    let title = title.unwrap_or_default();
    let file_name = file_name.unwrap_or_default();
    let uploaded = uploaded.unwrap_or_default();

    let dest = PathBuf::from(&dest_folder);
    if !dest.is_dir() {
        return Err(format!("Destination folder does not exist: {}", dest_folder));
    }

    let source = match external_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        None => hosts::Source::RhythmVerse(None),
        Some(u) => hosts::classify(u).ok_or_else(|| hosts::unsupported_message(u))?,
    };

    emit_progress(&app, &file_id, "starting", 0, 0, "Starting…");
    let client = build_download_client()?;
    let report = |phase: &str, received: u64, total: u64, message: &str| {
        emit_progress(&app, &file_id, phase, received, total, message);
    };
    let on_progress = |received, total| report("downloading", received, total, "Downloading…");

    let (extracted_to, entries) = match source {
        hosts::Source::RhythmVerse(linked_id) => {
            let rv_id = linked_id.unwrap_or_else(|| file_id.clone());
            let (file_url, referer) = rv_file_url(&client, &rv_id).await?;
            emit_progress(&app, &file_id, "downloading", 0, 0, "Downloading…");
            let temp = TempFile::new(&file_id)?;
            let fetched = download::fetch_to_file(
                &client,
                &file_url,
                &[("Referer", referer.as_str())],
                temp.path(),
                &on_progress,
            )
            .await?;
            let filename = download::sanitize_filename(fetched.filename, &format!("{}.bin", file_id));
            install_payload(
                &report,
                temp.path(),
                &filename,
                &dest,
                fetched.size,
                "RhythmVerse returned a web page instead of a file — this download may require signing in.",
            )?
        }
        other => match hosts::resolve(&client, other).await? {
            hosts::Plan::File(url) => {
                emit_progress(&app, &file_id, "downloading", 0, 0, "Downloading…");
                let temp = TempFile::new(&file_id)?;
                let fetched = fetch_external_file(&client, &url, temp.path(), &on_progress).await?;
                // Off-site hosts don't always name the file; fall back to the
                // listing's name (CON packages often have no extension at all).
                let fallback = if file_name.is_empty() { format!("{}.bin", file_id) } else { file_name.clone() };
                let filename = download::sanitize_filename(fetched.filename, &fallback);
                install_payload(
                    &report,
                    temp.path(),
                    &filename,
                    &dest,
                    fetched.size,
                    "The link returned a web page instead of a file. Use Open ↗ to download it in your browser.",
                )?
            }
            hosts::Plan::DriveFolder(folder_id) => {
                download_drive_folder(&report, &client, &file_id, &folder_id, &dest).await?
            }
        },
    };

    // If an earlier download of this same file landed at a different path
    // (e.g. the charter renamed the folder in an update), recycle the stale
    // copy so a re-download replaces the song instead of duplicating it.
    if let Some(old) = previous_dest(&app, &file_id) {
        let old_path = PathBuf::from(&old);
        if !old.is_empty() && old_path != extracted_to && old_path.exists() {
            // Recycle Bin only: if the drive has no bin, keep the old copy rather
            // than deleting it permanently without asking (the duplicate finder
            // will surface it).
            if let Err(e) = crate::commands::move_to_trash(&old) {
                eprintln!("failed to recycle previous version {}: {}", old, e);
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

    emit_progress(&app, &file_id, "done", 1, 1, "Done");

    Ok(RvDownloadResult {
        file_id,
        extracted_to: extracted_to.to_string_lossy().to_string(),
        entries,
    })
}

/// Finish a "Fix it" re-download: move the broken copy to the Recycle Bin and,
/// when the new install sits in the same folder under a different name, give
/// it the broken copy's name so the library layout stays the same. Returns the
/// song's final path.
#[tauri::command]
pub fn rv_replace_broken(app: AppHandle, broken_path: String, new_path: String) -> Result<String, String> {
    let final_path = replace_broken(&broken_path, &new_path)?;
    if final_path != new_path {
        if let Ok(conn) = open_db(&app) {
            let _ = conn.execute(
                "UPDATE rv_downloads SET dest_path = ?1 WHERE dest_path = ?2",
                rusqlite::params![final_path, new_path],
            );
        }
    }
    Ok(final_path)
}

/// File half of `rv_replace_broken`: recycle the broken copy, then rename the
/// new install onto its name when both sit in the same folder.
fn replace_broken(broken_path: &str, new_path: &str) -> Result<String, String> {
    // The download landed on top of the broken copy (same folder or file
    // name), so it already is the replacement.
    if broken_path.eq_ignore_ascii_case(new_path) {
        return Ok(new_path.to_string());
    }
    let broken = PathBuf::from(broken_path);
    let new = PathBuf::from(new_path);
    if broken.exists() {
        crate::commands::move_to_trash(broken_path).map_err(|e| {
            format!(
                "Downloaded the new copy to {}, but couldn't move the broken one to the Recycle Bin ({}).                  Delete it manually.",
                new_path, e
            )
        })?;
    }
    if new.parent() == broken.parent() && !broken.exists() && fs::rename(&new, &broken).is_ok() {
        return Ok(broken_path.to_string());
    }
    Ok(new_path.to_string())
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
