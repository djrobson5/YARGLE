//! Turn an off-site link from a chart listing into something we can stream.
//!
//! Supported: Google Drive files and folders, Mediafire, Dropbox, the common
//! link shorteners, and plain links straight to an archive. MEGA (encrypted),
//! Ko-fi shops and other web pages stay "open in browser".

use super::BROWSER_UA;
use base64::Engine;
use reqwest::{Client, Url};

const SHORTENERS: &[&str] = &[
    "bit.ly", "tinyurl.com", "t.co", "goo.gl", "ow.ly", "is.gd", "buff.ly", "cutt.ly",
    "rebrand.ly", "shorturl.at", "tiny.cc", "rb.gy",
];

/// Extensions that mark a plain link as a direct file download.
const DIRECT_EXTS: &[&str] = &[".zip", ".7z", ".rar", ".sng"];

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A rhythmverse.co songfile link: use the normal interstitial flow.
    /// Carries the file_id from the URL when it has one.
    RhythmVerse(Option<String>),
    DriveFile(String),
    DriveFolder(String),
    Mediafire(String),
    Dropbox(String),
    Shortener(String),
    Direct(String),
}

/// How to fetch a resolved link.
pub enum Plan {
    File(String),
    DriveFolder(String),
}

/// Classify a link. `None` means YARGLE can't download it (MEGA, Ko-fi, a
/// web page, a non-http scheme) and the UI should offer "Open ↗" only.
pub fn classify(url: &str) -> Option<Source> {
    let u = Url::parse(url.trim()).ok()?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return None;
    }
    let host = u.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let path = u.path();

    if host == "rhythmverse.co" {
        let id = path
            .strip_prefix("/songfile/")
            .or_else(|| path.strip_prefix("/download/"))
            .map(|s| s.trim_matches('/').to_string())
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit()));
        return Some(Source::RhythmVerse(id));
    }
    if host == "drive.google.com" || host == "docs.google.com" || host == "drive.usercontent.google.com" {
        if let Some(id) = segment_after(path, "folders") {
            return Some(Source::DriveFolder(id));
        }
        if let Some(id) = segment_after(path, "d") {
            return Some(Source::DriveFile(id));
        }
        // open?id=…, uc?id=…, download?id=…
        return query_param(&u, "id").map(Source::DriveFile);
    }
    if host.ends_with("mediafire.com") && (path.starts_with("/file/") || path.starts_with("/file_premium/")) {
        return Some(Source::Mediafire(u.to_string()));
    }
    if host == "dropbox.com" || host == "dl.dropboxusercontent.com" {
        // Folder shares (/sh/, /scl/fo/) download as a zip with dl=1 too.
        return Some(Source::Dropbox(u.to_string()));
    }
    if SHORTENERS.contains(&host.as_str()) {
        return Some(Source::Shortener(u.to_string()));
    }
    let lower = path.to_ascii_lowercase();
    if DIRECT_EXTS.iter().any(|ext| lower.ends_with(ext)) {
        return Some(Source::Direct(u.to_string()));
    }
    None
}

/// Resolve a non-RhythmVerse source to a fetch plan, expanding shorteners
/// and scraping host pages for the real file URL.
pub async fn resolve(client: &Client, source: Source) -> Result<Plan, String> {
    let mut source = source;
    // A shortener can point at another shortener; don't follow forever.
    for _ in 0..3 {
        match source {
            Source::Shortener(url) => {
                let final_url = expand_shortlink(client, &url).await?;
                source = classify(&final_url).ok_or_else(|| unsupported_message(&final_url))?;
            }
            Source::DriveFile(id) => return Ok(Plan::File(drive_file_url(&id))),
            Source::DriveFolder(id) => return Ok(Plan::DriveFolder(id)),
            Source::Mediafire(page) => return mediafire_direct(client, &page).await.map(Plan::File),
            Source::Dropbox(url) => return Ok(Plan::File(dropbox_direct(&url))),
            Source::Direct(url) => return Ok(Plan::File(url)),
            Source::RhythmVerse(_) => {
                return Err("This link points back to RhythmVerse; download it from its own row.".into())
            }
        }
    }
    Err("Too many link redirects.".into())
}

pub fn is_drive_url(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.ends_with("google.com")))
        .unwrap_or(false)
}

