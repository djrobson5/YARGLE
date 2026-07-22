use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

use crate::album_art;
use crate::album_art::ArtResult;
use crate::dta::parser::{extract_metadata, parse_dta};
use crate::dta::serializer::{apply_metadata, serialize_dta};
use crate::dta::types::{SongDetails, SongMetadata, SongSummary, ValidationIssue};
use crate::dta::validator::validate_metadata;
use crate::midi::parser as midi_parser;
use crate::midi::types::{ChartOverview, InstrumentNotes};
use crate::song_ini;
use crate::stfs::filesystem::{self, StfsFilesystem};
use crate::stfs::header::{parse_header, parse_header_summary};
use crate::stfs::texture;
use crate::stfs::writer;

/// Efficiently check magic bytes without reading entire file
fn has_stfs_magic(path: &Path) -> bool {
    use std::io::Read;
    if let Ok(mut f) = fs::File::open(path) {
        let mut magic = [0u8; 4];
        if f.read_exact(&mut magic).is_ok() {
            return &magic == b"CON " || &magic == b"LIVE" || &magic == b"PIRS";
        }
    }
    false
}

/// Read only the first 0x171A bytes needed for header summary
fn read_header_bytes(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut buf = vec![0u8; 0x171A];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Find the album art image in an unpacked song folder
fn find_folder_album_art(dir: &Path) -> Option<PathBuf> {
    for name in &["album.png", "album.jpg", "album.jpeg"] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Read album art from a song folder as a data URL
fn read_folder_thumbnail(dir: &Path) -> String {
    if let Some(art_path) = find_folder_album_art(dir) {
        if let Ok(data) = fs::read(&art_path) {
            let mime = if art_path.extension().and_then(|e| e.to_str()) == Some("png") {
                "image/png"
            } else {
                "image/jpeg"
            };
            return format!(
                "data:{};base64,{}",
                mime,
                base64::engine::general_purpose::STANDARD.encode(&data)
            );
        }
    }
    String::new()
}

/// A candidate entry found during enumeration, with the change-detection
/// signature the scan cache keys on. No file contents are read here — only
/// directory listings and stats — so enumeration stays fast on slow drives.
struct ScanEntry {
    path: PathBuf,
    mtime: i64,
    size: i64,
    is_dir: bool,
}

fn mtime_secs(md: &fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Walk one directory with a single `read_dir` and no extra stat calls: on
/// Windows the listing itself carries each entry's metadata, so files cost
/// nothing beyond the listing, and a song folder is recognized by spotting
/// `song.ini` among its own entries (instead of a separate path lookup).
/// Subdirectories are walked in parallel — on external drives enumeration is
/// bound by per-metadata-op latency, which concurrency hides well.
fn walk_song_entries(dir: PathBuf, dir_mtime: i64) -> Vec<ScanEntry> {
    use rayon::prelude::*;

    let rd = match fs::read_dir(&dir) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut files: Vec<ScanEntry> = Vec::new();
    let mut subdirs: Vec<(PathBuf, i64)> = Vec::new();
    let mut song_ini: Option<fs::Metadata> = None;
    for entry in rd.filter_map(|e| e.ok()) {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            let dmt = entry.metadata().map(|m| mtime_secs(&m)).unwrap_or(0);
            subdirs.push((entry.path(), dmt));
        } else if ft.is_file() {
            if entry.file_name().eq_ignore_ascii_case("song.ini") {
                if let Ok(md) = entry.metadata() {
                    song_ini = Some(md);
                }
            }
            // Defer the STFS magic check to the parallel parse phase (and the
            // cache remembers non-song files), so enumeration never opens files.
            if let Ok(md) = entry.metadata() {
                files.push(ScanEntry {
                    mtime: mtime_secs(&md),
                    size: md.len() as i64,
                    path: entry.path(),
                    is_dir: false,
                });
            }
        }
    }

    if let Some(ini_md) = song_ini {
        // This directory is itself a song entry. Signature: song.ini mtime/size,
        // plus the dir's own mtime so adding/removing album art invalidates.
        return vec![ScanEntry {
            mtime: mtime_secs(&ini_md).max(dir_mtime),
            size: ini_md.len() as i64,
            path: dir,
            is_dir: true,
        }];
    }

    let mut out = files;
    let nested: Vec<Vec<ScanEntry>> = subdirs
        .into_par_iter()
        .map(|(p, m)| walk_song_entries(p, m))
        .collect();
    for v in nested {
        out.extend(v);
    }
    out
}

/// Parse a single song entry (CON file or song folder) into a SongSummary.
fn parse_song_entry(file_path: &Path) -> Option<SongSummary> {
    if file_path.is_dir() {
        // Unpacked song folder
        let ini_path = file_path.join("song.ini");
        let content = fs::read_to_string(&ini_path).ok()?;
        let meta = song_ini::parse_song_ini(&content);
        let loading_phrase = song_ini::extract_loading_phrase(&content);
        let display_name = if !meta.name.is_empty() && !meta.artist.is_empty() {
            format!("{} - {}", meta.artist, meta.name)
        } else if !meta.name.is_empty() {
            meta.name.clone()
        } else {
            file_path.file_name()?.to_string_lossy().to_string()
        };
        let description = if !loading_phrase.is_empty() {
            loading_phrase
        } else {
            file_path.file_name()?.to_string_lossy().to_string()
        };
        let game_origin = meta.game_origin.clone();
        Some(SongSummary {
            path: file_path.to_string_lossy().to_string(),
            display_name,
            description,
            title_name: meta.name,
            has_thumbnail: find_folder_album_art(file_path).is_some(),
            is_folder: true,
            album_name: meta.album_name,
            author: meta.author,
            game_origin,
        })
    } else {
        // CON/STFS file — seek-based extraction (reads ~20-50KB, not the full file)
        let data = read_header_bytes(file_path).ok()?;
        let header = parse_header_summary(&data).ok()?;
        // Extract DTA metadata using seek-based I/O
        let (album_name, author, game_origin) =
            filesystem::extract_dta_from_file(file_path)
                .ok()
                .and_then(|dta_content| {
                    let raw_dta = match String::from_utf8(dta_content) {
                        Ok(s) => s,
                        Err(e) => {
                            let bytes = e.into_bytes();
                            let (decoded, _, _) =
                                encoding_rs::WINDOWS_1252.decode(&bytes);
                            decoded.to_string()
                        }
                    };
                    let nodes = parse_dta(&raw_dta).ok()?;
                    let meta = extract_metadata(&nodes, &raw_dta);
                    Some((meta.album_name, meta.author, meta.game_origin))
                })
                .unwrap_or_default();
        Some(SongSummary {
            path: file_path.to_string_lossy().to_string(),
            display_name: header.display_name,
            description: header.description,
            title_name: header.title_name,
            has_thumbnail: header.thumbnail_size > 0,
            is_folder: false,
            album_name,
            author,
            game_origin,
        })
    }
}

#[tauri::command]
pub async fn open_folder(app: AppHandle, path: String) -> Result<Vec<SongSummary>, String> {
    use rayon::prelude::*;
    use std::collections::HashSet;

    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err("Not a valid directory".into());
    }

    let _ = app.emit("open-folder-progress", serde_json::json!({
        "current": 0, "total": 0, "phase": "scanning"
    }));

    // Enumerate candidates with their change signatures (stats only, no reads).
    // A dedicated wider pool: enumeration is small metadata ops where extra
    // concurrency hides drive latency, unlike the bulk reads in the parse phase.
    let enum_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .map_err(|e| e.to_string())?;
    let root_mtime = fs::metadata(dir).map(|m| mtime_secs(&m)).unwrap_or(0);
    let entries: Vec<ScanEntry> =
        enum_pool.install(|| walk_song_entries(dir.to_path_buf(), root_mtime));

    // Serve unchanged entries straight from the scan cache — no file opens.
    let cache = crate::scan_cache::load(&app, &path);
    let mut all_songs: Vec<SongSummary> = Vec::with_capacity(entries.len());
    let mut to_parse: Vec<ScanEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::with_capacity(entries.len());
    for entry in entries {
        let key = entry.path.to_string_lossy().to_string();
        seen.insert(key.clone());
        match cache.get(&key) {
            Some(row) if row.mtime == entry.mtime && row.size == entry.size => {
                if let Some(summary) = &row.summary {
                    all_songs.push(summary.clone());
                }
                // Cached non-song file — skip without re-checking magic.
            }
            _ => to_parse.push(entry),
        }
    }

    let total = to_parse.len();
    let _ = app.emit("open-folder-progress", serde_json::json!({
        "current": 0, "total": total, "phase": "scanning"
    }));

    // Process in chunks to allow progress updates and avoid blocking
    let chunk_size = 200;
    let mut new_rows: Vec<(String, i64, i64, Option<SongSummary>)> =
        Vec::with_capacity(total);

    // Use a limited thread pool to avoid disk I/O contention with large libraries
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .map_err(|e| e.to_string())?;

    for (chunk_idx, chunk) in to_parse.chunks(chunk_size).enumerate() {
        let chunk_rows: Vec<(String, i64, i64, Option<SongSummary>)> = pool.install(|| {
            chunk
                .par_iter()
                .map(|entry| {
                    // The magic check runs here, in parallel, rather than
                    // serially during enumeration.
                    let summary = if entry.is_dir || has_stfs_magic(&entry.path) {
                        parse_song_entry(&entry.path)
                    } else {
                        None
                    };
                    (
                        entry.path.to_string_lossy().to_string(),
                        entry.mtime,
                        entry.size,
                        summary,
                    )
                })
                .collect()
        });

        for row in &chunk_rows {
            if let Some(summary) = &row.3 {
                all_songs.push(summary.clone());
            }
        }
        new_rows.extend(chunk_rows);

        let processed = ((chunk_idx + 1) * chunk_size).min(total);
        let _ = app.emit("open-folder-progress", serde_json::json!({
            "current": processed, "total": total, "phase": "loading"
        }));

        // Yield to the event loop so the UI stays responsive
        tokio::task::yield_now().await;
    }

    // Persist new results and drop rows for files that disappeared.
    if let Err(e) = crate::scan_cache::store(&app, &path, &seen, &cache, &new_rows) {
        eprintln!("scan cache write failed: {}", e);
    }

    all_songs.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));

    let _ = app.emit("open-folder-progress", serde_json::json!({
        "current": total, "total": total, "phase": "done"
    }));

    Ok(all_songs)
}

