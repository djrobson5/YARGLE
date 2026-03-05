use super::parser::DtaNode;
use super::types::SongMetadata;

/// Serialize a DTA parse tree back to text
pub fn serialize_dta(nodes: &[DtaNode]) -> String {
    let mut out = String::new();
    for node in nodes {
        serialize_node(node, &mut out);
    }
    out
}

fn serialize_node(node: &DtaNode, out: &mut String) {
    match node {
        DtaNode::Symbol(s) => out.push_str(s),
        DtaNode::String(s) => {
            out.push('"');
            out.push_str(s);
            out.push('"');
        }
        DtaNode::Int(i) => out.push_str(&i.to_string()),
        DtaNode::Float(f) => out.push_str(&format!("{}", f)),
        DtaNode::List(items) => {
            out.push('(');
            for item in items {
                serialize_node(item, out);
            }
            out.push(')');
        }
        DtaNode::Comment(s) => out.push_str(s),
        DtaNode::Whitespace(s) => out.push_str(s),
    }
}

/// Get the key (first meaningful symbol) from a list's nodes
fn get_key(nodes: &[DtaNode]) -> Option<&str> {
    for node in nodes {
        match node {
            DtaNode::Symbol(s) => return Some(s.as_str()),
            DtaNode::Whitespace(_) | DtaNode::Comment(_) => continue,
            _ => return None,
        }
    }
    None
}

/// Apply metadata changes to a parsed DTA tree
/// Returns the modified tree
pub fn apply_metadata(nodes: &mut Vec<DtaNode>, meta: &SongMetadata) {
    // Find the first top-level list (the song entry)
    let song_list = nodes.iter_mut().find_map(|n| match n {
        DtaNode::List(items) => Some(items),
        _ => None,
    });

    let items = match song_list {
        Some(items) => items,
        None => return,
    };

    // Walk through sub-lists and update matching keys
    for item in items.iter_mut() {
        if let DtaNode::List(sub) = item {
            let key = match get_key(sub) {
                Some(k) => k.to_string(),
                None => continue,
            };

            match key.as_str() {
                "name" => set_string_value(sub, &meta.name),
                "artist" => set_string_value(sub, &meta.artist),
                "album_name" => set_string_value(sub, &meta.album_name),
                "album_track_number" => {
                    if let Some(v) = meta.album_track_number {
                        set_int_value(sub, v);
                    }
                }
                "genre" => set_symbol_value_after_key(sub, &meta.genre),
                "sub_genre" => set_symbol_value_after_key(sub, &meta.sub_genre),
                "vocal_gender" => set_symbol_value_after_key(sub, &meta.vocal_gender),
                "year_released" => {
                    if let Some(v) = meta.year_released {
                        set_int_value(sub, v);
                    }
                }
                "song_length" => {
                    if let Some(v) = meta.song_length {
                        set_int_value(sub, v);
                    }
                }
                "rating" => {
                    if let Some(v) = meta.rating {
                        set_int_value(sub, v);
                    }
                }
                "song_id" => {
                    if let Some(v) = meta.song_id {
                        set_int_value(sub, v);
                    }
                }
                "game_origin" => set_symbol_value_after_key(sub, &meta.game_origin),
                "preview" => {
                    if let (Some(start), Some(end)) = (meta.preview_start, meta.preview_end) {
                        set_nth_int(sub, 0, start);
                        set_nth_int(sub, 1, end);
                    }
                }
                "rank" => {
                    for rank_item in sub.iter_mut() {
                        if let DtaNode::List(pair) = rank_item {
                            let instrument = match get_key(pair) {
                                Some(k) => k.to_string(),
                                None => continue,
                            };
                            let value = match instrument.as_str() {
                                "drum" => meta.rank_drum,
                                "guitar" => meta.rank_guitar,
                                "bass" => meta.rank_bass,
                                "vocals" => meta.rank_vocals,
                                "keys" => meta.rank_keys,
                                "band" => meta.rank_band,
                                "real_guitar" => meta.rank_real_guitar,
                                "real_bass" => meta.rank_real_bass,
                                "real_keys" => meta.rank_real_keys,
                                _ => None,
                            };
                            if let Some(v) = value {
                                set_int_value(pair, v);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Set the first String node after the key symbol
fn set_string_value(nodes: &mut [DtaNode], value: &str) {
    let mut found_key = false;
    for node in nodes.iter_mut() {
        match node {
            DtaNode::Symbol(_) if !found_key => { found_key = true; }
            DtaNode::String(_) if found_key => {
                *node = DtaNode::String(value.to_string());
                return;
            }
            DtaNode::Whitespace(_) | DtaNode::Comment(_) => continue,
            _ => {}
        }
    }
    // If no string found after key, try replacing a symbol after the key
    let mut found_key = false;
    for node in nodes.iter_mut() {
        match node {
            DtaNode::Symbol(_) if !found_key => { found_key = true; }
            DtaNode::Symbol(_) if found_key => {
                *node = DtaNode::String(value.to_string());
                return;
            }
            DtaNode::Whitespace(_) | DtaNode::Comment(_) => continue,
            _ => {}
        }
    }
}

/// Set the first symbol after the key symbol
fn set_symbol_value_after_key(nodes: &mut [DtaNode], value: &str) {
    let mut found_key = false;
    for node in nodes.iter_mut() {
        match node {
            DtaNode::Symbol(_) if !found_key => { found_key = true; }
            DtaNode::Symbol(_) if found_key => {
                *node = DtaNode::Symbol(value.to_string());
                return;
            }
            DtaNode::Whitespace(_) | DtaNode::Comment(_) => continue,
            _ => {}
        }
    }
}

/// Set the first Int node after the key symbol
fn set_int_value(nodes: &mut [DtaNode], value: i32) {
    let mut found_key = false;
    for node in nodes.iter_mut() {
        match node {
            DtaNode::Symbol(_) if !found_key => { found_key = true; }
            DtaNode::Int(_) if found_key => {
                *node = DtaNode::Int(value);
                return;
            }
            DtaNode::Whitespace(_) | DtaNode::Comment(_) => continue,
            _ => {}
        }
    }
}

/// Set the Nth Int node in the list (0-indexed, skipping whitespace/comments)
fn set_nth_int(nodes: &mut [DtaNode], n: usize, value: i32) {
    let mut count = 0;
    for node in nodes.iter_mut() {
        if let DtaNode::Int(_) = node {
            if count == n {
                *node = DtaNode::Int(value);
                return;
            }
            count += 1;
        }
    }
}
