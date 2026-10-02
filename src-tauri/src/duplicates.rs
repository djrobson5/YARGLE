//! Duplicate finder.
//!
//! 1. Group every song by normalized `artist|title` (names come from the scan,
//!    so this pass reads nothing from disk). Groups of one are dropped.
//! 2. Inspect only the songs left in those groups: hash the chart (MD5 of
//!    `notes.chart`, else `notes.mid`; for CON packages the inner `.mid`) and
//!    note each copy's extras (art, backgrounds, video, separate instrument
//!    tracks…). Results are cached in `yargle.db`, keyed by mtime + size.
//! 3. A group whose copies all share one chart hash is "identical" (the extra
//!    copies are safe to remove); otherwise it's "different versions" (other
//!    charters or revisions), and the user decides.

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use tauri::{AppHandle, Emitter};

use crate::dta::parser::{extract_metadata, parse_dta};
use crate::dta::types::SongMetadata;
use crate::song_ini;
use crate::stfs::filesystem::StfsReader;

/// What the frontend sends per song (straight from its `SongSummary` list).
#[derive(Deserialize)]
pub struct DupInput {
    pub path: String,
    pub artist: String,
    pub song_title: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
}

/// Extra content a copy carries, so the user can keep the richest one.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Extras {
    pub album_art: bool,
    pub background: bool,
    pub highway: bool,
    pub video: bool,
    /// Separate instrument audio tracks ("stems"), e.g. `["drums", "guitar"]`.
    pub stems: Vec<String>,
    /// CON packages: audio channels in the multitrack `.mogg` (from `pans`).
    pub audio_channels: Option<u32>,
    /// CON packages: has a `.milo` (lipsync / venue data).
    pub venue: bool,
}

/// Per-copy facts that cost disk I/O to learn; cached in `yargle.db`.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
struct Inspected {
    chart_hash: Option<String>,
    /// "notes.chart", "notes.mid" or "mid" (inside a CON); empty if none found.
    chart_kind: String,
    size: u64,
    extras: Extras,
    /// drums, guitar, bass, vocals, keys
    instruments: [bool; 5],
}

#[derive(Serialize, Clone)]
pub struct DuplicateEntry {
    pub path: String,
    pub display_name: String,
    pub description: String,
    pub is_folder: bool,
    pub file_size: u64,
    pub chart_hash: Option<String>,
    pub chart_kind: String,
    /// Copies in the same group with equal `match_id` have byte-identical
    /// charts. `None` = no chart could be read.
    pub match_id: Option<u32>,
    pub has_drums: bool,
    pub has_guitar: bool,
    pub has_bass: bool,
    pub has_vocals: bool,
    pub has_keys: bool,
    pub extras: Extras,
}

#[derive(Serialize, Clone)]
pub struct DuplicateGroup {
    pub key: String,
    pub display_name: String,
    /// "identical" (every copy has the same chart) or "versions".
    pub kind: String,
    pub entries: Vec<DuplicateEntry>,
}

#[derive(Serialize, Clone)]
struct DuplicateScanProgress {
    current: usize,
    total: usize,
    phase: String,
}

#[tauri::command]
pub async fn find_duplicates(app: AppHandle, songs: Vec<DupInput>) -> Result<Vec<DuplicateGroup>, String> {
    let db = crate::local_db::db_path(&app).ok();
    let emit = |current: usize, total: usize, phase: &str| {
        let _ = app.emit(
            "duplicate-scan-progress",
            DuplicateScanProgress { current, total, phase: phase.into() },
        );
    };
    find_groups(songs, db.as_deref(), &emit).await
}