#[tauri::command]
pub fn get_song_details(path: String) -> Result<SongDetails, String> {
    let p = Path::new(&path);

    // Unpacked song folder
    if p.is_dir() {
        let ini_path = p.join("song.ini");
        let content = fs::read_to_string(&ini_path)
            .map_err(|e| format!("Failed to read song.ini: {}", e))?;
        let metadata = song_ini::parse_song_ini(&content);
        let loading_phrase = song_ini::extract_loading_phrase(&content);
        let thumbnail_base64 = read_folder_thumbnail(p);

        let display_name = if !metadata.name.is_empty() && !metadata.artist.is_empty() {
            format!("{} - {}", metadata.artist, metadata.name)
        } else if !metadata.name.is_empty() {
            metadata.name.clone()
        } else {
            p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        };

        let ini_size = fs::metadata(&ini_path).map(|m| m.len() as u32).unwrap_or(0);
        let has_thumb = !thumbnail_base64.is_empty();
        let validation_issues = validate_metadata(&metadata, has_thumb);

        return Ok(SongDetails {
            path,
            display_name,
            description: loading_phrase,
            title_name: metadata.name.clone(),
            thumbnail_base64,
            metadata,
            raw_dta: content,
            dta_file_size: ini_size,
            validation_issues,
            is_folder: true,
        });
    }

    // CON/STFS file
    let data = read_file(&path).map_err(|e| format!("Failed to read file: {}", e))?;
    let header = parse_header(&data)?;

    // Encode thumbnail as base64
    let thumbnail_base64 = if header.thumbnail_size > 0 {
        let mime = if header.thumbnail_data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            "image/png"
        } else {
            "image/jpeg"
        };
        format!(
            "data:{};base64,{}",
            mime,
            base64::engine::general_purpose::STANDARD.encode(&header.thumbnail_data)
        )
    } else {
        String::new()
    };

    // Parse STFS filesystem and extract songs.dta
    let stfs_fs = StfsFilesystem::parse(data)?;
    let (dta_content, dta_entry) = stfs_fs.extract_songs_dta()?;

    // Decode DTA content (Latin-1 / UTF-8)
    let raw_dta = if dta_content.starts_with(&[0xFF, 0xFE]) || dta_content.starts_with(&[0xFE, 0xFF]) {
        // UTF-16
        let (decoded, _, _) = encoding_rs::UTF_16LE.decode(&dta_content);
        decoded.to_string()
    } else {
        // Try UTF-8 first, fall back to Latin-1
        match String::from_utf8(dta_content.clone()) {
            Ok(s) => s,
            Err(_) => {
                let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&dta_content);
                decoded.to_string()
            }
        }
    };

    let nodes = parse_dta(&raw_dta)?;
    let metadata = extract_metadata(&nodes, &raw_dta);
    let has_thumb = header.thumbnail_size > 0;
    let validation_issues = validate_metadata(&metadata, has_thumb);

    Ok(SongDetails {
        path,
        display_name: header.display_name,
        description: header.description,
        title_name: header.title_name,
        thumbnail_base64,
        metadata,
        raw_dta: raw_dta.clone(),
        dta_file_size: dta_entry.file_size,
        validation_issues,
        is_folder: false,
    })
}

#[tauri::command]
pub fn save_song(
    path: String,
    display_name: Option<String>,
    description: Option<String>,
    metadata: Option<SongMetadata>,
    thumbnail_base64: Option<String>,
) -> Result<(), String> {
    let p = Path::new(&path);

    // Unpacked song folder
    if p.is_dir() {
        let ini_path = p.join("song.ini");
        let original = fs::read_to_string(&ini_path).unwrap_or_default();

        if let Some(meta) = &metadata {
            let new_content = song_ini::serialize_song_ini(
                meta,
                &original,
                display_name.as_deref(),
                description.as_deref(),
            );
            fs::write(&ini_path, &new_content)
                .map_err(|e| format!("Failed to write song.ini: {}", e))?;
        }

        // Save thumbnail
        if let Some(thumb_b64) = &thumbnail_base64 {
            let b64_data = if let Some(idx) = thumb_b64.find(",") {
                &thumb_b64[idx + 1..]
            } else {
                thumb_b64
            };
            let image_data = base64::engine::general_purpose::STANDARD
                .decode(b64_data)
                .map_err(|e| format!("Invalid base64: {}", e))?;
            let art_path = p.join("album.png");
            fs::write(&art_path, &image_data)
                .map_err(|e| format!("Failed to write album art: {}", e))?;
        }

        return Ok(());
    }

    // CON/STFS file
    let mut data = read_file(&path).map_err(|e| format!("Failed to read file: {}", e))?;

    // Update header fields
    if let Some(name) = &display_name {
        writer::write_display_name(&mut data, name);
    }
    if let Some(desc) = &description {
        writer::write_description(&mut data, desc);
    }

    // Update thumbnail
    if let Some(thumb_b64) = &thumbnail_base64 {
        // Strip data URL prefix if present
        let b64_data = if let Some(idx) = thumb_b64.find(",") {
            &thumb_b64[idx + 1..]
        } else {
            thumb_b64
        };

        let image_data = base64::engine::general_purpose::STANDARD
            .decode(b64_data)
            .map_err(|e| format!("Invalid base64: {}", e))?;

        // Resize if image is too large for the 16KB header thumbnail slot
        let final_data = if image_data.len() > 0x4000 {
            writer::resize_thumbnail(&image_data)?
        } else {
            match image::load_from_memory(&image_data) {
                Ok(img) => {
                    if img.width() > 256 || img.height() > 256 {
                        writer::resize_thumbnail(&image_data)?
                    } else {
                        image_data
                    }
                }
                Err(_) => image_data,
            }
        };

        writer::write_thumbnail(&mut data, &final_data)?;
    }

    // Update DTA metadata
    if let Some(meta) = &metadata {
        let stfs_fs = StfsFilesystem::parse(data.clone())?;
        let (dta_content, _dta_entry) = stfs_fs.extract_songs_dta()?;
        let (_, block_offsets) = stfs_fs.get_songs_dta_location()?;

        // Decode current DTA
        let raw_dta = match String::from_utf8(dta_content.clone()) {
            Ok(s) => s,
            Err(_) => {
                let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&dta_content);
                decoded.to_string()
            }
        };

        // Parse, modify, serialize
        let mut nodes = parse_dta(&raw_dta)?;
        apply_metadata(&mut nodes, meta);
        let new_dta = serialize_dta(&nodes);
        let new_dta_bytes = new_dta.as_bytes();

        writer::write_dta_content(
            &mut data,
            new_dta_bytes,
            &block_offsets,
            _dta_entry.file_size,
            None, // TODO: pass file entry offset for size update
        )?;
    }

    writer::save_to_file(&path, &data)?;
    Ok(())
}

