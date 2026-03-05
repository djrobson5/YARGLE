use base64::Engine;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::album_art;
use crate::album_art::ArtResult;
use crate::dta::parser::{extract_metadata, parse_dta};
use crate::dta::serializer::{apply_metadata, serialize_dta};
use crate::dta::types::{SongDetails, SongMetadata, SongSummary};
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

#[tauri::command]
pub fn open_folder(path: String) -> Result<Vec<SongSummary>, String> {
    use rayon::prelude::*;

    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err("Not a valid directory".into());
    }

    // Collect candidate paths first
    let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;
    let paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && has_stfs_magic(p))
        .collect();

    // Parse headers in parallel, reading only ~6KB per file
    let mut songs: Vec<SongSummary> = paths
        .par_iter()
        .filter_map(|file_path| {
            let data = read_header_bytes(file_path).ok()?;
            let header = parse_header_summary(&data).ok()?;
            Some(SongSummary {
                path: file_path.to_string_lossy().to_string(),
                display_name: header.display_name,
                description: header.description,
                title_name: header.title_name,
                has_thumbnail: header.thumbnail_size > 0,
            })
        })
        .collect();

    songs.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));

    Ok(songs)
}

#[tauri::command]
pub fn get_song_details(path: String) -> Result<SongDetails, String> {
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
    let fs = StfsFilesystem::parse(data)?;
    let (dta_content, dta_entry) = fs.extract_songs_dta()?;

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

    Ok(SongDetails {
        path,
        display_name: header.display_name,
        description: header.description,
        title_name: header.title_name,
        thumbnail_base64,
        metadata,
        raw_dta: raw_dta.clone(),
        dta_file_size: dta_entry.file_size,
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
        let fs = StfsFilesystem::parse(data.clone())?;
        let (dta_content, _dta_entry) = fs.extract_songs_dta()?;
        let (_, block_offsets) = fs.get_songs_dta_location()?;

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
