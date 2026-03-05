use super::types::SongMetadata;

/// A node in the DTA parse tree
#[derive(Debug, Clone)]
pub enum DtaNode {
    Symbol(String),
    String(String),
    Int(i32),
    Float(f64),
    List(Vec<DtaNode>),
    Comment(String),
    Whitespace(String),
}

/// Parse raw DTA text into a list of top-level nodes
pub fn parse_dta(input: &str) -> Result<Vec<DtaNode>, String> {
    let mut parser = DtaParser::new(input);
    parser.parse_top_level()
}

/// Filter out whitespace and comment nodes to get only meaningful content
fn meaningful_nodes(nodes: &[DtaNode]) -> Vec<&DtaNode> {
    nodes
        .iter()
        .filter(|n| !matches!(n, DtaNode::Whitespace(_) | DtaNode::Comment(_)))
        .collect()
}

/// Get the key (first meaningful symbol) from a list's nodes
fn get_key<'a>(nodes: &'a [DtaNode]) -> Option<&'a str> {
    for node in nodes {
        match node {
            DtaNode::Symbol(s) => return Some(s.as_str()),
            DtaNode::Whitespace(_) | DtaNode::Comment(_) => continue,
            _ => return None,
        }
    }
    None
}

/// Extract SongMetadata from parsed DTA nodes
pub fn extract_metadata(nodes: &[DtaNode], raw_text: &str) -> SongMetadata {
    let mut meta = SongMetadata::default();

    // Extract author from comments
    for line in raw_text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(";Song authored by ") {
            meta.author = rest.trim().to_string();
        } else if let Some(rest) = trimmed.strip_prefix(";Author: ") {
            meta.author = rest.trim().to_string();
        }
    }

    // Find the first top-level list (the song entry)
    let song_list = nodes.iter().find_map(|n| match n {
        DtaNode::List(items) => Some(items),
        _ => None,
    });

    let items = match song_list {
        Some(items) => items,
        None => return meta,
    };

    // First meaningful element is the shortname
    for item in items {
        if let DtaNode::Symbol(name) = item {
            meta.shortname = name.clone();
            break;
        }
    }

    // Walk through sub-lists to find key-value pairs
    for item in items.iter() {
        if let DtaNode::List(sub) = item {
            let key = match get_key(sub) {
                Some(k) => k,
                None => continue,
            };

            match key {
                "name" => meta.name = get_string_value(sub),
                "artist" => meta.artist = get_string_value(sub),
                "album_name" => meta.album_name = get_string_value(sub),
                "album_track_number" => meta.album_track_number = get_int_value(sub),
                "genre" => meta.genre = get_symbol_value_after_key(sub),
                "sub_genre" => meta.sub_genre = get_symbol_value_after_key(sub),
                "vocal_gender" => meta.vocal_gender = get_symbol_value_after_key(sub),
                "year_released" => meta.year_released = get_int_value(sub),
                "song_length" => meta.song_length = get_int_value(sub),
                "rating" => meta.rating = get_int_value(sub),
                "song_id" => meta.song_id = get_int_value(sub),
                "game_origin" => meta.game_origin = get_symbol_value_after_key(sub),
                "preview" => {
                    let ints = get_all_ints(sub);
                    if ints.len() >= 2 {
                        meta.preview_start = Some(ints[0]);
                        meta.preview_end = Some(ints[1]);
                    }
                }
                "rank" => {
                    // (rank ('drum' N) ('guitar' N) ...)
                    for rank_item in sub.iter() {
                        if let DtaNode::List(rank_pair) = rank_item {
                            let instrument = match get_key(rank_pair) {
                                Some(k) => k,
                                None => continue,
                            };
                            let value = get_int_value(rank_pair);
                            match instrument {
                                "drum" => meta.rank_drum = value,
                                "guitar" => meta.rank_guitar = value,
                                "bass" => meta.rank_bass = value,
                                "vocals" => meta.rank_vocals = value,
                                "keys" => meta.rank_keys = value,
                                "band" => meta.rank_band = value,
                                "real_guitar" => meta.rank_real_guitar = value,
                                "real_bass" => meta.rank_real_bass = value,
                                "real_keys" => meta.rank_real_keys = value,
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    meta
}

fn get_string_value(nodes: &[DtaNode]) -> String {
    for node in nodes {
        if let DtaNode::String(s) = node {
            return s.clone();
        }
    }
    String::new()
}

/// Get the first symbol value AFTER the key symbol
fn get_symbol_value_after_key(nodes: &[DtaNode]) -> String {
    let mut found_key = false;
    for node in nodes {
        match node {
            DtaNode::Symbol(_) if !found_key => {
                found_key = true; // skip the key
            }
            DtaNode::Symbol(s) if found_key => return s.clone(),
            DtaNode::String(s) if found_key => return s.clone(),
            DtaNode::Whitespace(_) | DtaNode::Comment(_) => continue,
            _ => {
                if found_key {
                    continue;
                }
            }
        }
    }
    String::new()
}

fn get_int_value(nodes: &[DtaNode]) -> Option<i32> {
    for node in nodes {
        if let DtaNode::Int(v) = node {
            return Some(*v);
        }
    }
    None
}

/// Collect all integer values from a node list (skipping whitespace, key, etc.)
fn get_all_ints(nodes: &[DtaNode]) -> Vec<i32> {
    nodes
        .iter()
        .filter_map(|n| if let DtaNode::Int(v) = n { Some(*v) } else { None })
        .collect()
}

struct DtaParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> DtaParser<'a> {
    fn new(input: &'a str) -> Self {
        DtaParser { input, pos: 0 }
    }

    fn remaining(&self) -> &str {
        &self.input[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn parse_top_level(&mut self) -> Result<Vec<DtaNode>, String> {
        let mut nodes = Vec::new();
        loop {
            self.skip_whitespace_and_comments(&mut nodes);
            if self.pos >= self.input.len() {
                break;
            }
            match self.peek() {
                Some('(') => nodes.push(self.parse_list()?),
                Some(')') => {
                    self.advance();
                    // stray close paren, ignore
                }
                None => break,
                _ => nodes.push(self.parse_atom()?),
            }
        }
        Ok(nodes)
    }

    fn skip_whitespace_and_comments(&mut self, nodes: &mut Vec<DtaNode>) {
        loop {
            // Whitespace
            let start = self.pos;
            while let Some(ch) = self.peek() {
                if ch.is_whitespace() {
                    self.advance();
                } else {
                    break;
                }
            }
            if self.pos > start {
                nodes.push(DtaNode::Whitespace(self.input[start..self.pos].to_string()));
            }

            // Comment
            if self.peek() == Some(';') {
                let start = self.pos;
                while let Some(ch) = self.peek() {
                    if ch == '\n' {
                        self.advance();
                        break;
                    }
                    self.advance();
                }
                nodes.push(DtaNode::Comment(self.input[start..self.pos].to_string()));
            } else {
                break;
            }
        }
    }

    fn parse_list(&mut self) -> Result<DtaNode, String> {
        self.advance(); // consume '('
        let mut items = Vec::new();
        loop {
            self.skip_whitespace_and_comments(&mut items);
            match self.peek() {
                Some(')') => {
                    self.advance();
                    return Ok(DtaNode::List(items));
                }
                Some('(') => items.push(self.parse_list()?),
                None => return Err("Unexpected end of input in list".into()),
                _ => items.push(self.parse_atom()?),
            }
        }
    }

    fn parse_atom(&mut self) -> Result<DtaNode, String> {
        match self.peek() {
            Some('"') => self.parse_quoted_string(),
            Some('\'') => {
                self.advance(); // consume opening quote
                if self.peek() == Some('(') {
                    // Quoted list like '(...)
                    self.parse_list()
                } else {
                    // Quoted symbol like 'name' — read until closing quote
                    let start = self.pos;
                    while let Some(ch) = self.peek() {
                        if ch == '\'' {
                            let s = self.input[start..self.pos].to_string();
                            self.advance(); // consume closing quote
                            return Ok(DtaNode::Symbol(s));
                        }
                        if ch.is_whitespace() || ch == '(' || ch == ')' {
                            break;
                        }
                        self.advance();
                    }
                    // No closing quote found — return what we have
                    Ok(DtaNode::Symbol(self.input[start..self.pos].to_string()))
                }
            }
            _ => self.parse_symbol_or_number(),
        }
    }

    fn parse_quoted_string(&mut self) -> Result<DtaNode, String> {
        self.advance(); // consume opening quote
        let start = self.pos;
        let mut escaped = false;
        loop {
            match self.peek() {
                None => {
                    return Ok(DtaNode::String(self.input[start..self.pos].to_string()));
                }
                Some('\\') if !escaped => {
                    escaped = true;
                    self.advance();
                }
                Some('"') if !escaped => {
                    let s = self.input[start..self.pos].to_string();
                    self.advance(); // consume closing quote
                    return Ok(DtaNode::String(s));
                }
                _ => {
                    escaped = false;
                    self.advance();
                }
            }
        }
    }

    fn parse_symbol(&mut self) -> Result<DtaNode, String> {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() || ch == '(' || ch == ')' || ch == ';' {
                break;
            }
            self.advance();
        }
        Ok(DtaNode::Symbol(self.input[start..self.pos].to_string()))
    }

    fn parse_symbol_or_number(&mut self) -> Result<DtaNode, String> {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() || ch == '(' || ch == ')' || ch == ';' {
                break;
            }
            self.advance();
        }
        let token = &self.input[start..self.pos];

        // Try integer
        if let Ok(i) = token.parse::<i32>() {
            return Ok(DtaNode::Int(i));
        }
        // Try float
        if let Ok(f) = token.parse::<f64>() {
            return Ok(DtaNode::Float(f));
        }
        // Otherwise symbol
        Ok(DtaNode::Symbol(token.to_string()))
    }
}