#[tauri::command]
pub fn get_thumbnail(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    if p.is_dir() {
        return Ok(read_folder_thumbnail(p));
    }

    let data = read_file(&path).map_err(|e| format!("Failed to read file: {}", e))?;
    let header = parse_header(&data)?;

    if header.thumbnail_size == 0 {
        return Ok(String::new());
    }

    let mime = if header.thumbnail_data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else {
        "image/jpeg"
    };

    Ok(format!(
        "data:{};base64,{}",
        mime,
        base64::engine::general_purpose::STANDARD.encode(&header.thumbnail_data)
    ))
}

#[tauri::command]
pub fn get_album_art(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    if p.is_dir() {
        // For unpacked folders, album art is already returned via thumbnail
        return Ok(read_folder_thumbnail(p));
    }

    let data = read_file(&path).map_err(|e| format!("Failed to read file: {}", e))?;
    let stfs = StfsFilesystem::parse(data)?;

    let xbox_tex = stfs.extract_album_art()?;
    let decoded = texture::decode_png_xbox(&xbox_tex)?;
    let png_bytes = texture::texture_to_png(&decoded)?;

    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&png_bytes)
    ))
}

#[tauri::command]
pub async fn search_album_art(artist: String, album: String) -> Result<Vec<ArtResult>, String> {
    album_art::search_album_art(&artist, &album).await
}

#[tauri::command]
pub async fn download_album_art(url: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("YARGLE/1.0 (album art lookup)")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;
    album_art::download_art(&client, &url).await
}

// --- Score Sync ---

#[derive(Serialize)]
pub struct ScoreFileInfo {
    pub exists: bool,
    pub path: String,
    pub size: u64,
    pub last_modified: String,
}

#[derive(Serialize)]
pub struct ScoreInfo {
    pub stable: ScoreFileInfo,
    pub nightly: ScoreFileInfo,
}

fn yarg_scores_path(variant: &str) -> Option<PathBuf> {
    let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
    let base = PathBuf::from(local_app_data)
        .parent()?
        .join("LocalLow")
        .join("YARC")
        .join("YARG")
        .join(variant)
        .join("scores")
        .join("scores.db");
    Some(base)
}

fn score_file_info(path: &Path) -> ScoreFileInfo {
    if path.exists() {
        let meta = fs::metadata(path).ok();
        let size = meta.as_ref().map_or(0, |m| m.len());
        let last_modified = meta
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let duration = t
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                let secs = duration.as_secs();
                // Format as ISO-ish timestamp
                let dt = chrono::DateTime::from_timestamp(secs as i64, 0)
                    .unwrap_or_default();
                dt.format("%Y-%m-%d %H:%M:%S").to_string()
            })
            .unwrap_or_default();
        ScoreFileInfo {
            exists: true,
            path: path.to_string_lossy().to_string(),
            size,
            last_modified,
        }
    } else {
        ScoreFileInfo {
            exists: false,
            path: path.to_string_lossy().to_string(),
            size: 0,
            last_modified: String::new(),
        }
    }
}

#[tauri::command]
pub fn get_yarg_score_info() -> Result<ScoreInfo, String> {
    let stable_path = yarg_scores_path("release")
        .ok_or("Could not determine YARG stable scores path")?;
    let nightly_path = yarg_scores_path("nightly")
        .ok_or("Could not determine YARG nightly scores path")?;

    Ok(ScoreInfo {
        stable: score_file_info(&stable_path),
        nightly: score_file_info(&nightly_path),
    })
}

#[tauri::command]
pub fn sync_yarg_scores(direction: String) -> Result<String, String> {
    let stable_path = yarg_scores_path("release")
        .ok_or("Could not determine YARG stable scores path")?;
    let nightly_path = yarg_scores_path("nightly")
        .ok_or("Could not determine YARG nightly scores path")?;

    let (src, dst) = match direction.as_str() {
        "stable_to_nightly" => (&stable_path, &nightly_path),
        "nightly_to_stable" => (&nightly_path, &stable_path),
        _ => return Err(format!("Invalid direction: {}", direction)),
    };

    if !src.exists() {
        return Err(format!("Source file does not exist: {}", src.display()));
    }

    // Ensure destination directory exists
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create destination directory: {}", e))?;
    }

    // Backup destination if it exists
    if dst.exists() {
        let bak = dst.with_extension("db.bak");
        fs::copy(dst, &bak)
            .map_err(|e| format!("Failed to create backup: {}", e))?;
    }

    fs::copy(src, dst)
        .map_err(|e| format!("Failed to copy scores: {}", e))?;

    let label = if direction == "stable_to_nightly" {
        "Stable -> Nightly"
    } else {
        "Nightly -> Stable"
    };
    Ok(format!("Scores synced successfully ({})", label))
}

// --- Song Scores Lookup ---

#[derive(Serialize)]
pub struct SongScore {
    pub date: String,
    pub player_name: String,
    pub instrument: String,
    pub difficulty: String,
    pub score: i64,
    pub stars: i64,
    pub percent: f64,
    pub is_fc: bool,
    pub notes_hit: i64,
    pub notes_missed: i64,
    pub band_score: i64,
    pub band_stars: i64,
    pub speed: f64,
    // Which YARG build this play is from: "Stable", "Nightly", or (after a
    // score sync duplicates it into both) "Stable + Nightly".
    pub build: String,
    // How many distinct plays this best-score row was chosen from (>=1), for a
    // "best of N" hint after collapsing to one row per instrument+difficulty.
    pub attempts: i64,
}

// Values match YARG.Core's `Instrument` enum (YARC-Official/YARG.Core,
// InstrumentEnums.cs) — reserved in gaps of 10, so most IDs are non-contiguous.
fn instrument_name(id: i64) -> &'static str {
    match id {
        0 => "Guitar",
        1 => "Bass",
        2 => "Rhythm",
        3 => "Co-op Guitar",
        4 => "Keys",
        10 => "Guitar (6-fret)",
        11 => "Bass (6-fret)",
        12 => "Rhythm (6-fret)",
        13 => "Co-op Guitar (6-fret)",
        20 => "Drums",
        21 => "Pro Drums",
        22 => "5-Lane Drums",
        23 => "Elite Drums",
        30 => "Pro Guitar (17)",
        31 => "Pro Guitar (22)",
        32 => "Pro Bass (17)",
        33 => "Pro Bass (22)",
        34 => "Pro Keys",
        40 => "Vocals",
        41 => "Harmony",
        255 => "Band",
        _ => "Unknown",
    }
}

// Values match YARG.Core's `Difficulty` enum (starts at Beginner=0). YARGLE
// previously started at Easy=0, mislabeling every tier by one (e.g. Expert=4
// showed as "Expert+").
fn difficulty_name(id: i64) -> &'static str {
    match id {
        0 => "Beginner",
        1 => "Easy",
        2 => "Medium",
        3 => "Hard",
        4 => "Expert",
        5 => "Expert+",
        _ => "Unknown",
    }
}

