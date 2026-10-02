//! Offline RhythmVerse catalog: a local copy of every YARG-playable chart on
//! RhythmVerse (~140k rows) in `yargle.db`, kept fresh by a background sync.
//!
//! Browsing the catalog is instant, works offline, and supports filters the
//! site can't do (charter, genre, year, length, date added, instruments, "hide
//! songs I already have", random picks). Rows use the same `RvSongFile` shape
//! as live browsing, so the UI renders both identically.
//!
//! Sync (adapted from Clone Hero Chart Manager's `catalogsync.ts`):
//! - **Full build:** walk the `list` endpoint newest-first, 500 rows a page,
//!   3 pages in flight. Progress is saved per page batch, so a build
//!   interrupted by closing the app resumes (if it's under 30 days old).
//! - **Delta:** walk newest-first again and stop at the first page whose rows
//!   are all older than the cursor minus 24 h. The site's "newest first" order
//!   is only approximate (files cluster by song), hence the overlap.
//! - The cursor is a RhythmVerse timestamp and is only ever compared with
//!   other RhythmVerse timestamps: the site's clock isn't reliably UTC.
//! - Deleted charts are deliberately not reconciled.

use crate::rhythmverse::{self, cs, RvBrowseResult, RvSongFile, RvSongRaw};
use crate::download::hosts;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Bump to force every install to re-sync in place (e.g. after adding a
/// column that existing rows need filled).
const DATA_VERSION: &str = "1";
const GAME: &str = "yarg";
const PAGE_SIZE: u32 = 500;
const CONCURRENCY: usize = 3;
const PAGE_GAP: Duration = Duration::from_millis(150);
const RETRIES: u32 = 2;
const STARTUP_DELAY: Duration = Duration::from_secs(8);
const SYNC_INTERVAL: Duration = Duration::from_secs(30 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(5 * 60);
const RESUME_MAX_AGE_DAYS: i64 = 30;
const DELTA_OVERLAP_HOURS: i64 = 24;
/// The list endpoint won't page past this many rows with `data_type=full`
/// (later pages silently return page 250 again), so the oldest rows are
/// fetched from the other end, oldest-first.
const MAX_LIST_OFFSET: u32 = 125_000;
/// Extra oldest-first pages so the two halves overlap even if uploads shift
/// rows across the boundary while a build is paused.
const TAIL_OVERLAP_PAGES: u32 = 2;
/// Safety cap so a misbehaving sort order can't turn a delta into a full walk.
const DELTA_MAX_PAGES: u32 = 40;

const RV_DATE_FMT: &str = "%Y-%m-%d %H:%M:%S";

// ===== Status (shared with the UI) =====

#[derive(Debug, Clone, Serialize, Default)]
pub struct CatalogStatus {
    /// A full build has finished at least once, so the browser can query the
    /// catalog instead of the live API.
    pub ready: bool,
    pub syncing: bool,
    /// "full" or "delta" while syncing.
    pub mode: String,
    pub pages_done: u32,
    pub pages_total: u32,
    pub rows: i64,
    /// Local time (RFC 3339) of the last successful sync.
    pub last_sync: String,
    pub error: Option<String>,
}

static STATUS: Mutex<Option<CatalogStatus>> = Mutex::new(None);
static SYNCING: AtomicBool = AtomicBool::new(false);

fn update_status(app: &AppHandle, f: impl FnOnce(&mut CatalogStatus)) {
    let snapshot = {
        let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
        let st = guard.get_or_insert_with(CatalogStatus::default);
        f(st);
        st.clone()
    };
    let _ = app.emit("catalog-sync", snapshot);
}

// ===== Database =====

fn open(app: &AppHandle) -> Result<Connection, String> {
    // Reuse the RhythmVerse handle so `rv_downloads` exists for "hide owned".
    let conn = rhythmverse::open_db(app)?;
    conn.busy_timeout(Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rv_catalog (
            file_id TEXT PRIMARY KEY,
            song_id INTEGER,
            title TEXT COLLATE NOCASE,
            artist TEXT COLLATE NOCASE,
            album TEXT COLLATE NOCASE,
            genre TEXT,
            subgenre TEXT,
            year INTEGER,
            decade TEXT,
            song_length_sec INTEGER,
            album_art_url TEXT,
            charter TEXT COLLATE NOCASE,
            gameformat TEXT,
            gamesource TEXT,
            size_bytes INTEGER,
            downloads INTEGER,
            uploader TEXT COLLATE NOCASE,
            uploaded TEXT,
            updated TEXT,
            file_name TEXT,
            external_url TEXT,
            diff_guitar INTEGER,
            diff_bass INTEGER,
            diff_drums INTEGER,
            diff_vocals INTEGER,
            diff_keys INTEGER,
            search TEXT,
            key_at TEXT,
            key_ta TEXT,
            -- 1 when key_at/key_ta matches a library song (see catalog_set_owned)
            owned INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS rv_catalog_updated ON rv_catalog(updated);
        CREATE INDEX IF NOT EXISTS rv_catalog_uploaded ON rv_catalog(uploaded);
        CREATE INDEX IF NOT EXISTS rv_catalog_downloads ON rv_catalog(downloads);
        CREATE INDEX IF NOT EXISTS rv_catalog_title ON rv_catalog(title);
        CREATE INDEX IF NOT EXISTS rv_catalog_artist ON rv_catalog(artist);
        CREATE TABLE IF NOT EXISTS catalog_meta (key TEXT PRIMARY KEY, value TEXT);
        CREATE TABLE IF NOT EXISTS catalog_owned (key TEXT PRIMARY KEY);",
    )
    .map_err(|e| format!("Catalog init failed: {}", e))
}

fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM catalog_meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO catalog_meta (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn row_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM rv_catalog", [], |r| r.get(0))
        .unwrap_or(0)
}

/// Same normalization as the browser's `norm()`: lowercase, letters/digits only.
fn norm_key(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// Search text: lowercase words, punctuation dropped (so "AC/DC" → "acdc").
fn search_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if c.is_whitespace() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_string()
}

/// A RhythmVerse timestamp field, or None for unset (`0000-…`, `false`, "").
fn rv_date(v: &serde_json::Value, key: &str) -> Option<String> {
    let s = cs(v, key);
    (s.len() >= 10 && s.as_bytes()[0].is_ascii_digit() && !s.starts_with("0000")).then_some(s)
}

/// The sync cursor field: when the file record last changed.
fn updated_of(raw: &RvSongRaw) -> String {
    let f = &raw.file;
    rv_date(f, "update_date")
        .or_else(|| rv_date(f, "file_updated"))
        .or_else(|| rv_date(f, "upload_date"))
        .or_else(|| rv_date(f, "record_created"))
        .unwrap_or_default()
}

/// Upsert one page of raw rows; returns the newest `updated` value seen and
/// whether every row was older than `older_than` (empty = never).
fn upsert_page(
    conn: &mut Connection,
    rows: &[RvSongRaw],
    older_than: &str,
) -> Result<(String, bool), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut newest = String::new();
    let mut all_older = !older_than.is_empty();
    {
        let mut stmt = tx
            .prepare_cached(
                "INSERT OR REPLACE INTO rv_catalog (
                    file_id, song_id, title, artist, album, genre, subgenre, year, decade,
                    song_length_sec, album_art_url, charter, gameformat, gamesource, size_bytes,
                    downloads, uploader, uploaded, updated, file_name, external_url,
                    diff_guitar, diff_bass, diff_drums, diff_vocals, diff_keys,
                    search, key_at, key_ta, owned
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                    ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29,
                    ?28 IN (SELECT key FROM catalog_owned) OR ?29 IN (SELECT key FROM catalog_owned))",
            )
            .map_err(|e| e.to_string())?;
        for raw in rows {
            let updated = updated_of(raw);
            if updated > newest {
                newest = updated.clone();
            }
            if all_older && updated.as_str() >= older_than {
                all_older = false;
            }
            let Some(s) = rhythmverse::map_song(raw) else { continue };
            let search = search_words(&format!(
                "{} {} {} {} {}",
                s.title, s.artist, s.album, s.charter, s.uploader
            ));
            stmt.execute(params![
                s.file_id,
                s.song_id,
                s.title,
                s.artist,
                s.album,
                s.genre,
                s.subgenre,
                s.year,
                s.decade,
                s.song_length_sec,
                s.album_art_url,
                s.charter,
                s.gameformat,
                s.gamesource,
                s.size_bytes,
                s.downloads,
                s.uploader,
                s.uploaded,
                updated,
                s.file_name,
                s.external_url,
                s.diff_guitar,
                s.diff_bass,
                s.diff_drums,
                s.diff_vocals,
                s.diff_keys,
                search,
                norm_key(&format!("{}{}", s.artist, s.title)),
                norm_key(&format!("{}{}", s.title, s.artist)),
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok((newest, all_older))
}

/// Escape LIKE wildcards for use with `ESCAPE '\'`.
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

// ===== Sync =====

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(rhythmverse::USER_AGENT)
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// One page of the list (newest-first, or oldest-first for the tail), retried
/// on failure. Returns the raw rows and the total row count the site reports.
async fn fetch_page(
    client: &reqwest::Client,
    page: u32,
    newest_first: bool,
) -> Result<(Vec<RvSongRaw>, i64), String> {
    let url = format!("{}/api/{}/songfiles/list", rhythmverse::BASE, GAME);
    let params: Vec<(&str, String)> = vec![
        ("data_type", "full".into()),
        ("page", page.to_string()),
        ("records", PAGE_SIZE.to_string()),
        ("sort[0][sort_by]", "update_date".into()),
        ("sort[0][sort_order]", if newest_first { "DESC" } else { "ASC" }.into()),
    ];
    let mut attempt = 0;
    loop {
        match rhythmverse::post_songfiles(client, &url, &params).await {
            Ok(data) => return Ok((data.songs, data.records.total_filtered)),
            Err(e) if attempt >= RETRIES => return Err(format!("page {}: {}", page, e)),
            Err(_) => {
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
            }
        }
    }
}

fn pages_for(total: i64) -> u32 {
    ((total.max(0) as u64 + PAGE_SIZE as u64 - 1) / PAGE_SIZE as u64) as u32
}

/// A full build walks "build pages" 1..=N: newest-first pages up to the
/// server's offset cap, then oldest-first pages for the remaining tail.
/// Returns (newest-first page count, total build pages).
fn build_plan(total: i64) -> (u32, u32) {
    let pages = pages_for(total);
    let head = pages.min(MAX_LIST_OFFSET / PAGE_SIZE);
    let tail_rows = total - (head as i64) * PAGE_SIZE as i64;
    let tail = if tail_rows > 0 { pages_for(tail_rows) + TAIL_OVERLAP_PAGES } else { 0 };
    (head, head + tail)
}

/// Build page `n` → (API page, newest_first).
fn plan_page(n: u32, head: u32) -> (u32, bool) {
    if n <= head {
        (n, true)
    } else {
        (n - head, false)
    }
}

fn now_local() -> String {
    chrono::Local::now().to_rfc3339()
}

/// `ts` (an RV timestamp) shifted back by `hours`, in the same format.
fn rv_minus_hours(ts: &str, hours: i64) -> String {
    chrono::NaiveDateTime::parse_from_str(ts, RV_DATE_FMT)
        .map(|t| (t - chrono::Duration::hours(hours)).format(RV_DATE_FMT).to_string())
        .unwrap_or_default()
}

async fn full_build(app: &AppHandle, client: &reqwest::Client) -> Result<(), String> {
    let mut conn = open(app)?;

    // Resume an interrupted build if it's recent enough; otherwise start over.
    let started = meta_get(&conn, "rv:build_started").unwrap_or_default();
    let fresh_enough = chrono::DateTime::parse_from_rfc3339(&started)
        .map(|t| chrono::Local::now().signed_duration_since(t).num_days() < RESUME_MAX_AGE_DAYS)
        .unwrap_or(false);
    let mut done: u32 = if fresh_enough {
        meta_get(&conn, "rv:build_page").and_then(|v| v.parse().ok()).unwrap_or(0)
    } else {
        0
    };

    let (first, total) = fetch_page(client, 1, true).await?;
    let (head, pages_total) = build_plan(total);
    let (newest, _) = upsert_page(&mut conn, &first, "")?;
    if done == 0 {
        // The newest row at the start of the build becomes the delta cursor.
        meta_set(&conn, "rv:build_started", &now_local())?;
        meta_set(&conn, "rv:build_cursor", &newest)?;
        done = 1;
        meta_set(&conn, "rv:build_page", "1")?;
    }
    update_status(app, |s| {
        s.pages_done = done.min(pages_total);
        s.pages_total = pages_total;
        s.rows = row_count(&conn);
    });

    let mut next = done + 1;
    while next <= pages_total {
        let batch: Vec<u32> = (next..=pages_total).take(CONCURRENCY).collect();
        let mut handles = Vec::with_capacity(batch.len());
        for (i, &page) in batch.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(PAGE_GAP).await;
            }
            let client = client.clone();
            let (api_page, newest_first) = plan_page(page, head);
            handles.push(tauri::async_runtime::spawn(async move {
                fetch_page(&client, api_page, newest_first).await
            }));
        }
        // Await in order and stop at the first failure, so `rv:build_page`
        // only ever covers a contiguous run of saved pages.
        for (h, &page) in handles.into_iter().zip(batch.iter()) {
            let (rows, _) = h.await.map_err(|e| e.to_string())??;
            upsert_page(&mut conn, &rows, "")?;
            meta_set(&conn, "rv:build_page", &page.to_string())?;
            done = page;
        }
        update_status(app, |s| {
            s.pages_done = done;
            s.rows = row_count(&conn);
        });
        next = done + 1;
        tokio::time::sleep(PAGE_GAP).await;
    }

    let cursor = meta_get(&conn, "rv:build_cursor").unwrap_or_default();
    meta_set(&conn, "rv:cursor", &cursor)?;
    meta_set(&conn, "rv:full_done", "1")?;
    meta_set(&conn, "rv:ever_done", "1")?;
    meta_set(&conn, "rv:build_page", "0")?;
    Ok(())
}

async fn delta_sync(app: &AppHandle, client: &reqwest::Client) -> Result<(), String> {
    let mut conn = open(app)?;
    let cursor = meta_get(&conn, "rv:cursor").unwrap_or_default();
    let threshold = rv_minus_hours(&cursor, DELTA_OVERLAP_HOURS);
    let mut newest_seen = cursor.clone();
    let mut page = 1;
    loop {
        let (rows, total) = fetch_page(client, page, true).await?;
        let (newest, all_older) = upsert_page(&mut conn, &rows, &threshold)?;
        if newest > newest_seen {
            newest_seen = newest;
        }
        let pages_total = pages_for(total);
        update_status(app, |s| {
            s.pages_done = page;
            s.pages_total = pages_total;
        });
        if rows.is_empty() || all_older || threshold.is_empty() || page >= pages_total || page >= DELTA_MAX_PAGES {
            break;
        }
        page += 1;
        tokio::time::sleep(PAGE_GAP).await;
    }
    meta_set(&conn, "rv:cursor", &newest_seen)?;
    Ok(())
}

/// Run one sync pass (full build or delta). Returns false on failure.
/// A no-op returning true if a sync is already running.
async fn sync_once(app: &AppHandle) -> bool {
    if SYNCING.swap(true, Ordering::SeqCst) {
        return true;
    }
    let result = async {
        let conn = open(app)?;
        if meta_get(&conn, "rv:data_version").as_deref() != Some(DATA_VERSION) {
            // Re-sync in place: rows stay browsable while they're refreshed.
            meta_set(&conn, "rv:full_done", "0")?;
            meta_set(&conn, "rv:build_page", "0")?;
            meta_set(&conn, "rv:data_version", DATA_VERSION)?;
        }
        let full = meta_get(&conn, "rv:full_done").as_deref() != Some("1");
        drop(conn);
        update_status(app, |s| {
            s.syncing = true;
            s.mode = if full { "full" } else { "delta" }.into();
            s.pages_done = 0;
            s.pages_total = 0;
            s.error = None;
        });
        let client = build_client()?;
        if full {
            full_build(app, &client).await
        } else {
            delta_sync(app, &client).await
        }
    }
    .await;

    let ok = result.is_ok();
    let conn = open(app).ok();
    if ok {
        if let Some(c) = &conn {
            let _ = meta_set(c, "rv:last_sync", &now_local());
        }
    }
    update_status(app, |s| {
        s.syncing = false;
        s.mode.clear();
        s.error = result.err();
        if let Some(c) = &conn {
            s.ready = meta_get(c, "rv:ever_done").as_deref() == Some("1");
            s.rows = row_count(c);
            s.last_sync = meta_get(c, "rv:last_sync").unwrap_or_default();
        }
    });
    SYNCING.store(false, Ordering::SeqCst);
    ok
}

/// Start the background sync loop: shortly after launch, then every 30 min
/// (5 min after a failure).
pub fn start(app: AppHandle) {
    if let Ok(conn) = open(&app) {
        let st = CatalogStatus {
            ready: meta_get(&conn, "rv:ever_done").as_deref() == Some("1"),
            rows: row_count(&conn),
            last_sync: meta_get(&conn, "rv:last_sync").unwrap_or_default(),
            ..Default::default()
        };
        *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(st);
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            let ok = sync_once(&app).await;
            tokio::time::sleep(if ok { SYNC_INTERVAL } else { RETRY_INTERVAL }).await;
        }
    });
}

// ===== Commands =====

#[tauri::command]
pub fn catalog_status() -> CatalogStatus {
    STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// Kick off a sync now (no-op if one is already running).
#[tauri::command]
pub fn catalog_sync_now(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        sync_once(&app).await;
    });
}