/// Google Drive's direct-download endpoint. `confirm=t` skips the "can't scan
/// this file for viruses" interstitial that large files otherwise get.
pub fn drive_file_url(id: &str) -> String {
    format!(
        "https://drive.usercontent.google.com/download?id={}&export=download&confirm=t",
        id
    )
}

/// When Drive still answers with its virus-scan page, the page holds a form
/// whose action + hidden inputs make up the real download URL. `None` means
/// there's no form: quota exceeded, or the file isn't shared publicly.
pub fn drive_confirm_url(html: &str) -> Option<String> {
    let form_start = html.find("<form")?;
    let form_end = html[form_start..].find("</form>").map(|i| form_start + i).unwrap_or(html.len());
    let form = &html[form_start..form_end];
    let action = attr_value(form, "action")?.replace("&amp;", "&");
    let mut url = Url::parse(&action).ok()?;
    {
        let mut q = url.query_pairs_mut();
        let mut rest = form;
        while let Some(pos) = rest.find("<input") {
            rest = &rest[pos + 6..];
            let tag_end = rest.find('>').unwrap_or(rest.len());
            let tag = &rest[..tag_end];
            if let (Some(name), Some(value)) = (attr_value(tag, "name"), attr_value(tag, "value")) {
                q.append_pair(&name, &value.replace("&amp;", "&"));
            }
        }
    }
    Some(url.to_string())
}

pub const DRIVE_UNAVAILABLE: &str = "Google Drive won't serve this file right now (its download quota \
     may be used up, or it isn't shared publicly). Use Open ↗ to get it in your browser.";

/// One entry in a shared Drive folder.
pub struct DriveItem {
    pub id: String,
    pub name: String,
    pub is_folder: bool,
}

/// List a public Drive folder by reading the item data its page embeds in
/// `window['_DRIVE_ivd']` (a JS-escaped JSON array of
/// `[id, [parent], name, mimeType, …]` rows). Returns the folder's title too.
pub async fn list_drive_folder(client: &Client, id: &str) -> Result<(String, Vec<DriveItem>), String> {
    let html = client
        .get(format!("https://drive.google.com/drive/folders/{}", id))
        .header("User-Agent", BROWSER_UA)
        .send()
        .await
        .map_err(|e| format!("Google Drive request failed: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Failed to read Google Drive page: {}", e))?;

    let title = html
        .find("<title>")
        .and_then(|s| {
            let rest = &html[s + 7..];
            rest.find("</title>").map(|e| rest[..e].to_string())
        })
        .map(|t| t.trim_end_matches(" - Google Drive").trim().to_string())
        .unwrap_or_default();

    let marker = "window['_DRIVE_ivd'] = '";
    let start = html.find(marker).ok_or(
        "Couldn't read this Google Drive folder (it may be private). Use Open ↗ to get it in your browser.",
    )? + marker.len();
    let end = html[start..].find("';").map(|i| start + i).ok_or("Unexpected Google Drive page")?;
    let json = js_unescape(&html[start..end]);
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|_| "Unexpected Google Drive folder data")?;

    let rows = value.get(0).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let items = rows
        .iter()
        .filter_map(|row| {
            let id = row.get(0)?.as_str()?.to_string();
            let name = row.get(2)?.as_str()?.to_string();
            let mime = row.get(3).and_then(|m| m.as_str()).unwrap_or("");
            Some(DriveItem { id, name, is_folder: mime == "application/vnd.google-apps.folder" })
        })
        .collect();
    Ok((title, items))
}

pub fn unsupported_message(url: &str) -> String {
    let host = Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
        .unwrap_or_else(|| "an external site".into());
    format!(
        "This link leads to {}, which YARGLE can't download automatically. Use Open ↗ to get it in your browser.",
        host
    )
}

async fn expand_shortlink(client: &Client, url: &str) -> Result<String, String> {
    // reqwest follows redirects by default; the response URL is the target.
    let resp = client
        .get(url)
        .header("User-Agent", BROWSER_UA)
        .send()
        .await
        .map_err(|e| format!("Couldn't follow the short link: {}", e))?;
    Ok(resp.url().to_string())
}

async fn mediafire_direct(client: &Client, page: &str) -> Result<String, String> {
    let html = client
        .get(page)
        .header("User-Agent", BROWSER_UA)
        .send()
        .await
        .map_err(|e| format!("Mediafire request failed: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Failed to read Mediafire page: {}", e))?;
    mediafire_link_from_html(&html).ok_or_else(|| {
        "Couldn't find the download on the Mediafire page (the file may have been removed). \
         Use Open ↗ to check it in your browser."
            .into()
    })
}

