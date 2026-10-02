//! Shared download plumbing for the chart browser: stream a URL to a temp
//! file (verified against Content-Length, one retry), resolve off-site hosts
//! (`hosts`), and unpack whatever arrived (`archive`).

pub mod archive;
pub mod hosts;

use reqwest::Client;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Off-site hosts (Google Drive, Mediafire, …) serve different pages to
/// non-browser agents, so external requests present as a desktop browser.
pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// A scratch file under the OS temp dir, deleted on drop. Downloads land here
/// rather than in the library folder so a scan running mid-download never
/// picks up a half-written CON package.
pub struct TempFile(PathBuf);

impl TempFile {
    pub fn new(tag: &str) -> Result<Self, String> {
        let dir = std::env::temp_dir().join("yargle-downloads");
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create temp folder: {}", e))?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Ok(TempFile(dir.join(format!("{}-{}.part", tag, nanos))))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// What a finished transfer looked like.
pub struct Fetched {
    pub size: u64,
    /// Lowercased Content-Type, e.g. `application/zip` or `text/html; charset=utf-8`.
    pub content_type: String,
    /// Suggested name from Content-Disposition, else the final URL's last segment.
    pub filename: Option<String>,
}

/// GET `url` into `out`, reporting `on_progress(received, total)` as it goes.
///
/// A transfer that ends short of Content-Length (or drops mid-stream) is
/// retried once from scratch, then fails with a clear error instead of
/// leaving a truncated file that looks like a success.
pub async fn fetch_to_file(
    client: &Client,
    url: &str,
    headers: &[(&str, &str)],
    out: &Path,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<Fetched, String> {
    let mut last_err = String::new();
    for attempt in 0..2 {
        match fetch_once(client, url, headers, out, on_progress).await {
            Ok(f) => return Ok(f),
            Err(FetchError::Fatal(e)) => return Err(e),
            Err(FetchError::Retryable(e)) => {
                last_err = e;
                if attempt == 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                }
            }
        }
    }
    Err(last_err)
}

enum FetchError {
    Fatal(String),
    Retryable(String),
}

async fn fetch_once(
    client: &Client,
    url: &str,
    headers: &[(&str, &str)],
    out: &Path,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<Fetched, FetchError> {
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut resp = req
        .send()
        .await
        .map_err(|e| FetchError::Retryable(format!("Download request failed: {}", e)))?;
    let status = resp.status();
    if status.is_server_error() {
        return Err(FetchError::Retryable(format!("Download failed: HTTP {}", status)));
    }
    if !status.is_success() {
        return Err(FetchError::Fatal(format!("Download failed: HTTP {}", status)));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let cd_name = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(filename_from_disposition);
    let url_name = resp
        .url()
        .path_segments()
        .and_then(|segs| segs.filter(|s| !s.is_empty()).last().map(percent_decode));
    let filename = cd_name.or(url_name);

    let total = resp.content_length().unwrap_or(0);
    let mut file =
        File::create(out).map_err(|e| FetchError::Fatal(format!("Failed to create temp file: {}", e)))?;
    let mut received: u64 = 0;
    let mut last_emit: u64 = 0;
    loop {
        // A connection that closes early surfaces here as a body error rather
        // than a clean short read.
        let chunk = resp.chunk().await.map_err(|e| {
            FetchError::Retryable(if total > 0 {
                cut_off_message(received, total)
            } else {
                format!("Download interrupted: {}", e)
            })
        })?;
        let Some(chunk) = chunk else { break };
        file.write_all(&chunk)
            .map_err(|e| FetchError::Fatal(format!("Failed to write download: {}", e)))?;
        received += chunk.len() as u64;
        if received - last_emit >= 256 * 1024 || (total > 0 && received >= total) {
            last_emit = received;
            on_progress(received, total);
        }
    }
    file.flush()
        .map_err(|e| FetchError::Fatal(format!("Failed to write download: {}", e)))?;

    if total > 0 && received < total {
        return Err(FetchError::Retryable(cut_off_message(received, total)));
    }

    Ok(Fetched { size: received, content_type, filename })
}

/// Move a finished temp file to its final home. `rename` fails across
/// volumes (temp dir on C:, library on a USB drive), so fall back to a copy;
/// the `TempFile` guard removes the original either way.
pub fn place_file(temp: &Path, dest: &Path) -> Result<(), String> {
    if fs::rename(temp, dest).is_ok() {
        return Ok(());
    }
    fs::copy(temp, dest)
        .map(|_| ())
        .map_err(|e| format!("Failed to save file: {}", e))
}

/// Heuristic: does this payload look like an HTML page (a sign-in wall, a
/// quota notice) rather than a real file? Zips start with `PK`, CON/STFS with
/// `CON `/`LIVE`/`PIRS`, so a leading `<` plus an html-ish tag is a strong signal.
pub fn looks_like_html(head: &[u8]) -> bool {
    let head = &head[..head.len().min(512)];
    if head.iter().copied().find(|b| !b.is_ascii_whitespace()) != Some(b'<') {
        return false;
    }
    let s = String::from_utf8_lossy(head).to_ascii_lowercase();
    s.contains("<!doctype") || s.contains("<html") || s.contains("<head") || s.contains("<body")
}

/// First `n` bytes of a file (fewer if it's shorter).
pub fn read_head(path: &Path, n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(n);
    if let Ok(f) = File::open(path) {
        let _ = f.take(n as u64).read_to_end(&mut buf);
    }
    buf
}

fn cut_off_message(received: u64, total: u64) -> String {
    format!(
        "Download was cut off: got {} of {}. Try again later.",
        human_size(received),
        human_size(total)
    )
}

fn human_size(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Minimal percent-decoder for filenames pulled out of URLs/headers.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Parse a filename out of a Content-Disposition header value.
fn filename_from_disposition(cd: &str) -> Option<String> {
    let lower = cd.to_ascii_lowercase();
    if let Some(pos) = lower.find("filename*=") {
        let val = cd[pos + "filename*=".len()..]
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"');
        // RFC 5987: charset'lang'value — take the part after the last ''
        let encoded = val.rsplit("''").next().unwrap_or(val);
        let decoded = percent_decode(encoded);
        if !decoded.is_empty() {
            return Some(decoded);
        }
    }
    if let Some(pos) = lower.find("filename=") {
        let val = cd[pos + "filename=".len()..]
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"');
        if !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

/// Reduce a suggested name to a safe basename, falling back to `fallback`.
pub fn sanitize_filename(name: Option<String>, fallback: &str) -> String {
    let raw = name.unwrap_or_default();
    // Keep only the final path component and drop anything unsafe.
    let base = raw
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or("")
        .trim();
    let cleaned: String = base
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\0') && !c.is_control())
        .collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c == ' ');
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A server that promises 1000 bytes, sends 400 and hangs up must be
    /// retried once and then reported as cut off — never saved as a success.
    #[test]
    fn truncated_download_retries_then_fails() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let server_hits = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                server_hits.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(&[b'x'; 400]);
            }
        });

        let temp = TempFile::new("truncation-test").unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result = rt.block_on(async {
            let client = Client::new();
            fetch_to_file(&client, &format!("http://{}/song.zip", addr), &[], temp.path(), &|_, _| {}).await
        });
        let err = result.err().expect("truncated download must fail");
        assert!(err.contains("cut off"), "{}", err);
        assert_eq!(hits.load(Ordering::SeqCst), 2, "expected exactly one retry");
    }
}