/// Replace the set of normalized artist+title keys for songs in the user's
/// library, used by the "hide songs I already have" filter.
#[tauri::command]
pub async fn catalog_set_owned(app: AppHandle, keys: Vec<String>) -> Result<(), String> {
    let mut conn = open(&app)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM catalog_owned", []).map_err(|e| e.to_string())?;
    {
        let mut stmt = tx
            .prepare("INSERT OR IGNORE INTO catalog_owned (key) VALUES (?1)")
            .map_err(|e| e.to_string())?;
        for k in keys.iter().filter(|k| !k.is_empty()) {
            stmt.execute([k]).map_err(|e| e.to_string())?;
        }
    }
    // Precompute the flag so the filter is a column check, not two lookups
    // per row on every query. Only rows whose state changed are written.
    tx.execute(
        "UPDATE rv_catalog SET owned = 1 - owned
         WHERE owned <> (key_at IN (SELECT key FROM catalog_owned)
                         OR key_ta IN (SELECT key FROM catalog_owned))",
        [],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CatalogQuery {
    pub text: String,
    pub charter: String,
    pub genre: String,
    pub gameformat: String,
    /// Instruments that must be charted: guitar, bass, drums, vocals, keys.
    pub instruments: Vec<String>,
    pub year_min: Option<i64>,
    pub year_max: Option<i64>,
    pub length_min: Option<i64>,
    pub length_max: Option<i64>,
    pub added_within_days: Option<i64>,
    pub hide_owned: bool,
    pub sort_by: String,
    pub sort_order: String,
    pub page: u32,
    pub records: u32,
    /// Pick `records` random rows matching the filters ("Surprise me").
    pub random: bool,
}

const COLUMNS: &str = "file_id, song_id, title, artist, album, genre, subgenre, year, decade,
    song_length_sec, album_art_url, charter, gameformat, gamesource, size_bytes, downloads,
    uploader, uploaded, file_name, external_url,
    diff_guitar, diff_bass, diff_drums, diff_vocals, diff_keys";

fn row_to_song(r: &rusqlite::Row) -> rusqlite::Result<RvSongFile> {
    let file_id: String = r.get(0)?;
    let external_url: String = r.get::<_, Option<String>>(19)?.unwrap_or_default();
    let text = |i: usize| -> rusqlite::Result<String> {
        Ok(r.get::<_, Option<String>>(i)?.unwrap_or_default())
    };
    Ok(RvSongFile {
        song_id: r.get(1)?,
        title: text(2)?,
        artist: text(3)?,
        album: text(4)?,
        genre: text(5)?,
        subgenre: text(6)?,
        year: r.get(7)?,
        decade: text(8)?,
        song_length_sec: r.get(9)?,
        album_art_url: text(10)?,
        charter: text(11)?,
        gameformat: text(12)?,
        gamesource: text(13)?,
        size_bytes: r.get(14)?,
        downloads: r.get(15)?,
        uploader: text(16)?,
        uploaded: text(17)?,
        file_name: text(18)?,
        detail_url: format!("{}/songfile/{}", rhythmverse::BASE, file_id),
        download_url: format!("{}/download/{}", rhythmverse::BASE, file_id),
        external_auto: !external_url.is_empty() && hosts::classify(&external_url).is_some(),
        external_url,
        diff_guitar: r.get(20)?,
        diff_bass: r.get(21)?,
        diff_drums: r.get(22)?,
        diff_vocals: r.get(23)?,
        diff_keys: r.get(24)?,
        file_id,
    })
}

/// Browse/search the offline catalog. Returns the same shape as `rv_browse`.
#[tauri::command]
pub async fn catalog_query(app: AppHandle, query: CatalogQuery) -> Result<RvBrowseResult, String> {
    let conn = open(&app)?;
    let mut wh: Vec<String> = Vec::new();
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    let mut arg = |v: rusqlite::types::Value| {
        args.push(v);
        format!("?{}", args.len())
    };

    for word in search_words(&query.text).split(' ').filter(|w| !w.is_empty()) {
        let p = arg(format!("%{}%", word).into());
        wh.push(format!("search LIKE {}", p));
    }
    let charter = query.charter.trim();
    if !charter.is_empty() {
        let p = arg(format!("%{}%", like_escape(charter)).into());
        wh.push(format!("(charter LIKE {p} ESCAPE '\\' OR uploader LIKE {p} ESCAPE '\\')"));
    }
    if !query.genre.is_empty() {
        let p = arg(query.genre.clone().into());
        wh.push(format!("genre = {}", p));
    }
    if !query.gameformat.is_empty() {
        let p = arg(query.gameformat.to_lowercase().into());
        wh.push(format!("lower(gameformat) = {}", p));
    }
    for inst in &query.instruments {
        let col = match inst.as_str() {
            "guitar" => "diff_guitar",
            "bass" => "diff_bass",
            "drums" => "diff_drums",
            "vocals" => "diff_vocals",
            "keys" => "diff_keys",
            _ => continue,
        };
        wh.push(format!("{} >= 1", col));
    }
    if let Some(v) = query.year_min {
        let p = arg(v.into());
        wh.push(format!("year >= {}", p));
    }
    if let Some(v) = query.year_max {
        let p = arg(v.into());
        wh.push(format!("year <= {}", p));
    }
    if let Some(v) = query.length_min {
        let p = arg(v.into());
        wh.push(format!("song_length_sec >= {}", p));
    }
    if let Some(v) = query.length_max {
        let p = arg(v.into());
        wh.push(format!("song_length_sec <= {}", p));
    }
    if let Some(days) = query.added_within_days.filter(|d| *d > 0) {
        // Day-scale window, so the site's few-minute clock skew is irrelevant.
        let since = (chrono::Utc::now() - chrono::Duration::days(days))
            .format(RV_DATE_FMT)
            .to_string();
        let p = arg(since.into());
        wh.push(format!("uploaded >= {}", p));
    }
    if query.hide_owned {
        wh.push(
            "owned = 0 AND file_id NOT IN (SELECT file_id FROM rv_downloads)".into(),
        );
    }
    let where_sql = if wh.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", wh.join(" AND "))
    };

    let total_available = row_count(&conn);
    let total_filtered: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM rv_catalog {}", where_sql),
            params_from_iter(args.iter()),
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    let records = query.records.clamp(1, 200);
    let page = query.page.max(1);
    let order_sql = if query.random {
        "ORDER BY random()".to_string()
    } else {
        let dir = if query.sort_order.eq_ignore_ascii_case("ASC") { "ASC" } else { "DESC" };
        let col = match query.sort_by.as_str() {
            "uploaded" => "uploaded",
            "downloads" => "downloads",
            "title" => "title",
            "artist" => "artist",
            "length" => "song_length_sec",
            "year" => "year",
            _ => "updated",
        };
        // Missing values always sort last, whichever direction.
        let tie = if col == "artist" { ", title ASC" } else { "" };
        format!("ORDER BY ({col} IS NULL OR {col} = ''), {col} {dir}{tie}, file_id")
    };
    let offset = if query.random { 0 } else { (page as i64 - 1) * records as i64 };
    let sql = format!(
        "SELECT {} FROM rv_catalog {} {} LIMIT {} OFFSET {}",
        COLUMNS, where_sql, order_sql, records, offset
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let songs: Vec<RvSongFile> = stmt
        .query_map(params_from_iter(args.iter()), row_to_song)
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(RvBrowseResult {
        returned: songs.len() as i64,
        songs,
        total_available,
        total_filtered,
        page,
    })
}

#[derive(Debug, Serialize)]
pub struct FacetCount {
    pub value: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct CatalogFacets {
    pub genres: Vec<FacetCount>,
    pub formats: Vec<FacetCount>,
}

/// Option lists for the filter dropdowns.
#[tauri::command]
pub async fn catalog_facets(app: AppHandle) -> Result<CatalogFacets, String> {
    let conn = open(&app)?;
    let facet = |sql: &str| -> Result<Vec<FacetCount>, String> {
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok(FacetCount { value: r.get(0)?, count: r.get(1)? }))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    };
    Ok(CatalogFacets {
        genres: facet(
            "SELECT genre, COUNT(*) AS n FROM rv_catalog WHERE genre <> ''
             GROUP BY genre HAVING n >= 10 ORDER BY genre COLLATE NOCASE",
        )?,
        formats: facet(
            "SELECT lower(gameformat), COUNT(*) AS n FROM rv_catalog WHERE gameformat <> ''
             GROUP BY lower(gameformat) ORDER BY n DESC",
        )?,
    })
}

#[derive(Debug, Serialize)]
pub struct CatalogSuggestion {
    /// "artist" or "title".
    pub kind: String,
    pub value: String,
    pub count: i64,
}

/// Type-ahead: artists and titles starting with the typed text, most common first.
#[tauri::command]
pub async fn catalog_suggest(app: AppHandle, text: String) -> Result<Vec<CatalogSuggestion>, String> {
    let t = text.trim();
    if t.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let conn = open(&app)?;
    let pattern = format!("{}%", like_escape(t));
    let mut out = Vec::new();
    for (kind, col, limit) in [("artist", "artist", 5), ("title", "title", 5)] {
        let sql = format!(
            "SELECT {col}, COUNT(*) AS n FROM rv_catalog WHERE {col} LIKE ?1 ESCAPE '\\'
             GROUP BY {col} COLLATE NOCASE ORDER BY n DESC LIMIT {limit}"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&pattern], |r| {
                Ok(CatalogSuggestion { kind: kind.into(), value: r.get(0)?, count: r.get(1)? })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok());
        out.extend(rows);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_browser_norm() {
        assert_eq!(norm_key("AC/DC - Back in Black"), "acdcbackinblack");
        assert_eq!(search_words("Guns N' Roses  AC/DC"), "guns n roses acdc");
    }

    #[test]
    fn plan_covers_tail() {
        // 138,075 rows: 250 newest-first pages, then 27 + 2 oldest-first.
        assert_eq!(build_plan(138_075), (250, 279));
        assert_eq!(plan_page(250, 250), (250, true));
        assert_eq!(plan_page(251, 250), (1, false));
        assert_eq!(build_plan(1_000), (2, 2));
    }

    #[test]
    fn cursor_overlap() {
        assert_eq!(rv_minus_hours("2026-10-02 04:50:11", 24), "2026-10-01 04:50:11");
        assert_eq!(rv_minus_hours("", 24), "");
    }
}
