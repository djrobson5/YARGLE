use crate::dta::types::SongMetadata;

/// Parse a song.ini file into SongMetadata.
/// song.ini is a simple INI format with `[song]` or `[Song]` section header
/// and `key = value` pairs.
pub fn parse_song_ini(content: &str) -> SongMetadata {
    let mut meta = SongMetadata::default();

    for line in content.lines() {
        let trimmed = line.trim();
        // Skip section headers and empty/comment lines
        if trimmed.is_empty() || trimmed.starts_with('[') || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim().to_lowercase();
            let value = value.trim();

            match key.as_str() {
                "name" => meta.name = value.to_string(),
                "artist" => meta.artist = value.to_string(),
                "album" => meta.album_name = value.to_string(),
                "genre" => meta.genre = value.to_string(),
                "year" => meta.year_released = value.parse().ok(),
                "charter" | "frets" => {
                    if meta.author.is_empty() {
                        meta.author = value.to_string();
                    }
                }
                "song_length" => meta.song_length = value.parse().ok(),
                "preview_start_time" => meta.preview_start = value.parse().ok(),
                "album_track" => meta.album_track_number = value.parse().ok(),
                "loading_phrase" => { /* stored separately as description */ }
                "diff_guitar" => meta.rank_guitar = value.parse().ok(),
                "diff_bass" => meta.rank_bass = value.parse().ok(),
                "diff_drums" => meta.rank_drum = value.parse().ok(),
                "diff_keys" => meta.rank_keys = value.parse().ok(),
                "diff_vocals" => meta.rank_vocals = value.parse().ok(),
                "diff_band" => meta.rank_band = value.parse().ok(),
                "diff_guitar_real" => meta.rank_real_guitar = value.parse().ok(),
                "diff_bass_real" => meta.rank_real_bass = value.parse().ok(),
                "diff_keys_real" => meta.rank_real_keys = value.parse().ok(),
                "icon" => meta.game_origin = value.to_string(),
                _ => {}
            }
        }
    }

    meta
}

/// Extract the loading_phrase from song.ini content (used as description).
pub fn extract_loading_phrase(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some((key, value)) = trimmed.split_once('=') {
            if key.trim().to_lowercase() == "loading_phrase" {
                return value.trim().to_string();
            }
        }
    }
    String::new()
}

/// Round-trip serialize: update known keys in the original content, preserving
/// unknown keys, comments, and ordering. If a key doesn't exist yet but has a
/// value, append it.
pub fn serialize_song_ini(meta: &SongMetadata, original: &str, display_name: Option<&str>, description: Option<&str>) -> String {
    let mut lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();
    let mut updated_keys = std::collections::HashSet::new();

    // Build the key->value map of what we want to write
    let mut desired: Vec<(&str, String)> = vec![
        ("name", meta.name.clone()),
        ("artist", meta.artist.clone()),
        ("album", meta.album_name.clone()),
        ("genre", meta.genre.clone()),
        ("charter", meta.author.clone()),
    ];

    if let Some(y) = meta.year_released {
        desired.push(("year", y.to_string()));
    }
    if let Some(v) = meta.song_length {
        desired.push(("song_length", v.to_string()));
    }
    if let Some(v) = meta.preview_start {
        desired.push(("preview_start_time", v.to_string()));
    }
    if let Some(v) = meta.album_track_number {
        desired.push(("album_track", v.to_string()));
    }
    if let Some(v) = meta.rank_guitar {
        desired.push(("diff_guitar", v.to_string()));
    }
    if let Some(v) = meta.rank_bass {
        desired.push(("diff_bass", v.to_string()));
    }
    if let Some(v) = meta.rank_drum {
        desired.push(("diff_drums", v.to_string()));
    }
    if let Some(v) = meta.rank_keys {
        desired.push(("diff_keys", v.to_string()));
    }
    if let Some(v) = meta.rank_vocals {
        desired.push(("diff_vocals", v.to_string()));
    }
    if let Some(v) = meta.rank_band {
        desired.push(("diff_band", v.to_string()));
    }
    if let Some(v) = meta.rank_real_guitar {
        desired.push(("diff_guitar_real", v.to_string()));
    }
    if let Some(v) = meta.rank_real_bass {
        desired.push(("diff_bass_real", v.to_string()));
    }
    if let Some(v) = meta.rank_real_keys {
        desired.push(("diff_keys_real", v.to_string()));
    }
    if !meta.game_origin.is_empty() {
        desired.push(("icon", meta.game_origin.clone()));
    }

    // Override name/loading_phrase from header fields if provided
    if let Some(dn) = display_name {
        // Find and update the "name" entry in desired
        for (k, v) in desired.iter_mut() {
            if *k == "name" {
                *v = dn.to_string();
                break;
            }
        }
    }
    if let Some(desc) = description {
        desired.push(("loading_phrase", desc.to_string()));
    }

    // Update existing lines in-place
    for line in lines.iter_mut() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('[') || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key_part, _)) = trimmed.split_once('=') {
            let key_lower = key_part.trim().to_lowercase();
            for (desired_key, desired_val) in &desired {
                if key_lower == *desired_key {
                    // Preserve original key casing
                    let original_key = key_part.trim();
                    *line = format!("{} = {}", original_key, desired_val);
                    updated_keys.insert(desired_key.to_string());
                    break;
                }
                // Handle "frets" as alias for "charter"
                if *desired_key == "charter" && key_lower == "frets" {
                    let original_key = key_part.trim();
                    *line = format!("{} = {}", original_key, desired_val);
                    updated_keys.insert(desired_key.to_string());
                    break;
                }
            }
        }
    }

    // Append any keys that weren't already in the file
    for (key, val) in &desired {
        if !updated_keys.contains(*key) && !val.is_empty() {
            lines.push(format!("{} = {}", key, val));
        }
    }

    let mut result = lines.join("\n");
    // Preserve trailing newline if original had one
    if original.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic() {
        let ini = "[song]\nname = Test Song\nartist = Test Artist\nalbum = Test Album\nyear = 2020\ncharter = TestCharter\ndiff_guitar = 4\n";
        let meta = parse_song_ini(ini);
        assert_eq!(meta.name, "Test Song");
        assert_eq!(meta.artist, "Test Artist");
        assert_eq!(meta.album_name, "Test Album");
        assert_eq!(meta.year_released, Some(2020));
        assert_eq!(meta.author, "TestCharter");
        assert_eq!(meta.rank_guitar, Some(4));
    }

    #[test]
    fn test_roundtrip() {
        let ini = "[song]\nname = Old Name\nartist = Old Artist\ncharter = Someone\n";
        let mut meta = parse_song_ini(ini);
        meta.name = "New Name".to_string();
        meta.artist = "New Artist".to_string();
        let result = serialize_song_ini(&meta, ini, None, None);
        assert!(result.contains("name = New Name"));
        assert!(result.contains("artist = New Artist"));
        assert!(result.contains("charter = Someone"));
    }
}
