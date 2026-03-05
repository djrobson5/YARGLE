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
use crate::stfs::filesystem::StfsFilesystem;
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

/// Check if a directory is an unpacked song folder (contains song.ini)
fn is_song_folder(dir: &Path) -> bool {
    dir.join("song.ini").is_file()
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

fn collect_song_entries(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_dir() {
            if is_song_folder(&p) {
                // This directory is a song entry — add it, don't recurse into it
                out.push(p);
            } else {
                // Regular directory — recurse
                collect_song_entries(&p, out);
            }
        } else if p.is_file() && has_stfs_magic(&p) {
            out.push(p);
        }
    }
}

#[tauri::command]
pub fn open_folder(path: String) -> Result<Vec<SongSummary>, String> {
    use rayon::prelude::*;

    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err("Not a valid directory".into());
    }

    // Collect candidate paths recursively (both CON files and song folders)
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_song_entries(dir, &mut paths);

    // Parse headers in parallel
    let mut songs: Vec<SongSummary> = paths
        .par_iter()
        .filter_map(|file_path| {
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
                Some(SongSummary {
                    path: file_path.to_string_lossy().to_string(),
                    display_name,
                    description,
                    title_name: meta.name,
                    has_thumbnail: find_folder_album_art(file_path).is_some(),
                    is_folder: true,
                })
            } else {
                // CON/STFS file
                let data = read_header_bytes(file_path).ok()?;
                let header = parse_header_summary(&data).ok()?;
                Some(SongSummary {
                    path: file_path.to_string_lossy().to_string(),
                    display_name: header.display_name,
                    description: header.description,
                    title_name: header.title_name,
                    has_thumbnail: header.thumbnail_size > 0,
                    is_folder: false,
                })
            }
        })
        .collect();

    songs.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));

    Ok(songs)
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
        });
    }

    // CON/STFS file
    let data = fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;
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
    let mut data = fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;

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

    let data = fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;
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

    let data = fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;
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
}

fn instrument_name(id: i64) -> &'static str {
    match id {
        0 => "Guitar",
        1 => "Bass",
        2 => "Drums",
        3 => "Vocals",
        4 => "Keys",
        5 => "Real Guitar",
        6 => "Real Bass",
        7 => "Real Drums",
        8 => "Real Keys",
        9 => "Harmony",
        _ => "Unknown",
    }
}

fn difficulty_name(id: i64) -> &'static str {
    match id {
        0 => "Easy",
        1 => "Medium",
        2 => "Hard",
        3 => "Expert",
        4 => "Expert+",
        _ => "Unknown",
    }
}

/// Find the most recently modified scores.db between stable and nightly
fn most_recent_scores_db() -> Option<PathBuf> {
    let stable = yarg_scores_path("release").filter(|p| p.exists());
    let nightly = yarg_scores_path("nightly").filter(|p| p.exists());

    match (stable, nightly) {
        (Some(s), Some(n)) => {
            let s_mod = fs::metadata(&s).and_then(|m| m.modified()).ok();
            let n_mod = fs::metadata(&n).and_then(|m| m.modified()).ok();
            match (s_mod, n_mod) {
                (Some(st), Some(nt)) if nt > st => Some(n),
                _ => Some(s),
            }
        }
        (Some(s), None) => Some(s),
        (None, Some(n)) => Some(n),
        (None, None) => None,
    }
}