pub async fn find_groups(
    songs: Vec<DupInput>,
    db: Option<&Path>,
    progress: &(dyn Fn(usize, usize, &str) + Send + Sync),
) -> Result<Vec<DuplicateGroup>, String> {
    use rayon::prelude::*;

    // 1. Group by name. Dedupe paths so overlapping scopes can't pair a song
    //    with itself.
    let mut seen = HashSet::new();
    let mut by_key: HashMap<String, Vec<DupInput>> = HashMap::new();
    for song in songs {
        if !seen.insert(song.path.clone()) {
            continue;
        }
        let key = group_key(&song);
        if !key.is_empty() {
            by_key.entry(key).or_default().push(song);
        }
    }
    let groups: Vec<(String, Vec<DupInput>)> = by_key.into_iter().filter(|(_, v)| v.len() > 1).collect();
    if groups.is_empty() {
        return Ok(vec![]);
    }

    // 2. Inspect the candidates (cache first, then disk).
    let candidates: Vec<&str> = groups.iter().flat_map(|(_, v)| v.iter().map(|s| s.path.as_str())).collect();
    let total = candidates.len();
    progress(0, total, "Comparing charts…");

    let cache = db.map(|d| load_cache(d)).unwrap_or_default();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .map_err(|e| e.to_string())?;
    let mut inspected: HashMap<String, Inspected> = HashMap::with_capacity(total);
    let mut new_rows: Vec<(String, i64, i64, Inspected)> = Vec::new();
    let mut done = 0usize;
    for chunk in candidates.chunks(100) {
        let results: Vec<(String, Option<(i64, i64, Inspected, bool)>)> = pool.install(|| {
            chunk
                .par_iter()
                .map(|path| (path.to_string(), inspect(Path::new(path), &cache)))
                .collect()
        });
        for (path, result) in results {
            if let Some((mtime, size, info, fresh)) = result {
                if fresh {
                    new_rows.push((path.clone(), mtime, size, info.clone()));
                }
                inspected.insert(path, info);
            }
        }
        done += chunk.len();
        progress(done, total, "Comparing charts…");
        tokio::task::yield_now().await;
    }
    if let Some(d) = db {
        store_cache(d, &new_rows);
    }

    // 3. Build the result.
    let mut result: Vec<DuplicateGroup> = groups
        .into_iter()
        .map(|(key, songs)| build_group(key, songs, &inspected))
        .collect();
    result.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    Ok(result)
}

fn build_group(key: String, songs: Vec<DupInput>, inspected: &HashMap<String, Inspected>) -> DuplicateGroup {
    let display_name = songs
        .iter()
        .find(|s| !s.artist.trim().is_empty() && !s.song_title.trim().is_empty())
        .map(|s| format!("{} - {}", strip_tags(s.artist.trim()), strip_tags(s.song_title.trim())))
        .unwrap_or_else(|| strip_tags(&songs[0].display_name));

    // Number distinct hashes in order of first appearance.
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut entries: Vec<DuplicateEntry> = songs
        .into_iter()
        .map(|s| {
            let info = inspected.get(&s.path).cloned().unwrap_or_default();
            let match_id = info.chart_hash.as_ref().map(|h| {
                let next = ids.len() as u32;
                *ids.entry(h.clone()).or_insert(next)
            });
            let [has_drums, has_guitar, has_bass, has_vocals, has_keys] = info.instruments;
            DuplicateEntry {
                is_folder: Path::new(&s.path).is_dir(),
                path: s.path,
                display_name: s.display_name,
                description: s.description,
                file_size: info.size,
                chart_hash: info.chart_hash,
                chart_kind: info.chart_kind,
                match_id,
                has_drums,
                has_guitar,
                has_bass,
                has_vocals,
                has_keys,
                extras: info.extras,
            }
        })
        .collect();

    let identical = ids.len() == 1 && entries.iter().all(|e| e.match_id.is_some());
    entries.sort_by(|a, b| {
        a.match_id
            .unwrap_or(u32::MAX)
            .cmp(&b.match_id.unwrap_or(u32::MAX))
            .then(b.file_size.cmp(&a.file_size))
    });
    DuplicateGroup {
        key,
        display_name,
        kind: if identical { "identical" } else { "versions" }.into(),
        entries,
    }
}

/// `artist|title`, normalized; falls back to the display name for songs whose
/// metadata couldn't be read.
fn group_key(song: &DupInput) -> String {
    let title = norm(&song.song_title);
    if title.is_empty() {
        return norm(&song.display_name);
    }
    format!("{}|{}", norm(&song.artist), title)
}

