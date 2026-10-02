//! Folder naming templates for Organize, e.g. `{genre}/{artist}/{artist} - {title}`.
//!
//! Everything before the last `/` becomes subfolders under the library root;
//! the last segment names the song itself (the folder for a song folder, the
//! file for a CON package). Every segment is sanitized for Windows.

use crate::dta::types::SongMetadata;

/// Tags a template can use, in the order the UI lists them.
pub const TAGS: &[&str] = &["artist", "title", "album", "genre", "year", "charter", "original"];

/// Today's layout before templates existed: `Artist/Album/<unchanged name>`.
pub const DEFAULT_TEMPLATE: &str = "{artist}/{album}/{original}";

/// Values for one song. `original` is its current file or folder name.
pub struct TemplateValues {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub genre: String,
    pub year: String,
    pub charter: String,
    pub original: String,
}

impl TemplateValues {
    pub fn from_metadata(meta: &SongMetadata, original: &str) -> Self {
        TemplateValues {
            artist: meta.artist.clone(),
            title: meta.name.clone(),
            album: meta.album_name.clone(),
            genre: genre_display(&meta.genre),
            year: meta.year_released.filter(|y| *y > 0).map(|y| y.to_string()).unwrap_or_default(),
            charter: meta.author.clone(),
            original: original.to_string(),
        }
    }

    fn get(&self, tag: &str) -> Option<String> {
        let (value, fallback) = match tag {
            "artist" => (&self.artist, "Unknown Artist"),
            "title" => (&self.title, "Unknown Song"),
            "album" => (&self.album, "Unknown Album"),
            "genre" => (&self.genre, "Unknown Genre"),
            "year" => (&self.year, "Unknown Year"),
            "charter" => (&self.charter, "Unknown Charter"),
            "original" => return Some(self.original.clone()),
            _ => return None,
        };
        let clean = crate::duplicates::strip_tags(value).trim().to_string();
        Some(if clean.is_empty() { fallback.to_string() } else { clean })
    }
}

/// Song folders store genres as free text ("Rock"), CON packages as Rock Band
/// codes, sometimes still quoted ("'classicrock'"). Map the codes to their
/// display names so both kinds land in the same `{genre}` folder.
fn genre_display(raw: &str) -> String {
    let g = raw.trim().trim_matches(|c| c == '\'' || c == '"').trim();
    let name = match g.to_ascii_lowercase().as_str() {
        "alternative" => "Alternative",
        "blues" => "Blues",
        "classical" => "Classical",
        "classic" | "classicrock" => "Classic Rock",
        "country" => "Country",
        "emo" => "Emo",
        "fusion" => "Fusion",
        "glam" => "Glam",
        "grunge" => "Grunge",
        "hiphoprap" => "Hip-Hop & Rap",
        "indierock" => "Indie Rock",
        "inspirational" => "Inspirational",
        "jazz" => "Jazz",
        "jrock" => "J-Rock",
        "latin" => "Latin",
        "metal" => "Metal",
        "new_wave" | "newwave" => "New Wave",
        "novelty" => "Novelty",
        "numetal" => "Nu-Metal",
        "other" => "Other",
        "pop" => "Pop",
        "popdanceelectronic" => "Pop, Dance & Electronic",
        "poprock" => "Pop-Rock",
        "prog" => "Prog",
        "punk" => "Punk",
        "rb" => "R&B",
        "rbsoulfunk" => "R&B, Soul & Funk",
        "reggaeska" => "Reggae & Ska",
        "rock" => "Rock",
        "southernrock" => "Southern Rock",
        "urban" => "Urban",
        "world" => "World",
        _ => return g.to_string(),
    };
    name.to_string()
}

