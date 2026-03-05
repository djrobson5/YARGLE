use super::types::{SongMetadata, ValidationIssue, ValidationLevel};

pub fn validate_metadata(meta: &SongMetadata, has_thumbnail: bool) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Errors — critical missing fields
    if meta.name.trim().is_empty() {
        issues.push(ValidationIssue {
            level: ValidationLevel::Error,
            field: "name".into(),
            message: "Song title is empty".into(),
        });
    }
    if meta.artist.trim().is_empty() {
        issues.push(ValidationIssue {
            level: ValidationLevel::Error,
            field: "artist".into(),
            message: "Artist is empty".into(),
        });
    }

    // Warnings — important but not fatal
    if meta.shortname.trim().is_empty() {
        issues.push(ValidationIssue {
            level: ValidationLevel::Warning,
            field: "shortname".into(),
            message: "Shortname is empty".into(),
        });
    }
    if meta.genre.trim().is_empty() {
        issues.push(ValidationIssue {
            level: ValidationLevel::Warning,
            field: "genre".into(),
            message: "Genre is empty".into(),
        });
    }

    let all_ranks = [
        meta.rank_drum,
        meta.rank_guitar,
        meta.rank_bass,
        meta.rank_vocals,
        meta.rank_keys,
        meta.rank_band,
        meta.rank_real_guitar,
        meta.rank_real_bass,
        meta.rank_real_keys,
    ];
    if all_ranks.iter().all(|r| r.is_none() || *r == Some(0)) {
        issues.push(ValidationIssue {
            level: ValidationLevel::Warning,
            field: "ranks".into(),
            message: "All difficulty ranks are missing or zero".into(),
        });
    }

    match meta.song_length {
        None => issues.push(ValidationIssue {
            level: ValidationLevel::Warning,
            field: "song_length".into(),
            message: "Song length is not set".into(),
        }),
        Some(len) if len <= 0 => issues.push(ValidationIssue {
            level: ValidationLevel::Warning,
            field: "song_length".into(),
            message: "Song length is zero or negative".into(),
        }),
        _ => {}
    }

    if meta.author.trim().is_empty() {
        issues.push(ValidationIssue {
            level: ValidationLevel::Warning,
            field: "author".into(),
            message: "Charter/author is empty".into(),
        });
    }

    match meta.year_released {
        None => issues.push(ValidationIssue {
            level: ValidationLevel::Warning,
            field: "year_released".into(),
            message: "Year released is not set".into(),
        }),
        Some(y) if y < 1900 || y > 2030 => issues.push(ValidationIssue {
            level: ValidationLevel::Warning,
            field: "year_released".into(),
            message: format!("Year {} looks invalid (expected 1900–2030)", y),
        }),
        _ => {}
    }

    if meta.album_name.trim().is_empty() {
        issues.push(ValidationIssue {
            level: ValidationLevel::Warning,
            field: "album_name".into(),
            message: "Album name is empty".into(),
        });
    }

    if meta.game_origin.trim().is_empty() || meta.game_origin == "ugc_plus" {
        issues.push(ValidationIssue {
            level: ValidationLevel::Warning,
            field: "game_origin".into(),
            message: if meta.game_origin == "ugc_plus" {
                "Game origin is default 'ugc_plus'".into()
            } else {
                "Game origin is empty".into()
            },
        });
    }

    // Info — nice to have
    if !has_thumbnail {
        issues.push(ValidationIssue {
            level: ValidationLevel::Info,
            field: "thumbnail".into(),
            message: "No album art / thumbnail".into(),
        });
    }

    if meta.preview_start.is_none() {
        issues.push(ValidationIssue {
            level: ValidationLevel::Info,
            field: "preview_start".into(),
            message: "Preview start time is not set".into(),
        });
    }

    if meta.rating.is_none() {
        issues.push(ValidationIssue {
            level: ValidationLevel::Info,
            field: "rating".into(),
            message: "Content rating is not set".into(),
        });
    }

    // Sort: errors first, then warnings, then info
    issues.sort_by(|a, b| a.level.cmp(&b.level));

    issues
}