/// Remove rich-text tags such as `<color=#FF0000>` / `</b>`.
pub(crate) fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('<') {
        match rest[start..].find('>') {
            Some(end) => {
                out.push_str(&rest[..start]);
                rest = &rest[start + end + 1..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// Lowercase, tags stripped, punctuation folded to spaces, whitespace
/// collapsed, and a leading "the " dropped — so `The Who` and `Who`, or
/// `Don't Stop` and `Dont Stop`… mostly line up.
fn norm(s: &str) -> String {
    let cleaned: String = strip_tags(s)
        .to_lowercase()
        .chars()
        .filter(|c| *c != '\'' && *c != '’')
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.strip_prefix("the ").map(str::to_string).unwrap_or(joined)
}

// ---------- inspection ----------

/// Returns (mtime, size, info, fresh) where `fresh` = not from cache.
fn inspect(path: &Path, cache: &HashMap<String, (i64, i64, Inspected)>) -> Option<(i64, i64, Inspected, bool)> {
    let key = path.to_string_lossy().to_string();
    let hit = |mtime: i64, size: i64| {
        cache
            .get(&key)
            .filter(|(m, s, _)| *m == mtime && *s == size)
            .map(|(_, _, info)| info.clone())
    };
    if path.is_dir() {
        let listing = list_folder(path)?;
        let (mtime, size) = listing.signature;
        if let Some(info) = hit(mtime, size) {
            return Some((mtime, size, info, false));
        }
        Some((mtime, size, inspect_folder(path, &listing), true))
    } else {
        let md = fs::metadata(path).ok()?;
        let (mtime, size) = (mtime_secs(&md), md.len() as i64);
        if let Some(info) = hit(mtime, size) {
            return Some((mtime, size, info, false));
        }
        Some((mtime, size, inspect_con(path, md.len()), true))
    }
}

struct FolderListing {
    /// Lowercase file name → size.
    files: Vec<(String, u64)>,
    /// (mtime, size) used as the cache key: newest of the folder / song.ini /
    /// chart mtimes, and song.ini + chart sizes. Adding or removing files
    /// bumps the folder mtime; editing song.ini or the chart bumps theirs.
    signature: (i64, i64),
}

fn list_folder(dir: &Path) -> Option<FolderListing> {
    let mut mtime = fs::metadata(dir).map(|m| mtime_secs(&m)).unwrap_or(0);
    let mut size = 0i64;
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let Ok(md) = entry.metadata() else { continue };
        if !md.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if matches!(name.as_str(), "song.ini" | "notes.chart" | "notes.mid") {
            mtime = mtime.max(mtime_secs(&md));
            size += md.len() as i64;
        }
        files.push((name, md.len()));
    }
    Some(FolderListing { files, signature: (mtime, size) })
}

fn inspect_folder(dir: &Path, listing: &FolderListing) -> Inspected {
    let has = |n: &str| listing.files.iter().any(|(f, _)| f == n);
    let chart_kind = if has("notes.chart") {
        "notes.chart"
    } else if has("notes.mid") {
        "notes.mid"
    } else {
        ""
    };
    let chart_hash = if chart_kind.is_empty() {
        None
    } else {
        fs::read(dir.join(chart_kind)).ok().map(|b| md5_hex(&b))
    };
    let instruments = song_ini::read_song_ini(&dir.join("song.ini"))
        .map(|c| instruments_of(&song_ini::parse_song_ini(&c)))
        .unwrap_or_default();

    let names = || listing.files.iter().map(|(f, _)| f.as_str());
    let split = |f: &str| -> (String, String) {
        match f.rsplit_once('.') {
            Some((stem, ext)) => (stem.to_string(), ext.to_string()),
            None => (f.to_string(), String::new()),
        }
    };
    let is_img = |e: &str| matches!(e, "png" | "jpg" | "jpeg");
    let mut stems: Vec<String> = names()
        .filter_map(|f| {
            let (stem, ext) = split(f);
            (matches!(ext.as_str(), "ogg" | "opus" | "mp3" | "wav") && is_stem_name(&stem)).then_some(stem)
        })
        .collect();
    stems.sort();
    let extras = Extras {
        album_art: names().any(|f| {
            let (stem, ext) = split(f);
            stem == "album" && (is_img(&ext) || ext == "webp")
        }),
        background: names().any(|f| {
            let (stem, ext) = split(f);
            f == "bg.png"
                || (is_img(&ext)
                    && stem
                        .strip_prefix("background")
                        .map(|n| n.chars().all(|c| c.is_ascii_digit()))
                        .unwrap_or(false))
        }),
        highway: names().any(|f| {
            let (stem, ext) = split(f);
            stem == "highway" && is_img(&ext)
        }),
        video: names().any(|f| {
            let (_, ext) = split(f);
            matches!(ext.as_str(), "mp4" | "webm" | "avi" | "m4v" | "mpg")
        }),
        stems,
        audio_channels: None,
        venue: false,
    };
    Inspected {
        chart_hash,
        chart_kind: chart_kind.into(),
        size: listing.files.iter().map(|(_, s)| s).sum(),
        extras,
        instruments,
    }
}

fn is_stem_name(stem: &str) -> bool {
    matches!(
        stem,
        "guitar" | "guitar_1" | "guitar_2" | "bass" | "rhythm" | "drums" | "drums_1" | "drums_2" | "drums_3"
            | "drums_4" | "vocals" | "vocals_1" | "vocals_2" | "keys"
    )
}

fn inspect_con(path: &Path, size: u64) -> Inspected {
    let mut info = Inspected { size, ..Default::default() };
    let Ok(mut reader) = StfsReader::open(path) else { return info };

    if let Some(mid) = reader.find(|n| n.ends_with(".mid")) {
        if let Ok(bytes) = reader.extract(&mid) {
            info.chart_hash = Some(md5_hex(&bytes));
            info.chart_kind = "mid".into();
        }
    }
    if let Some(dta) = reader.find(|n| n == "songs.dta") {
        if let Ok(bytes) = reader.extract(&dta) {
            let raw = match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(e) => encoding_rs::WINDOWS_1252.decode(e.as_bytes()).0.into_owned(),
            };
            if let Ok(nodes) = parse_dta(&raw) {
                info.instruments = instruments_of(&extract_metadata(&nodes, &raw));
            }
            info.extras.audio_channels = count_pans(&raw);
        }
    }
    info.extras.album_art = reader.find(|n| n.contains("_keep.png")).is_some();
    info.extras.venue = reader.find(|n| n.ends_with(".milo_xbox")).is_some();
    info
}

/// Number of values in the DTA's `(pans (…))` list = audio channels.
fn count_pans(dta: &str) -> Option<u32> {
    let pos = dta.find("pans")?;
    let open = pos + dta[pos..].find('(')?;
    let close = open + dta[open..].find(')')?;
    let n = dta[open + 1..close]
        .split_whitespace()
        .filter(|t| t.parse::<f32>().is_ok())
        .count() as u32;
    (n > 0).then_some(n)
}

fn instruments_of(m: &SongMetadata) -> [bool; 5] {
    let on = |r: Option<i32>| r.map_or(false, |r| r > 0);
    [on(m.rank_drum), on(m.rank_guitar), on(m.rank_bass), on(m.rank_vocals), on(m.rank_keys)]
}

fn md5_hex(bytes: &[u8]) -> String {
    Md5::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect()
}

fn mtime_secs(md: &fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------- cache (yargle.db) ----------

fn open_cache(db: &Path) -> Option<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(db).ok()?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS duplicate_cache (
            path TEXT PRIMARY KEY,
            mtime INTEGER NOT NULL,
            size INTEGER NOT NULL,
            info TEXT NOT NULL
        )",
        [],
    )
    .ok()?;
    Some(conn)
}

/// Rows whose JSON no longer matches `Inspected` are skipped (re-inspected).
fn load_cache(db: &Path) -> HashMap<String, (i64, i64, Inspected)> {
    let mut map = HashMap::new();
    let Some(conn) = open_cache(db) else { return map };
    let Ok(mut stmt) = conn.prepare("SELECT path, mtime, size, info FROM duplicate_cache") else {
        return map;
    };
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))
    });
    if let Ok(rows) = rows {
        for (path, mtime, size, json) in rows.flatten() {
            if let Ok(info) = serde_json::from_str::<Inspected>(&json) {
                map.insert(path, (mtime, size, info));
            }
        }
    }
    map
}