/// Normalize a title/artist for tolerant matching: lowercase, keep only
/// alphanumerics. Absorbs case, whitespace, and punctuation differences (the
/// usual reason a chart's title in YARG's DB doesn't byte-match YARGLE's).
fn norm_title(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Read all plays from one scores.db whose normalized title matches, tagging
/// each with the build label. Returns `(score, normalized_artist)` so the caller
/// can prefer artist matches without letting a differing artist (accents,
/// `&`/`and`) drop a real score. Returns empty on any DB/query error.
fn read_scores(db_path: &Path, build: &str, title_norm: &str) -> Vec<(SongScore, String)> {
    let conn = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT g.Date, p2.Name, ps.Instrument, ps.Difficulty,
                ps.Score, ps.Stars, ps.Percent, ps.IsFc,
                ps.NotesHit, ps.NotesMissed, g.BandScore, g.BandStars, g.SongSpeed,
                g.SongName, g.SongArtist
         FROM GameRecords g
         JOIN PlayerScores ps ON ps.GameRecordId = g.Id
         LEFT JOIN Players p2 ON ps.PlayerId = p2.Id
         ORDER BY ps.Score DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = stmt.query_map([], |row| {
        let date_ticks: i64 = row.get(0)?;
        // .NET ticks -> Unix seconds: ticks are 100ns intervals since 0001-01-01
        let unix_secs = (date_ticks - 621355968000000000) / 10_000_000;
        let date_str = chrono::DateTime::from_timestamp(unix_secs, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let score = SongScore {
            date: date_str,
            player_name: row.get::<_, String>(1).unwrap_or_default(),
            instrument: instrument_name(row.get::<_, i64>(2)?).to_string(),
            difficulty: difficulty_name(row.get::<_, i64>(3)?).to_string(),
            score: row.get(4)?,
            stars: row.get(5)?,
            percent: row.get::<_, f64>(6).unwrap_or(0.0),
            is_fc: row.get::<_, i64>(7).unwrap_or(0) != 0,
            notes_hit: row.get::<_, i64>(8).unwrap_or(0),
            notes_missed: row.get::<_, i64>(9).unwrap_or(0),
            band_score: row.get::<_, i64>(10).unwrap_or(0),
            band_stars: row.get::<_, i64>(11).unwrap_or(0),
            speed: row.get::<_, f64>(12).unwrap_or(1.0),
            build: build.to_string(),
            attempts: 0, // set during the collapse below
        };
        let song_name = row.get::<_, String>(13).unwrap_or_default();
        let song_artist = row.get::<_, String>(14).unwrap_or_default();
        Ok((score, song_name, song_artist))
    });
    let rows = match rows {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for (score, song_name, song_artist) in rows.flatten() {
        if norm_title(&song_name) != title_norm {
            continue;
        }
        out.push((score, norm_title(&song_artist)));
    }
    out
}

#[tauri::command]
pub fn get_song_scores(song_name: String, artist: Option<String>) -> Result<Vec<SongScore>, String> {
    let title_norm = norm_title(&song_name);
    if title_norm.is_empty() {
        return Ok(vec![]);
    }
    let artist_norm = norm_title(&artist.unwrap_or_default());

    // Read BOTH builds so a play on either shows up (the user plays on both and
    // wants to see, e.g., a gold star earned on the other build).
    let mut all: Vec<(SongScore, String)> = Vec::new();
    for (variant, label) in [("release", "Stable"), ("nightly", "Nightly")] {
        if let Some(db) = yarg_scores_path(variant).filter(|p| p.exists()) {
            all.extend(read_scores(&db, label, &title_norm));
        }
    }

    // Artist is a PREFERENCE, not a gate: if some title matches also match the
    // artist, keep only those (disambiguates distinct same-titled songs, e.g.
    // two "My Way"s); otherwise keep all title matches, so a differing artist
    // spelling (accents, "&"/"and") never drops a real score.
    let use_artist = !artist_norm.is_empty() && all.iter().any(|(_, a)| *a == artist_norm);
    let filtered = all
        .into_iter()
        .filter(|(_, a)| !use_artist || *a == artist_norm)
        .map(|(s, _)| s);

    // Collapse a play present in both DBs (e.g. after a score sync) into one row.
    let mut combined: Vec<SongScore> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for s in filtered {
        let key = format!(
            "{}|{}|{}|{}|{}",
            s.date, s.player_name, s.instrument, s.difficulty, s.score
        );
        if let Some(&i) = index.get(&key) {
            if combined[i].build != s.build && combined[i].build != "Stable + Nightly" {
                combined[i].build = "Stable + Nightly".to_string();
            }
        } else {
            index.insert(key, combined.len());
            combined.push(s);
        }
    }

    // Collapse to the single best (highest-score) play per instrument+difficulty,
    // counting how many plays fed each group for a "best of N" hint. Higher score
    // implies more stars (stars derive from score), so this keeps the gold-star run.
    let mut best_idx: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
    let mut collapsed: Vec<SongScore> = Vec::new();
    for mut s in combined {
        let key = (s.instrument.clone(), s.difficulty.clone());
        if let Some(&i) = best_idx.get(&key) {
            let attempts = collapsed[i].attempts + 1;
            if s.score > collapsed[i].score {
                s.attempts = attempts;
                collapsed[i] = s;
            } else {
                collapsed[i].attempts = attempts;
            }
        } else {
            s.attempts = 1;
            best_idx.insert(key, collapsed.len());
            collapsed.push(s);
        }
    }

    collapsed.sort_by(|a, b| b.score.cmp(&a.score));
    Ok(collapsed)
}

/// Reveal a song in the OS file manager. For an unpacked folder song the folder
/// itself is opened; for a CON/STFS file the containing folder is opened with the
/// file selected. Spawns the manager and returns immediately (Explorer notably
/// exits non-zero even on success, so we never wait on it).
#[tauri::command]
pub fn reveal_in_explorer(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("Path no longer exists: {}", path));
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("explorer");
        if p.is_dir() {
            cmd.arg(&path);
        } else {
            // raw_arg avoids Rust's auto-quoting so `/select,"<path>"` reaches
            // Explorer intact (paths with spaces otherwise break the switch).
            cmd.raw_arg(format!("/select,\"{}\"", path));
        }
        cmd.spawn()
            .map_err(|e| format!("Failed to open Explorer: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        if p.is_dir() {
            cmd.arg(&path);
        } else {
            cmd.arg("-R").arg(&path);
        }
        cmd.spawn()
            .map_err(|e| format!("Failed to open Finder: {}", e))?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // No portable "select"; open the folder (or the file's parent).
        let target = if p.is_dir() {
            path.clone()
        } else {
            p.parent()
                .map(|d| d.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone())
        };
        std::process::Command::new("xdg-open")
            .arg(&target)
            .spawn()
            .map_err(|e| format!("Failed to open file manager: {}", e))?;
    }

    Ok(())
}

// --- Duplicate Detection ---

#[derive(Serialize, Clone)]
pub struct DuplicateEntry {
    pub path: String,
    pub display_name: String,
    pub description: String,
    pub file_size: u64,
    pub has_drums: bool,
    pub has_guitar: bool,
    pub has_bass: bool,
    pub has_vocals: bool,
    pub has_keys: bool,
}

#[derive(Serialize, Clone)]
pub struct DuplicateGroup {
    pub shortname: String,
    pub display_name: String,
    pub entries: Vec<DuplicateEntry>,
}

#[derive(Serialize, Clone)]
struct DuplicateScanProgress {
    current: usize,
    total: usize,
    phase: String,
}

fn normalize_name(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[tauri::command]
pub async fn find_duplicates(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<Vec<DuplicateGroup>, String> {
    use std::collections::HashMap;

    let total = paths.len();

    // Phase 1: Group by normalized display_name
    let _ = app.emit(
        "duplicate-scan-progress",
        DuplicateScanProgress { current: 0, total, phase: "Grouping by name...".into() },
    );

    let mut name_groups: HashMap<String, Vec<(String, String, String, u64)>> = HashMap::new();
    for (i, path) in paths.iter().enumerate() {
        let p = Path::new(path);
        let file_size = fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        if p.is_dir() {
            if let Ok(content) = fs::read_to_string(p.join("song.ini")) {
                let meta = song_ini::parse_song_ini(&content);
                let display_name = if !meta.name.is_empty() && !meta.artist.is_empty() {
                    format!("{} - {}", meta.artist, meta.name)
                } else if !meta.name.is_empty() {
                    meta.name.clone()
                } else {
                    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                };
                let key = normalize_name(&display_name);
                let description = song_ini::extract_loading_phrase(&content);
                name_groups.entry(key).or_default().push((
                    path.clone(),
                    display_name,
                    description,
                    file_size,
                ));
            }
        } else if let Ok(data) = read_header_bytes(p) {
            if let Ok(header) = parse_header_summary(&data) {
                let key = normalize_name(&header.display_name);
                name_groups.entry(key).or_default().push((
                    path.clone(),
                    header.display_name,
                    header.description,
                    file_size,
                ));
            }
        }
        if (i + 1) % 10 == 0 || i + 1 == total {
            let _ = app.emit(
                "duplicate-scan-progress",
                DuplicateScanProgress {
                    current: i + 1,
                    total,
                    phase: "Grouping by name...".into(),
                },
            );
        }
    }

    // Keep only groups with 2+ entries
    let candidate_groups: Vec<_> = name_groups
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .collect();

    if candidate_groups.is_empty() {
        return Ok(vec![]);
    }

    // Phase 2: Verify with DTA shortname
    let candidates_flat: Vec<_> = candidate_groups
        .iter()
        .flat_map(|(_, entries)| entries.iter())
        .collect();
    let verify_total = candidates_flat.len();

    let _ = app.emit(
        "duplicate-scan-progress",
        DuplicateScanProgress {
            current: 0,
            total: verify_total,
            phase: "Verifying with DTA...".into(),
        },
    );

    // Build path -> shortname map and path -> instruments map
    let mut shortname_map: HashMap<String, String> = HashMap::new();
    let mut instruments_map: HashMap<String, (bool, bool, bool, bool, bool)> = HashMap::new();
    for (i, (path, _, _, _)) in candidates_flat.iter().enumerate() {
        let p = Path::new(path.as_str());
        if p.is_dir() {
            // For song folders, use the song name as shortname
            if let Ok(content) = fs::read_to_string(p.join("song.ini")) {
                let meta = song_ini::parse_song_ini(&content);
                if !meta.name.is_empty() {
                    shortname_map.insert(path.clone(), meta.name.to_lowercase().replace(' ', ""));
                }
                instruments_map.insert(path.clone(), (
                    meta.rank_drum.map_or(false, |r| r > 0),
                    meta.rank_guitar.map_or(false, |r| r > 0),
                    meta.rank_bass.map_or(false, |r| r > 0),
                    meta.rank_vocals.map_or(false, |r| r > 0),
                    meta.rank_keys.map_or(false, |r| r > 0),
                ));
            }
        } else if let Ok(data) = read_file(path) {
            if let Ok(stfs) = StfsFilesystem::parse(data) {
                if let Ok((dta_content, _)) = stfs.extract_songs_dta() {
                    let raw_dta = match String::from_utf8(dta_content.clone()) {
                        Ok(s) => s,
                        Err(_) => {
                            let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&dta_content);
                            decoded.to_string()
                        }
                    };
                    if let Ok(nodes) = parse_dta(&raw_dta) {
                        let meta = extract_metadata(&nodes, &raw_dta);
                        if !meta.shortname.is_empty() {
                            shortname_map.insert(path.clone(), meta.shortname);
                        }
                        instruments_map.insert(path.clone(), (
                            meta.rank_drum.map_or(false, |r| r > 0),
                            meta.rank_guitar.map_or(false, |r| r > 0),
                            meta.rank_bass.map_or(false, |r| r > 0),
                            meta.rank_vocals.map_or(false, |r| r > 0),
                            meta.rank_keys.map_or(false, |r| r > 0),
                        ));
                    }
                }
            }
        }
        if (i + 1) % 5 == 0 || i + 1 == verify_total {
            let _ = app.emit(
                "duplicate-scan-progress",
                DuplicateScanProgress {
                    current: i + 1,
                    total: verify_total,
                    phase: "Verifying with DTA...".into(),
                },
            );
        }
    }

    // Re-group by shortname (or fall back to normalized display_name)
    let mut final_groups: HashMap<String, Vec<(String, String, String, u64)>> = HashMap::new();
    for (_, entries) in &candidate_groups {
        for (path, display_name, description, file_size) in entries {
            let key = shortname_map
                .get(path)
                .cloned()
                .unwrap_or_else(|| normalize_name(display_name));
            final_groups
                .entry(key)
                .or_default()
                .push((path.clone(), display_name.clone(), description.clone(), *file_size));
        }
    }

    // Build result, keeping only groups with 2+
    let mut result: Vec<DuplicateGroup> = final_groups
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(shortname, entries)| {
            let display_name = entries[0].1.clone();
            let entries = entries
                .into_iter()
                .map(|(path, display_name, description, file_size)| {
                    let (has_drums, has_guitar, has_bass, has_vocals, has_keys) =
                        instruments_map.get(&path).copied().unwrap_or_default();
                    DuplicateEntry {
                        path,
                        display_name,
                        description,
                        file_size,
                        has_drums,
                        has_guitar,
                        has_bass,
                        has_vocals,
                        has_keys,
                    }
                })
                .collect();
            DuplicateGroup {
                shortname,
                display_name,
                entries,
            }
        })
        .collect();

    result.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));

    Ok(result)
}

