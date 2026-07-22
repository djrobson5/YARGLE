//! Lightweight update check against GitHub Releases.
//!
//! Deliberately notify-only: we compare the latest release tag against the
//! running version and let the user open the release page in their browser —
//! no auto-download/install (which would need signing keys and updater
//! infrastructure for little gain at this project's scale).

use reqwest::Client;
use serde::{Deserialize, Serialize};

const REPO: &str = "djrobson5/YARGLE";
const USER_AGENT: &str = concat!("YARGLE/", env!("CARGO_PKG_VERSION"), " (update check)");

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
    pub notes: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Parse "v1.2.3" / "1.2.3" into comparable numeric parts.
fn parse_version(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u64>()
                .unwrap_or(0)
        })
        .collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    let l = parse_version(latest);
    let c = parse_version(current);
    let len = l.len().max(c.len());
    for i in 0..len {
        let a = l.get(i).copied().unwrap_or(0);
        let b = c.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

/// Returns update info when the newest GitHub release is ahead of the running
/// version; `None` means up to date (or no releases yet / network hiccup —
/// both intentionally silent: an update check should never bother the user
/// with errors).
#[tauri::command]
pub async fn check_for_update() -> Result<Option<UpdateInfo>, String> {
    let current = env!("CARGO_PKG_VERSION");
    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);

    let client = Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = match client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return Ok(None), // offline — stay quiet
    };
    if !resp.status().is_success() {
        return Ok(None); // 404 = no releases yet, etc.
    }
    let release: GhRelease = match resp.json().await {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    if release.draft || release.prerelease || !is_newer(&release.tag_name, current) {
        return Ok(None);
    }

    Ok(Some(UpdateInfo {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        url: release.html_url,
        notes: {
            let n = release.body.unwrap_or_default();
            if n.is_empty() { release.name } else { n }
        },
    }))
}
