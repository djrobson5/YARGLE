use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtResult {
    pub url: String,
    pub source: String,
    pub thumbnail_url: String,
}

fn build_client() -> Result<Client, String> {
    Client::builder()
        .user_agent("YARGLE/1.0 (album art lookup)")
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

pub async fn search_album_art(artist: &str, album: &str) -> Result<Vec<ArtResult>, String> {
    let client = build_client()?;
    let mut results = Vec::new();

    // iTunes search (fast, single hop)
    if let Ok(mut itunes) = search_itunes(&client, artist, album).await {
        results.append(&mut itunes);
    }

    // MusicBrainz + Cover Art Archive (slower, two hops)
    if let Ok(mut mb) = search_musicbrainz(&client, artist, album).await {
        results.append(&mut mb);
    }

    Ok(results)
}

pub async fn download_art(client: &Client, url: &str) -> Result<String, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();

    let mime = if content_type.contains("png") {
        "image/png"
    } else {
        "image/jpeg"
    };

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read image: {}", e))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, b64))
}

// --- iTunes ---

#[derive(Deserialize)]
struct ItunesResponse {
    results: Vec<ItunesAlbum>,
}

#[derive(Deserialize)]
struct ItunesAlbum {
    #[serde(rename = "artworkUrl100")]
    artwork_url_100: Option<String>,
}

async fn search_itunes(client: &Client, artist: &str, album: &str) -> Result<Vec<ArtResult>, String> {
    let query = format!("{} {}", artist, album);
    let resp: ItunesResponse = client
        .get("https://itunes.apple.com/search")
        .query(&[
            ("term", query.as_str()),
            ("entity", "album"),
            ("limit", "6"),
        ])
        .send()
        .await
        .map_err(|e| format!("iTunes request failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("iTunes parse failed: {}", e))?;

    Ok(resp
        .results
        .into_iter()
        .filter_map(|a| {
            let thumb = a.artwork_url_100?;
            let full = thumb.replace("100x100bb", "600x600bb");
            Some(ArtResult {
                url: full,
                source: "iTunes".to_string(),
                thumbnail_url: thumb,
            })
        })
        .collect())
}

// --- MusicBrainz + Cover Art Archive ---

#[derive(Deserialize)]
struct MbResponse {
    releases: Option<Vec<MbRelease>>,
}

#[derive(Deserialize)]
struct MbRelease {
    id: String,
}

#[derive(Deserialize)]
struct CaaResponse {
    images: Vec<CaaImage>,
}

#[derive(Deserialize)]
struct CaaImage {
    front: Option<bool>,
    image: Option<String>,
    thumbnails: Option<CaaThumbnails>,
}

#[derive(Deserialize)]
struct CaaThumbnails {
    small: Option<String>,
    #[serde(rename = "250")]
    size_250: Option<String>,
}

async fn search_musicbrainz(client: &Client, artist: &str, album: &str) -> Result<Vec<ArtResult>, String> {
    let query = format!("artist:{} release:{}", artist, album);
    let resp: MbResponse = client
        .get("https://musicbrainz.org/ws/2/release/")
        .query(&[
            ("query", query.as_str()),
            ("fmt", "json"),
            ("limit", "6"),
        ])
        .send()
        .await
        .map_err(|e| format!("MusicBrainz request failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("MusicBrainz parse failed: {}", e))?;

    let releases = resp.releases.unwrap_or_default();
    let mut results = Vec::new();

    for release in releases.iter().take(6) {
        let caa_url = format!("https://coverartarchive.org/release/{}", release.id);
        let caa_resp = match client.get(&caa_url).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => continue,
        };

        let caa: CaaResponse = match caa_resp.json().await {
            Ok(c) => c,
            Err(_) => continue,
        };

        for img in &caa.images {
            if img.front != Some(true) {
                continue;
            }
            if let Some(full_url) = &img.image {
                let thumb = img
                    .thumbnails
                    .as_ref()
                    .and_then(|t| t.small.as_ref().or(t.size_250.as_ref()))
                    .cloned()
                    .unwrap_or_else(|| full_url.clone());

                results.push(ArtResult {
                    url: full_url.clone(),
                    source: "MusicBrainz".to_string(),
                    thumbnail_url: thumb,
                });
            }
            break; // one front image per release
        }
    }

    Ok(results)
}