/// Check a template without rendering it: non-empty name segment, known tags,
/// balanced braces.
pub fn validate(template: &str) -> Result<(), String> {
    let segments = split(template);
    if segments.last().map_or(true, |s| s.trim().is_empty()) {
        return Err("The template needs a name after the last \"/\".".into());
    }
    for seg in &segments {
        let mut rest = seg.as_str();
        while let Some(open) = rest.find('{') {
            let close = rest[open..]
                .find('}')
                .ok_or_else(|| format!("Missing \"}}\" in \"{}\".", seg))?;
            let tag = &rest[open + 1..open + close];
            if !TAGS.contains(&tag.to_ascii_lowercase().as_str()) {
                return Err(format!(
                    "Unknown tag {{{}}}. Use {}.",
                    tag,
                    TAGS.iter().map(|t| format!("{{{}}}", t)).collect::<Vec<_>>().join(", ")
                ));
            }
            rest = &rest[open + close + 1..];
        }
        if rest.contains('}') {
            return Err(format!("Stray \"}}\" in \"{}\".", seg));
        }
    }
    Ok(())
}

/// Render a template: (subfolders, song name). Empty folder segments (e.g. a
/// lone "/") are dropped.
pub fn render(template: &str, values: &TemplateValues) -> Result<(Vec<String>, String), String> {
    validate(template)?;
    let mut parts: Vec<String> = split(template)
        .iter()
        .map(|seg| sanitize(&fill(seg, values)))
        .collect();
    let name = parts.pop().unwrap_or_default();
    if name.is_empty() {
        return Err("The song's name came out empty.".into());
    }
    parts.retain(|p| !p.is_empty());
    Ok((parts, name))
}

fn split(template: &str) -> Vec<String> {
    template.split(|c| c == '/' || c == '\\').map(str::to_string).collect()
}

fn fill(segment: &str, values: &TemplateValues) -> String {
    let mut out = String::new();
    let mut rest = segment;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        match rest[open..].find('}') {
            Some(close) => {
                let tag = rest[open + 1..open + close].to_ascii_lowercase();
                out.push_str(&values.get(&tag).unwrap_or_default());
                rest = &rest[open + close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Windows-safe path segment: no reserved characters, collapsed whitespace,
/// no trailing dots/spaces (which `\\?\` paths would keep verbatim, leaving
/// folders Explorer can't open), and no reserved device names.
fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| !r#"<>:"/\|?*"#.contains(*c) && !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(|c: char| c == '.' || c == ' ')
        .to_string();
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("{}_", cleaned)
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals() -> TemplateValues {
        TemplateValues {
            artist: "AC/DC".into(),
            title: "Back In Black".into(),
            album: "".into(),
            genre: "rock".into(),
            year: "1980".into(),
            charter: "<color=#FF0000>Harmonix</color>".into(),
            original: "acdc_bib_rb3con".into(),
        }
    }

    #[test]
    fn renders_folders_and_name() {
        let (dirs, name) = render("{genre}/{artist}/{artist} - {title}", &vals()).unwrap();
        assert_eq!(dirs, ["rock", "ACDC"]);
        assert_eq!(name, "ACDC - Back In Black");
    }

    #[test]
    fn default_keeps_original_name() {
        let (dirs, name) = render(DEFAULT_TEMPLATE, &vals()).unwrap();
        assert_eq!(dirs, ["ACDC", "Unknown Album"]);
        assert_eq!(name, "acdc_bib_rb3con");
    }

    #[test]
    fn strips_rich_text_and_trailing_dots() {
        let mut v = vals();
        v.title = "Song...".into();
        let (dirs, name) = render("{charter}/{title}", &v).unwrap();
        assert_eq!(dirs, ["Harmonix"]);
        assert_eq!(name, "Song");
        v.title = "con".into();
        assert_eq!(render("{title}", &v).unwrap().1, "con_");
    }

    #[test]
    fn maps_rock_band_genres() {
        assert_eq!(genre_display("'classicrock'"), "Classic Rock");
        assert_eq!(genre_display("rbsoulfunk"), "R&B, Soul & Funk");
        assert_eq!(genre_display("Acid Jazz"), "Acid Jazz");
    }

    #[test]
    fn rejects_bad_templates() {
        assert!(validate("{artist}/").is_err());
        assert!(validate("{artst} - {title}").unwrap_err().contains("Unknown tag {artst}"));
        assert!(validate("{artist - {title}").is_err());
        assert!(validate("{Artist} - {TITLE}").is_ok());
    }
}