/// The download button's `href` points at `downloadNNNN.mediafire.com`; some
/// page variants hide it base64-encoded in `data-scrambled-url` instead.
fn mediafire_link_from_html(html: &str) -> Option<String> {
    let mut rest = html;
    while let Some(pos) = rest.find("href=\"") {
        rest = &rest[pos + 6..];
        let end = rest.find('"')?;
        let href = rest[..end].replace("&amp;", "&");
        if is_mediafire_download_host(&href) {
            return Some(if href.starts_with("//") { format!("https:{}", href) } else { href });
        }
    }
    let marker = "data-scrambled-url=\"";
    let pos = html.find(marker)? + marker.len();
    let end = html[pos..].find('"')? + pos;
    let decoded = base64::engine::general_purpose::STANDARD.decode(&html[pos..end]).ok()?;
    let url = String::from_utf8(decoded).ok()?;
    is_mediafire_download_host(&url).then_some(url)
}

fn is_mediafire_download_host(href: &str) -> bool {
    let rest = href
        .strip_prefix("https://")
        .or_else(|| href.strip_prefix("http://"))
        .or_else(|| href.strip_prefix("//"));
    let Some(rest) = rest else { return false };
    let host = rest.split('/').next().unwrap_or("");
    host.ends_with(".mediafire.com")
        && host
            .strip_prefix("download")
            .and_then(|h| h.split('.').next())
            .map(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false)
}

fn dropbox_direct(url: &str) -> String {
    let Ok(mut u) = Url::parse(url) else { return url.to_string() };
    let pairs: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| k != "dl" && k != "raw")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    u.query_pairs_mut().clear().extend_pairs(pairs).append_pair("dl", "1");
    u.to_string()
}

/// The path segment right after `name`, e.g. `/file/d/<id>/view` with "d".
fn segment_after(path: &str, name: &str) -> Option<String> {
    let mut segs = path.split('/').filter(|s| !s.is_empty());
    while let Some(s) = segs.next() {
        if s == name {
            return segs.next().map(str::to_string).filter(|s| !s.is_empty());
        }
    }
    None
}

fn query_param(u: &Url, key: &str) -> Option<String> {
    u.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}

/// Value of `name="…"` inside an HTML tag snippet.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{}=\"", name);
    let mut from = 0;
    while let Some(rel) = tag[from..].find(&needle) {
        let pos = from + rel;
        // Make sure we matched a whole attribute name (not data-name=…).
        let ok = pos == 0 || tag.as_bytes()[pos - 1].is_ascii_whitespace();
        let start = pos + needle.len();
        if ok {
            let end = tag[start..].find('"')? + start;
            return Some(tag[start..end].to_string());
        }
        from = start;
    }
    None
}