#[tauri::command]
pub fn delete_files(paths: Vec<String>) -> Result<Vec<String>, String> {
    let mut failures = Vec::new();
    for path in &paths {
        let long_path = to_win_long_path(path);
        let p = Path::new(&long_path);
        let result = if p.is_dir() {
            fs::remove_dir_all(&long_path)
        } else {
            fs::remove_file(&long_path)
        };
        if let Err(e) = result {
            failures.push(format!("{}: {}", path, e));
        }
    }
    Ok(failures)
}

// --- MOGG Decrypt ---

#[derive(Serialize, Clone)]
pub struct BatchDecryptResult {
    pub total: usize,
    pub decrypted: usize,
    pub already_decrypted: usize,
    pub no_mogg: usize,
    pub errors: Vec<String>,
}

#[derive(Serialize, Clone)]
struct MoggDecryptProgress {
    current: usize,
    total: usize,
    filename: String,
    status: String, // "decrypting", "skipped", "done", "error"
}

#[tauri::command]
pub async fn batch_decrypt_moggs(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<BatchDecryptResult, String> {
    let total = paths.len();
    let mut decrypted = 0usize;
    let mut already_decrypted = 0usize;
    let mut no_mogg = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for (i, path) in paths.iter().enumerate() {
        let filename = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());

        let _ = app.emit(
            "mogg-decrypt-progress",
            MoggDecryptProgress {
                current: i + 1,
                total,
                filename: filename.clone(),
                status: "decrypting".into(),
            },
        );

        let result = if Path::new(path).is_dir() {
            decrypt_mogg_in_folder(path)
        } else {
            decrypt_mogg_in_con(path)
        };
        match result {
            Ok(DecryptStatus::Decrypted) => {
                decrypted += 1;
                let _ = app.emit(
                    "mogg-decrypt-progress",
                    MoggDecryptProgress {
                        current: i + 1,
                        total,
                        filename,
                        status: "done".into(),
                    },
                );
            }
            Ok(DecryptStatus::AlreadyDecrypted) => {
                already_decrypted += 1;
                let _ = app.emit(
                    "mogg-decrypt-progress",
                    MoggDecryptProgress {
                        current: i + 1,
                        total,
                        filename,
                        status: "skipped".into(),
                    },
                );
            }
            Ok(DecryptStatus::NoMogg) => {
                no_mogg += 1;
                let _ = app.emit(
                    "mogg-decrypt-progress",
                    MoggDecryptProgress {
                        current: i + 1,
                        total,
                        filename,
                        status: "skipped".into(),
                    },
                );
            }
            Err(e) => {
                errors.push(format!("{}: {}", filename, e));
                let _ = app.emit(
                    "mogg-decrypt-progress",
                    MoggDecryptProgress {
                        current: i + 1,
                        total,
                        filename,
                        status: "error".into(),
                    },
                );
            }
        }
    }

    Ok(BatchDecryptResult {
        total,
        decrypted,
        already_decrypted,
        no_mogg,
        errors,
    })
}

