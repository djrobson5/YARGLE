use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ValidationLevel {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub level: ValidationLevel,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SongMetadata {
    pub shortname: String,
    pub name: String,
    pub artist: String,
    pub album_name: String,
    pub album_track_number: Option<i32>,
    pub genre: String,
    pub sub_genre: String,
    pub vocal_gender: String,
    pub year_released: Option<i32>,
    pub song_length: Option<i32>,
    pub rating: Option<i32>, // 1=Family, 2=Supervision, 3=Mature
    pub song_id: Option<i32>,
    pub game_origin: String,
    pub preview_start: Option<i32>,
    pub preview_end: Option<i32>,
    pub rank_drum: Option<i32>,
    pub rank_guitar: Option<i32>,
    pub rank_bass: Option<i32>,
    pub rank_vocals: Option<i32>,
    pub rank_keys: Option<i32>,
    pub rank_band: Option<i32>,
    pub rank_real_guitar: Option<i32>,
    pub rank_real_bass: Option<i32>,
    pub rank_real_keys: Option<i32>,
    pub author: String,
}

/// Summary info for the file list (quick to parse from header only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongSummary {
    pub path: String,
    pub display_name: String,
    pub description: String,
    pub title_name: String,
    pub has_thumbnail: bool,
    #[serde(default)]
    pub is_folder: bool,
    #[serde(default)]
    pub album_name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub game_origin: String,
}

/// Full details for the metadata editor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongDetails {
    pub path: String,
    pub display_name: String,
    pub description: String,
    pub title_name: String,
    pub thumbnail_base64: String,
    pub metadata: SongMetadata,
    pub raw_dta: String,
    pub dta_file_size: u32,
    pub validation_issues: Vec<ValidationIssue>,
    // True for unpacked song folders (song.ini), false for CON/STFS packages.
    // The editor uses this to interpret difficulties: song.ini `diff_*` are
    // native 0-6 tiers, while DTA ranks are the Rock Band 0-400+ scale.
    pub is_folder: bool,
}