fn store_cache(db: &Path, rows: &[(String, i64, i64, Inspected)]) {
    if rows.is_empty() {
        return;
    }
    let Some(mut conn) = open_cache(db) else { return };
    let Ok(tx) = conn.transaction() else { return };
    for (path, mtime, size, info) in rows {
        if let Ok(json) = serde_json::to_string(info) {
            let _ = tx.execute(
                "INSERT OR REPLACE INTO duplicate_cache (path, mtime, size, info) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![path, mtime, size, json],
            );
        }
    }
    let _ = tx.commit();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_names() {
        assert_eq!(norm("<color=#FFAA00>The Who</color>"), "who");
        assert_eq!(norm("Don't Stop Me Now"), norm("Dont Stop Me Now"));
        assert_eq!(norm("  AC/DC  "), "ac dc");
        assert_eq!(norm("Beyoncé"), "beyoncé");
    }

    #[test]
    fn counts_pans() {
        let dta = "(song (name \"x\") (tracks ((drum (0 1)))) (pans (-1.0 1.0 0.0)) (vols (0 0 0)))";
        assert_eq!(count_pans(dta), Some(3));
        assert_eq!(count_pans("(song (name \"x\"))"), None);
    }

    #[test]
    fn identical_vs_versions() {
        let mk = |p: &str| DupInput {
            path: p.into(),
            artist: "A".into(),
            song_title: "T".into(),
            display_name: "A - T".into(),
            description: String::new(),
        };
        let info = |h: &str| Inspected { chart_hash: Some(h.into()), ..Default::default() };
        let mut map = HashMap::new();
        map.insert("x1".to_string(), info("aa"));
        map.insert("x2".to_string(), info("aa"));
        map.insert("x3".to_string(), info("bb"));
        let g = build_group("k".into(), vec![mk("x1"), mk("x2")], &map);
        assert_eq!(g.kind, "identical");
        let g = build_group("k".into(), vec![mk("x1"), mk("x3"), mk("x2")], &map);
        assert_eq!(g.kind, "versions");
        assert_eq!(g.entries.iter().filter(|e| e.match_id == Some(0)).count(), 2);
    }
}