enum DecryptStatus {
    Decrypted,
    AlreadyDecrypted,
    NoMogg,
}

// --- Batch Rename ---

#[derive(Serialize, Clone)]
pub struct RenamePreview {
    pub path: String,
    pub current_name: String,
    pub new_name: String,
    pub status: String, // "rename", "skip_same", "skip_no_metadata"
}

#[derive(Serialize, Clone)]
struct RenamePreviewProgress {
    current: usize,
    total: usize,
}

/// Convert a path to Windows extended-length format (\\?\) to bypass MAX_PATH limits.
/// On non-Windows or paths that already have the prefix, returns as-is.
fn to_win_long_path(path: &str) -> String {
    if cfg!(windows) && !path.starts_with("\\\\?\\") {
        let normalized = path.replace('/', "\\");
        format!("\\\\?\\{}", normalized)
    } else {
        path.to_string()
    }
}

/// Read a file using extended-length path on Windows (handles USB/long paths).
fn read_file(path: &str) -> std::io::Result<Vec<u8>> {
    fs::read(to_win_long_path(path))
}

fn sanitize_filename(s: &str) -> String {
    // Windows forbids trailing dots and spaces in path components. Win32 normally
    // strips them, but \\?\ extended-length paths preserve them verbatim — which
    // leaves folders Explorer and most APIs can't open.
    s.chars()
        .filter(|c| !r#"<>:"/\|?*"#.contains(*c))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(|c: char| c == '.' || c == ' ')
        .to_string()
}

#[tauri::command]
pub async fn preview_renames(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<Vec<RenamePreview>, String> {
    let total = paths.len();
    let mut previews = Vec::new();

    for (i, path) in paths.iter().enumerate() {
        let p = Path::new(path);
        let current_name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let preview = match extract_artist_name(path) {
            Some((artist, name)) => {
                let new_name = sanitize_filename(&format!("{} - {}_rb3con", artist, name));
                if new_name == current_name {
                    RenamePreview {
                        path: path.clone(),
                        current_name,
                        new_name,
                        status: "skip_same".into(),
                    }
                } else {
                    RenamePreview {
                        path: path.clone(),
                        current_name,
                        new_name,
                        status: "rename".into(),
                    }
                }
            }
            None => RenamePreview {
                path: path.clone(),
                current_name,
                new_name: String::new(),
                status: "skip_no_metadata".into(),
            },
        };
        previews.push(preview);

        if (i + 1) % 5 == 0 || i + 1 == total {
            let _ = app.emit(
                "rename-preview-progress",
                RenamePreviewProgress {
                    current: i + 1,
                    total,
                },
            );
        }
    }

    Ok(previews)
}

fn extract_artist_album(path: &str) -> Option<(String, String, String)> {
    let p = Path::new(path);

    let meta = if p.is_dir() {
        let content = fs::read_to_string(p.join("song.ini")).ok()?;
        song_ini::parse_song_ini(&content)
    } else {
        let data = read_file(path).ok()?;
        let stfs = StfsFilesystem::parse(data).ok()?;
        let (dta_content, _) = stfs.extract_songs_dta().ok()?;
        let raw_dta = match String::from_utf8(dta_content.clone()) {
            Ok(s) => s,
            Err(_) => {
                let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&dta_content);
                decoded.to_string()
            }
        };
        let nodes = parse_dta(&raw_dta).ok()?;
        extract_metadata(&nodes, &raw_dta)
    };

    if meta.artist.is_empty() && meta.name.is_empty() && meta.album_name.is_empty() {
        return None;
    }

    let artist = if meta.artist.is_empty() { "Unknown Artist".to_string() } else { meta.artist };
    let name = if meta.name.is_empty() { "Unknown Song".to_string() } else { meta.name };
    let album = if meta.album_name.is_empty() { "Unknown Album".to_string() } else { meta.album_name };

    Some((artist, name, album))
}

fn extract_artist_name(path: &str) -> Option<(String, String)> {
    let p = Path::new(path);

    let meta = if p.is_dir() {
        let content = fs::read_to_string(p.join("song.ini")).ok()?;
        song_ini::parse_song_ini(&content)
    } else {
        let data = read_file(path).ok()?;
        let stfs = StfsFilesystem::parse(data).ok()?;
        let (dta_content, _) = stfs.extract_songs_dta().ok()?;
        let raw_dta = match String::from_utf8(dta_content.clone()) {
            Ok(s) => s,
            Err(_) => {
                let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&dta_content);
                decoded.to_string()
            }
        };
        let nodes = parse_dta(&raw_dta).ok()?;
        extract_metadata(&nodes, &raw_dta)
    };

    if meta.artist.is_empty() && meta.name.is_empty() {
        return None;
    }

    let artist = if meta.artist.is_empty() { "Unknown Artist".to_string() } else { meta.artist };
    let name = if meta.name.is_empty() { "Unknown Song".to_string() } else { meta.name };

    Some((artist, name))
}

#[derive(Deserialize)]
pub struct RenameRequest {
    pub old_path: String,
    pub new_path: String,
}

#[derive(Serialize)]
pub struct RenameResult {
    pub old_path: String,
    pub new_path: String,
    pub success: bool,
    pub error: String,
}

#[tauri::command]
pub fn batch_rename(renames: Vec<RenameRequest>) -> Result<Vec<RenameResult>, String> {
    let mut results = Vec::new();

    for req in renames {
        let mut target = PathBuf::from(&req.new_path);

        // Handle collision: append (2), (3), etc.
        if target.exists() {
            let parent = target.parent().unwrap_or(Path::new("."));
            let stem = target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut counter = 2;
            loop {
                let candidate = parent.join(format!("{} ({})", stem, counter));
                if !candidate.exists() {
                    target = candidate;
                    break;
                }
                counter += 1;
            }
        }

        let src = to_win_long_path(&req.old_path);
        let dst = to_win_long_path(&target.to_string_lossy());
        match fs::rename(&src, &dst) {
            Ok(_) => results.push(RenameResult {
                old_path: req.old_path,
                new_path: target.to_string_lossy().to_string(),
                success: true,
                error: String::new(),
            }),
            Err(e) => results.push(RenameResult {
                old_path: req.old_path,
                new_path: target.to_string_lossy().to_string(),
                success: false,
                error: e.to_string(),
            }),
        }
    }

    Ok(results)
}

// --- Batch Field Edit ---

#[derive(Serialize, Clone)]
pub struct SongFieldPreview {
    pub path: String,
    pub filename: String,
    pub current_value: String,
    pub metadata: SongMetadata,
}

#[derive(Serialize, Clone)]
struct BatchFieldProgress {
    current: usize,
    total: usize,
}

#[tauri::command]
pub async fn batch_get_field(
    app: AppHandle,
    paths: Vec<String>,
    field: String,
) -> Result<Vec<SongFieldPreview>, String> {
    let total = paths.len();
    let mut results = Vec::new();

    for (i, path) in paths.iter().enumerate() {
        let p = Path::new(path);
        let filename = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());

        let meta_opt: Option<SongMetadata> = if p.is_dir() {
            fs::read_to_string(p.join("song.ini"))
                .ok()
                .map(|content| song_ini::parse_song_ini(&content))
        } else if let Ok(data) = read_file(path) {
            StfsFilesystem::parse(data).ok().and_then(|stfs| {
                let (dta_content, _) = stfs.extract_songs_dta().ok()?;
                let raw_dta = match String::from_utf8(dta_content.clone()) {
                    Ok(s) => s,
                    Err(_) => {
                        let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&dta_content);
                        decoded.to_string()
                    }
                };
                let nodes = parse_dta(&raw_dta).ok()?;
                Some(extract_metadata(&nodes, &raw_dta))
            })
        } else {
            None
        };

        if let Some(meta) = meta_opt {
            let current_value = match field.as_str() {
                "author" => meta.author.clone(),
                "genre" => meta.genre.clone(),
                "sub_genre" => meta.sub_genre.clone(),
                "vocal_gender" => meta.vocal_gender.clone(),
                "game_origin" => meta.game_origin.clone(),
                "rating" => meta.rating.map(|r| r.to_string()).unwrap_or_default(),
                "year_released" => meta.year_released.map(|y| y.to_string()).unwrap_or_default(),
                _ => String::new(),
            };
            results.push(SongFieldPreview {
                path: path.clone(),
                filename,
                current_value,
                metadata: meta,
            });
        }

        if (i + 1) % 5 == 0 || i + 1 == total {
            let _ = app.emit(
                "batch-field-progress",
                BatchFieldProgress {
                    current: i + 1,
                    total,
                },
            );
        }
    }

    Ok(results)
}