/// Decode a JavaScript string literal's escapes (`\xNN`, `\uNNNN`, `\/`,
/// `\"`, …) back into the text it holds — here, a JSON document.
fn js_unescape(s: &str) -> String {
    fn hex(chars: &mut std::iter::Peekable<std::str::Chars>, n: usize) -> Option<u32> {
        let h: String = chars.by_ref().take(n).collect();
        u32::from_str_radix(&h, 16).ok()
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('x') => out.extend(hex(&mut chars, 2).and_then(char::from_u32)),
            Some('u') => {
                let Some(hi) = hex(&mut chars, 4) else { continue };
                if (0xD800..0xDC00).contains(&hi) {
                    // Surrogate pair: expect a following \uDC00–\uDFFF.
                    let mut ahead = chars.clone();
                    if ahead.next() == Some('\\') && ahead.next() == Some('u') {
                        if let Some(lo) = hex(&mut ahead, 4).filter(|lo| (0xDC00..0xE000).contains(lo)) {
                            chars = ahead;
                            out.extend(char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)));
                            continue;
                        }
                    }
                    out.push('\u{FFFD}');
                } else {
                    out.extend(char::from_u32(hi));
                }
            }
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_hosts() {
        assert_eq!(
            classify("https://drive.google.com/file/d/1BNlvpm_BvMQnupGSh91UZ3cxWoE6Mpxn/view?usp=drive_link"),
            Some(Source::DriveFile("1BNlvpm_BvMQnupGSh91UZ3cxWoE6Mpxn".into()))
        );
        assert_eq!(
            classify("https://drive.google.com/open?id=abc123"),
            Some(Source::DriveFile("abc123".into()))
        );
        assert_eq!(
            classify("https://drive.google.com/drive/u/0/folders/18Q7PaNQCOupMPi04zmMNHHohmvau8UHy?usp=drive_link"),
            Some(Source::DriveFolder("18Q7PaNQCOupMPi04zmMNHHohmvau8UHy".into()))
        );
        assert!(matches!(
            classify("https://www.mediafire.com/file/plyezplpqlpen78/La_Vida_Boheme_-_El_Zar_rb3con/file"),
            Some(Source::Mediafire(_))
        ));
        assert_eq!(
            classify("https://rhythmverse.co/songfile/505b3d924a1043708ccdf0ad50f837e9"),
            Some(Source::RhythmVerse(Some("505b3d924a1043708ccdf0ad50f837e9".into())))
        );
        assert!(matches!(classify("https://bit.ly/xyz"), Some(Source::Shortener(_))));
        assert!(matches!(classify("https://example.com/songs/pack.7z"), Some(Source::Direct(_))));
        assert_eq!(classify("https://ko-fi.com/shubanl"), None);
        assert_eq!(classify("https://mega.nz/file/abc#key"), None);
        assert_eq!(classify("ftp://example.com/a.zip"), None);
    }

    #[test]
    fn dropbox_forces_download() {
        assert_eq!(
            dropbox_direct("https://www.dropbox.com/scl/fi/abc/song.zip?rlkey=k&dl=0"),
            "https://www.dropbox.com/scl/fi/abc/song.zip?rlkey=k&dl=1"
        );
    }

    #[test]
    fn mediafire_links() {
        let html = r#"<a class="input popsok" aria-label="Download file" href="https://download1347.mediafire.com/abc/plyezplpqlpen78/La+Vida_rb3con" id="downloadButton">"#;
        assert_eq!(
            mediafire_link_from_html(html).as_deref(),
            Some("https://download1347.mediafire.com/abc/plyezplpqlpen78/La+Vida_rb3con")
        );
        let scrambled = format!(
            r#"<a href="https://www.mediafire.com/" id="downloadButton" data-scrambled-url="{}">"#,
            base64::engine::general_purpose::STANDARD.encode("https://download42.mediafire.com/x/y.zip")
        );
        assert_eq!(
            mediafire_link_from_html(&scrambled).as_deref(),
            Some("https://download42.mediafire.com/x/y.zip")
        );
    }

    #[test]
    fn drive_virus_scan_form() {
        let html = r#"<html><form id="download-form" action="https://drive.usercontent.google.com/download" method="get">
            <input type="submit" id="uc-download-link" value="Download anyway"/>
            <input type="hidden" name="id" value="1abc"><input type="hidden" name="export" value="download">
            <input type="hidden" name="confirm" value="t"><input type="hidden" name="uuid" value="u-1"></form></html>"#;
        assert_eq!(
            drive_confirm_url(html).as_deref(),
            Some("https://drive.usercontent.google.com/download?id=1abc&export=download&confirm=t&uuid=u-1")
        );
        assert_eq!(drive_confirm_url("<html><p>Quota exceeded</p></html>"), None);
    }

    #[test]
    fn unescapes_drive_ivd() {
        let raw = r#"\x5b\x5b\x5b\x221zw\x22,\x5b\x22p\x22\x5d,\x22Ghost Town\x22,\x22application\/vnd.google-apps.folder\x22\x5d\x5d\x5d"#;
        let v: serde_json::Value = serde_json::from_str(&js_unescape(raw)).unwrap();
        assert_eq!(v[0][0][2], "Ghost Town");
        assert_eq!(v[0][0][3], "application/vnd.google-apps.folder");
        // A quote inside a name is JSON-escaped, then JS-escaped on top.
        let raw = r#"\x5b\x22a \x5c\x22b\x5c\x22 é🎸\x22\x5d"#;
        let v: serde_json::Value = serde_json::from_str(&js_unescape(raw)).unwrap();
        assert_eq!(v[0], "a \"b\" \u{e9}\u{1f3b8}");
    }
}
