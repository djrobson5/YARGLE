//! Guess a song's artist and title from its file or folder name, for songs
//! whose metadata is missing them. Adapted from Clone Hero Chart Manager's
//! `parseFileName` / `humanize`: strip the extension and trailing release tags
//! (`_rb3con`, `_v2`, `CH`…), split on " - ", then un-squash names like
//! `SonicYouth` or `Sonic.Youth`.

/// Extensions to drop. Only known ones, so a dot inside a name ("Mr. Brightside") stays.
const EXTENSIONS: &[&str] = &["con", "live", "sng", "zip", "rar", "7z", "mid", "chart"];

/// Release tags that trail a file name, compared case-insensitively. `v<digits>` is handled separately.
const TAGS: &[&str] = &[
    "rb3con", "rb2con", "chps", "con", "ps", "ch", "rb", "rb1", "rb2", "rb3", "rb4", "ps3", "ps4", "xbox",
    "wii", "chart", "final", "fixed", "update", "updated",
];

fn strip_extension(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)) => stem,
        _ => name,
    }
}

fn is_tag(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    TAGS.contains(&lower.as_str())
        || (lower.len() > 1 && lower.starts_with('v') && lower[1..].bytes().all(|b| b.is_ascii_digit()))
}

/// Drop trailing tags, repeatedly. A tag must follow a separator, so the
/// "ch" in "Bach" or the "final" in "TheFinal" is left alone.
fn strip_tags(mut name: &str) -> &str {
    loop {
        let trimmed = name.trim_end_matches(|c: char| c == '_' || c == '-' || c == '.' || c.is_whitespace());
        let Some(cut) = trimmed.rfind(|c: char| c == '_' || c == '-' || c == '.' || c.is_whitespace()) else {
            return trimmed;
        };
        let (head, word) = (&trimmed[..cut], &trimmed[cut + 1..]);
        if head.trim().is_empty() || !is_tag(word) {
            return trimmed;
        }
        name = head;
    }
}

/// Space out a squashed name: `SonicYouth` → `Sonic Youth`, `ABCWord` → `ABC Word`,
/// `Sonic.Youth` → `Sonic Youth`. Names that already contain spaces are left
/// alone, so "Paul McCartney" and "Mr. Brightside" survive.
fn humanize(part: &str) -> String {
    let part = part.trim();
    if part.contains(' ') {
        return part.to_string();
    }
    let chars: Vec<char> = part.replace('.', " ").chars().collect();
    let mut out = String::with_capacity(chars.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_ascii_digit() || (prev.is_uppercase() && next_lower) {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `(artist, title)` guessed from a file or folder name, or `None` when the
/// name has no " - " separating the two.
pub fn guess_artist_title(file_name: &str) -> Option<(String, String)> {
    let name = strip_tags(strip_extension(file_name.trim()));
    let spaced = name.replace('_', " ");
    let spaced = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let (artist, title) = spaced.split_once(" - ")?;
    let (artist, title) = (humanize(artist), humanize(title));
    (!artist.is_empty() && !title.is_empty()).then_some((artist, title))
}

#[cfg(test)]
mod tests {
    use super::guess_artist_title;

    fn check(name: &str, expected: Option<(&str, &str)>) {
        let got = guess_artist_title(name);
        assert_eq!(got.as_ref().map(|(a, t)| (a.as_str(), t.as_str())), expected, "{name}");
    }

    #[test]
    fn library_names() {
        check("Judas Priest - Diamonds and Rust_v2_rb3con", Some(("Judas Priest", "Diamonds and Rust")));
        check(
            "My Chemical Romance - Blood [Hidden Track] (Jaded)_chps_rb3con",
            Some(("My Chemical Romance", "Blood [Hidden Track] (Jaded)")),
        );
        check("Sonic Youth - Candle_rb3con", Some(("Sonic Youth", "Candle")));
        check("Sonic_Youth_-_Candle", Some(("Sonic Youth", "Candle")));
        check("SonicYouth - Candle.sng", Some(("Sonic Youth", "Candle")));
        check("Sonic.Youth - Teen.Age.Riot", Some(("Sonic Youth", "Teen Age Riot")));
        check("ACDC - BackInBlack_final", Some(("ACDC", "Back In Black")));
        check("HTMLRocks - XMLParser", Some(("HTML Rocks", "XML Parser")));
    }

    #[test]
    fn leaves_real_names_alone() {
        check("Paul McCartney - Mr. Brightside", Some(("Paul McCartney", "Mr. Brightside")));
        check("Johann Sebastian Bach - Toccata Bach", Some(("Johann Sebastian Bach", "Toccata Bach")));
        check("Blink-182 - Dammit", Some(("Blink-182", "Dammit")));
        check("The Killers - Mr. Brightside CH", Some(("The Killers", "Mr. Brightside")));
    }

    #[test]
    fn needs_both_parts() {
        check("microchip_rb3con", None);
        check("MazingerZRockanime", None);
        check(" - Candle", None);
        check("Sonic Youth - _rb3con", None);
    }
}