fn decrypt_mogg_in_folder(dir: &str) -> Result<DecryptStatus, String> {
    let dir_path = Path::new(dir);
    let mogg_path = fs::read_dir(dir_path)
        .map_err(|e| format!("Failed to read directory: {}", e))?
        .filter_map(|e| e.ok())
        .find(|e| {
            e.path()
                .extension()
                .map(|ext| ext.to_ascii_lowercase() == "mogg")
                .unwrap_or(false)
        })
        .map(|e| e.path());

    let mogg_path = match mogg_path {
        Some(p) => p,
        None => return Ok(DecryptStatus::NoMogg),
    };

    let mogg_str = mogg_path.to_string_lossy();
    let mut mogg_buf = read_file(&mogg_str).map_err(|e| format!("Failed to read .mogg: {}", e))?;

    if mogg_buf.len() < 4 {
        return Err("MOGG file too small".into());
    }

    let version = u32::from_le_bytes([mogg_buf[0], mogg_buf[1], mogg_buf[2], mogg_buf[3]]);

    if version == 0x0A {
        return Ok(DecryptStatus::AlreadyDecrypted);
    }
    if version < 0x0B || version > 0x11 {
        return Err(format!("Unsupported MOGG version: 0x{:02X}", version));
    }

    let mogg_len = mogg_buf.len();
    let success = unsafe { themethod3::capi::decrypt_mogg(mogg_buf.as_mut_ptr(), mogg_len) };
    if !success {
        return Err("themethod3 decryption failed".into());
    }

    let long_path = to_win_long_path(&mogg_str);
    fs::write(&long_path, &mogg_buf).map_err(|e| format!("Failed to write decrypted .mogg: {}", e))?;

    Ok(DecryptStatus::Decrypted)
}

fn decrypt_mogg_in_con(path: &str) -> Result<DecryptStatus, String> {
    let mut data = read_file(path).map_err(|e| format!("Failed to read: {}", e))?;
    let stfs = StfsFilesystem::parse(data.clone())?;

    let mogg_entry = match stfs.find_mogg_file() {
        Some(entry) => entry.clone(),
        None => return Ok(DecryptStatus::NoMogg),
    };

    let block_offsets = stfs.get_file_block_offsets(&mogg_entry)?;
    drop(stfs); // release the parsed clone

    if block_offsets.is_empty() {
        return Err("MOGG file has no blocks".into());
    }

    // Read the MOGG version from the first 4 bytes (LE u32)
    let first_offset = block_offsets[0] as usize;
    if first_offset + 4 > data.len() {
        return Err("MOGG data offset out of bounds".into());
    }
    let version = u32::from_le_bytes([
        data[first_offset],
        data[first_offset + 1],
        data[first_offset + 2],
        data[first_offset + 3],
    ]);

    if version == 0x0A {
        return Ok(DecryptStatus::AlreadyDecrypted);
    }

    if version < 0x0B || version > 0x11 {
        return Err(format!("Unsupported MOGG version: 0x{:02X}", version));
    }

    // Extract full MOGG content
    let stfs2 = StfsFilesystem::parse(data.clone())?;
    let mogg_content = stfs2.extract_file(&mogg_entry)?;
    drop(stfs2);

    let mut mogg_buf = mogg_content;
    let mogg_len = mogg_buf.len();

    let success = unsafe { themethod3::capi::decrypt_mogg(mogg_buf.as_mut_ptr(), mogg_len) };
    if !success {
        return Err("themethod3 decryption failed".into());
    }

    // Write decrypted content back to the CON file at the same block offsets
    writer::write_file_content_inplace(&mut data, &mogg_buf, &block_offsets)?;

    // Save back to disk
    writer::save_to_file(path, &data)?;

    Ok(DecryptStatus::Decrypted)
}

// --- Auto-Organize ---

#[derive(Serialize, Clone)]
pub struct OrganizePreview {
    pub path: String,
    pub filename: String,
    pub artist: String,
    pub album: String,
    pub target_folder: String,
    pub target_path: String,
    pub status: String, // "move", "skip_same", "skip_no_metadata"
}

#[derive(Serialize, Clone)]
struct OrganizePreviewProgress {
    current: usize,
    total: usize,
}

#[tauri::command]
pub async fn preview_organize(
    app: AppHandle,
    paths: Vec<String>,
    base_folder: String,
) -> Result<Vec<OrganizePreview>, String> {
    let total = paths.len();
    let mut previews = Vec::new();

    for (i, path) in paths.iter().enumerate() {
        let p = Path::new(path);
        let filename = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let preview = match extract_artist_album(path) {
            Some((artist, _name, album)) => {
                let safe_artist = sanitize_filename(&artist);
                let safe_album = sanitize_filename(&album);
                let target_folder = format!("{}/{}", safe_artist, safe_album);

                // Use the user's opened folder as the base, not the file's parent
                let base = Path::new(&base_folder);
                let target_dir = base.join(&safe_artist).join(&safe_album);
                let target_path = target_dir.join(&filename);
                let target_path_str = target_path.to_string_lossy().to_string();

                // Check if already in correct location
                let canonical_current = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
                let canonical_target = target_path
                    .canonicalize()
                    .unwrap_or_else(|_| target_path.clone());

                if canonical_current == canonical_target {
                    OrganizePreview {
                        path: path.clone(),
                        filename,
                        artist: safe_artist,
                        album: safe_album,
                        target_folder,
                        target_path: target_path_str,
                        status: "skip_same".into(),
                    }
                } else {
                    OrganizePreview {
                        path: path.clone(),
                        filename,
                        artist: safe_artist,
                        album: safe_album,
                        target_folder,
                        target_path: target_path_str,
                        status: "move".into(),
                    }
                }
            }
            None => OrganizePreview {
                path: path.clone(),
                filename,
                artist: String::new(),
                album: String::new(),
                target_folder: String::new(),
                target_path: String::new(),
                status: "skip_no_metadata".into(),
            },
        };
        previews.push(preview);

        if (i + 1) % 5 == 0 || i + 1 == total {
            let _ = app.emit(
                "organize-preview-progress",
                OrganizePreviewProgress {
                    current: i + 1,
                    total,
                },
            );
        }
    }

    Ok(previews)
}

#[derive(Deserialize)]
pub struct OrganizeRequest {
    pub old_path: String,
    pub new_path: String,
}