#[tauri::command]
pub fn get_song_scores(song_name: String) -> Result<Vec<SongScore>, String> {
    let db_path = match most_recent_scores_db() {
        Some(p) => p,
        None => return Ok(vec![]),
    };

    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| format!("SQLite error: {}", e))?;

    let mut stmt = conn
        .prepare(
            "SELECT g.Date, p2.Name, ps.Instrument, ps.Difficulty,
                    ps.Score, ps.Stars, ps.Percent, ps.IsFc,
                    ps.NotesHit, ps.NotesMissed, g.BandScore, g.BandStars, g.SongSpeed
             FROM GameRecords g
             JOIN PlayerScores ps ON ps.GameRecordId = g.Id
             LEFT JOIN Players p2 ON ps.PlayerId = p2.Id
             WHERE g.SongName = ?1
             ORDER BY ps.Score DESC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([&song_name], |row| {
            let date_ticks: i64 = row.get(0)?;
            // .NET ticks -> Unix seconds: ticks are 100ns intervals since 0001-01-01
            let unix_secs = (date_ticks - 621355968000000000) / 10_000_000;
            let date_str = chrono::DateTime::from_timestamp(unix_secs, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default();

            Ok(SongScore {
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
            })
        })
        .map_err(|e| e.to_string())?;

    let scores: Vec<SongScore> = rows.filter_map(|r| r.ok()).collect();
    Ok(scores)
}

// --- Duplicate Detection ---

#[derive(Serialize, Clone)]
pub struct DuplicateEntry {
    pub path: String,
    pub display_name: String,
    pub description: String,
    pub file_size: u64,
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

    let mut name_groups: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    for (i, path) in paths.iter().enumerate() {
        let p = Path::new(path);
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
                ));
            }
        } else if let Ok(data) = read_header_bytes(p) {
            if let Ok(header) = parse_header_summary(&data) {
                let key = normalize_name(&header.display_name);
                name_groups.entry(key).or_default().push((
                    path.clone(),
                    header.display_name,
                    header.description,
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

    // Build path -> shortname map
    let mut shortname_map: HashMap<String, String> = HashMap::new();
    for (i, (path, _, _)) in candidates_flat.iter().enumerate() {
        let p = Path::new(path.as_str());
        if p.is_dir() {
            // For song folders, use the song name as shortname
            if let Ok(content) = fs::read_to_string(p.join("song.ini")) {
                let meta = song_ini::parse_song_ini(&content);
                if !meta.name.is_empty() {
                    shortname_map.insert(path.clone(), meta.name.to_lowercase().replace(' ', ""));
                }
            }
        } else if let Ok(data) = fs::read(path) {
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
    let mut final_groups: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    for (_, entries) in &candidate_groups {
        for (path, display_name, description) in entries {
            let key = shortname_map
                .get(path)
                .cloned()
                .unwrap_or_else(|| normalize_name(display_name));
            final_groups
                .entry(key)
                .or_default()
                .push((path.clone(), display_name.clone(), description.clone()));
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
                .map(|(path, display_name, description)| {
                    let file_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    DuplicateEntry {
                        path,
                        display_name,
                        description,
                        file_size,
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
        if let Err(e) = fs::remove_file(path) {
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

        match decrypt_mogg_in_con(path) {
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

fn sanitize_filename(s: &str) -> String {
    s.chars()
        .filter(|c| !r#"<>:"/\|?*"#.contains(*c))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
        let data = fs::read(path).ok()?;
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
        let data = fs::read(path).ok()?;
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

        match fs::rename(&req.old_path, &target) {
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
        } else if let Ok(data) = fs::read(path) {
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

fn decrypt_mogg_in_con(path: &str) -> Result<DecryptStatus, String> {
    let mut data = fs::read(path).map_err(|e| format!("Failed to read: {}", e))?;
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

                // Compute the base folder: walk up from file to find the opened folder
                // We use the parent directory of the file as the base
                let base = p.parent().unwrap_or(Path::new("."));
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
pub fn execute_organize(requests: Vec<OrganizeRequest>) -> Result<Vec<RenameResult>, String> {
    let mut results = Vec::new();

    for req in requests {
        let target = PathBuf::from(&req.new_path);

        // Create target directory
        if let Some(parent) = target.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
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

        match fs::rename(&req.old_path, &final_target) {
            Ok(_) => results.push(RenameResult {
                old_path: req.old_path,
                new_path: final_target.to_string_lossy().to_string(),
                success: true,
                error: String::new(),
            }),
            Err(e) => results.push(RenameResult {
                old_path: req.old_path,
                new_path: final_target.to_string_lossy().to_string(),
                success: false,
                error: e.to_string(),
            }),
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
            match fs::read(path_str) {
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
        let midi_bytes = fs::read(&mid_path)
            .map_err(|e| format!("Failed to read .mid file: {}", e))?;
        return midi_parser::parse_chart_overview(&midi_bytes);
    }

    // CON/STFS file
    let data = fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;
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
        let midi_bytes = fs::read(&mid_path)
            .map_err(|e| format!("Failed to read .mid file: {}", e))?;
        return midi_parser::parse_instrument_notes(&midi_bytes, &instrument, &difficulty);
    }

    let data = fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;
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