#[tauri::command]
pub fn execute_organize(requests: Vec<OrganizeRequest>, base_folder: String) -> Result<Vec<RenameResult>, String> {
    let mut results = Vec::new();
    let base = PathBuf::from(&base_folder);

    for req in requests {
        let target = PathBuf::from(&req.new_path);

        // Create target directory (use extended-length path for USB / long paths)
        if let Some(parent) = target.parent() {
            let parent_long = to_win_long_path(&parent.to_string_lossy());
            if let Err(e) = fs::create_dir_all(&parent_long) {
                results.push(RenameResult {
                    old_path: req.old_path,
                    new_path: target.to_string_lossy().to_string(),
                    success: false,
                    error: format!("Failed to create directory: {}", e),
                });
                continue;
            }
        }

        // Handle collision
        let mut final_target = target.clone();
        if final_target.exists() {
            let parent = final_target.parent().unwrap_or(Path::new("."));
            let stem = final_target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut counter = 2;
            loop {
                let candidate = parent.join(format!("{} ({})", stem, counter));
                if !candidate.exists() {
                    final_target = candidate;
                    break;
                }
                counter += 1;
            }
        }

        // Use extended-length path prefix on Windows for long paths / USB drives
        let src_path = to_win_long_path(&req.old_path);
        let dst_path = to_win_long_path(&final_target.to_string_lossy());

        match fs::rename(&src_path, &dst_path) {
            Ok(_) => results.push(RenameResult {
                old_path: req.old_path,
                new_path: final_target.to_string_lossy().to_string(),
                success: true,
                error: String::new(),
            }),
            Err(rename_err) => {
                // Fallback: copy + delete (handles cross-volume moves)
                match fs::copy(&src_path, &dst_path)
                    .and_then(|_| fs::remove_file(&src_path))
                {
                    Ok(_) => results.push(RenameResult {
                        old_path: req.old_path,
                        new_path: final_target.to_string_lossy().to_string(),
                        success: true,
                        error: String::new(),
                    }),
                    Err(copy_err) => results.push(RenameResult {
                        old_path: req.old_path,
                        new_path: final_target.to_string_lossy().to_string(),
                        success: false,
                        error: format!("rename: {} / copy: {}", rename_err, copy_err),
                    }),
                }
            }
        }
    }

    // Clean up empty directories left behind after moves.
    // Walk up from each source file's parent, removing empty dirs, but never above base_folder.
    let canonical_base = base.canonicalize().unwrap_or_else(|_| base.clone());
    let mut cleaned: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for res in &results {
        if !res.success {
            continue;
        }
        let mut dir = PathBuf::from(&res.old_path);
        dir.pop(); // start at parent of the moved file
        loop {
            let canonical_dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            if canonical_dir <= canonical_base || cleaned.contains(&canonical_dir) {
                break;
            }
            // Only remove if truly empty
            let is_empty = fs::read_dir(&dir)
                .map(|mut rd| rd.next().is_none())
                .unwrap_or(false);
            if is_empty {
                let _ = fs::remove_dir(&dir);
                cleaned.insert(canonical_dir);
                dir.pop();
            } else {
                break;
            }
        }
    }

    Ok(results)
}

// --- Batch Validation ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongValidationResult {
    pub path: String,
    pub display_name: String,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchValidateResult {
    pub total_songs: usize,
    pub songs_with_errors: usize,
    pub songs_with_warnings: usize,
    pub songs_clean: usize,
    pub parse_failures: usize,
    pub results: Vec<SongValidationResult>,
}

#[derive(Debug, Clone, Serialize)]
struct BatchValidateProgress {
    current: usize,
    total: usize,
}

#[tauri::command]
pub fn batch_validate(paths: Vec<String>, app: AppHandle) -> Result<BatchValidateResult, String> {
    let total = paths.len();
    let mut results: Vec<SongValidationResult> = Vec::new();
    let mut parse_failures: usize = 0;

    for (i, path_str) in paths.iter().enumerate() {
        let _ = app.emit("batch-validate-progress", BatchValidateProgress {
            current: i + 1,
            total,
        });

        let p = Path::new(path_str);

        let validation_result: Option<(String, Vec<ValidationIssue>)> = if p.is_dir() {
            let ini_path = p.join("song.ini");
            match fs::read_to_string(&ini_path) {
                Ok(content) => {
                    let meta = song_ini::parse_song_ini(&content);
                    let has_thumb = find_folder_album_art(p).is_some();
                    let display = if !meta.name.is_empty() && !meta.artist.is_empty() {
                        format!("{} - {}", meta.artist, meta.name)
                    } else if !meta.name.is_empty() {
                        meta.name.clone()
                    } else {
                        p.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default()
                    };
                    let issues = validate_metadata(&meta, has_thumb);
                    Some((display, issues))
                }
                Err(_) => None,
            }
        } else {
            match read_file(path_str) {
                Ok(data) => {
                    match parse_header(&data) {
                        Ok(header) => {
                            let display = header.display_name.clone();
                            let has_thumb = header.thumbnail_size > 0;
                            match StfsFilesystem::parse(data) {
                                Ok(stfs_fs) => match stfs_fs.extract_songs_dta() {
                                    Ok((dta_content, _)) => {
                                        let raw_dta = match String::from_utf8(dta_content.clone()) {
                                            Ok(s) => s,
                                            Err(_) => {
                                                let (decoded, _, _) =
                                                    encoding_rs::WINDOWS_1252.decode(&dta_content);
                                                decoded.to_string()
                                            }
                                        };
                                        match parse_dta(&raw_dta) {
                                            Ok(nodes) => {
                                                let meta = extract_metadata(&nodes, &raw_dta);
                                                let issues = validate_metadata(&meta, has_thumb);
                                                Some((display, issues))
                                            }
                                            Err(_) => None,
                                        }
                                    }
                                    Err(_) => None,
                                },
                                Err(_) => None,
                            }
                        }
                        Err(_) => None,
                    }
                }
                Err(_) => None,
            }
        };

        match validation_result {
            Some((display_name, issues)) => {
                if !issues.is_empty() {
                    results.push(SongValidationResult {
                        path: path_str.clone(),
                        display_name,
                        issues,
                    });
                }
            }
            None => {
                parse_failures += 1;
            }
        }
    }

    let songs_with_errors = results
        .iter()
        .filter(|r| {
            r.issues
                .iter()
                .any(|i| i.level == crate::dta::types::ValidationLevel::Error)
        })
        .count();
    let songs_with_warnings = results
        .iter()
        .filter(|r| {
            !r.issues
                .iter()
                .any(|i| i.level == crate::dta::types::ValidationLevel::Error)
                && r.issues
                    .iter()
                    .any(|i| i.level == crate::dta::types::ValidationLevel::Warning)
        })
        .count();
    let songs_clean = total - results.len() - parse_failures;

    Ok(BatchValidateResult {
        total_songs: total,
        songs_with_errors,
        songs_with_warnings,
        songs_clean,
        parse_failures,
        results,
    })
}

#[tauri::command]
pub fn get_chart_overview(path: String) -> Result<ChartOverview, String> {
    let p = Path::new(&path);

    if p.is_dir() {
        // Unpacked song folder — look for .mid directly
        let mid_path = find_mid_in_dir(p)?;
        let midi_bytes = read_file(&mid_path.to_string_lossy())
            .map_err(|e| format!("Failed to read .mid file: {}", e))?;
        return midi_parser::parse_chart_overview(&midi_bytes);
    }

    // CON/STFS file
    let data = read_file(&path).map_err(|e| format!("Failed to read file: {}", e))?;
    let stfs = StfsFilesystem::parse(data)?;
    let mid_entry = stfs
        .find_mid_file()
        .ok_or("No .mid file found in package")?
        .clone();
    let midi_bytes = stfs.extract_file(&mid_entry)?;
    midi_parser::parse_chart_overview(&midi_bytes)
}

#[tauri::command]
pub fn get_chart_notes(
    path: String,
    instrument: String,
    difficulty: String,
) -> Result<InstrumentNotes, String> {
    let p = Path::new(&path);

    if p.is_dir() {
        let mid_path = find_mid_in_dir(p)?;
        let midi_bytes = read_file(&mid_path.to_string_lossy())
            .map_err(|e| format!("Failed to read .mid file: {}", e))?;
        return midi_parser::parse_instrument_notes(&midi_bytes, &instrument, &difficulty);
    }

    let data = read_file(&path).map_err(|e| format!("Failed to read file: {}", e))?;
    let stfs = StfsFilesystem::parse(data)?;
    let mid_entry = stfs
        .find_mid_file()
        .ok_or("No .mid file found in package")?
        .clone();
    let midi_bytes = stfs.extract_file(&mid_entry)?;
    midi_parser::parse_instrument_notes(&midi_bytes, &instrument, &difficulty)
}

/// Find a .mid file in an unpacked song directory
fn find_mid_in_dir(dir: &Path) -> Result<PathBuf, String> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.ends_with(".mid") {
                return Ok(entry.path());
            }
        }
    }
    Err("No .mid file found in song folder".into())
}
