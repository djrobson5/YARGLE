//! Lightweight update check against GitHub Releases.
//!
//! Deliberately notify-only: we compare the latest release tag against the
//! running version and let the user open the release page in their browser —
//! no auto-download/install (which would need signing keys and updater
//! infrastructure for little gain at this project's scale).

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use tauri::{AppHandle, Emitter};

const REPO: &str = "djrobson5/YARGLE";
const USER_AGENT: &str = concat!("YARGLE/", env!("CARGO_PKG_VERSION"), " (update check)");

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
    pub notes: String,
    // Direct download URL for the release's yargle.exe asset, if present. None
    // when the release ships no exe (then only "View release" is offered).
    pub download_url: Option<String>,
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
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
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

    // Prefer an asset literally named yargle.exe, else the first .exe asset.
    let download_url = release
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("yargle.exe"))
        .or_else(|| {
            release
                .assets
                .iter()
                .find(|a| a.name.to_ascii_lowercase().ends_with(".exe"))
        })
        .map(|a| a.browser_download_url.clone())
        .filter(|u| u.starts_with("http"));

    Ok(Some(UpdateInfo {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        url: release.html_url,
        notes: {
            let n = release.body.unwrap_or_default();
            if n.is_empty() { release.name } else { n }
        },
        download_url,
    }))
}

fn emit_update_progress(app: &AppHandle, phase: &str, received: u64, total: u64) {
    let _ = app.emit(
        "update-progress",
        serde_json::json!({ "phase": phase, "received": received, "total": total }),
    );
}

/// Download the given release exe and swap it in for the running one, then
/// relaunch. Emits `update-progress` events (phases: starting → downloading →
/// installing → restarting). On any failure it returns an error WITHOUT
/// touching the running app, so a bad download never leaves things broken.
///
/// Windows can't overwrite a running exe, so `self_replace` renames the running
/// one aside and drops the new one into place; the current process keeps running
/// from memory until we relaunch the freshly-installed binary and exit.
#[tauri::command]
pub async fn download_and_apply_update(app: AppHandle, url: String) -> Result<(), String> {
    let current = std::env::current_exe()
        .map_err(|e| format!("Cannot locate the running executable: {}", e))?;
    let dir = current
        .parent()
        .ok_or("Cannot determine the application directory")?
        .to_path_buf();
    // Download beside the current exe (same volume) for a clean swap.
    let tmp = dir.join("yargle-update.download");

    emit_update_progress(&app, "starting", 0, 0);

    let client = Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Download failed: HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);

    let mut file = std::fs::File::create(&tmp)
        .map_err(|e| format!("Cannot write to {}: {}", dir.display(), e))?;
    let mut received: u64 = 0;
    let mut last_emit: u64 = 0;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("Download interrupted: {}", e))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("Write failed: {}", e))?;
        received += chunk.len() as u64;
        if received - last_emit >= 512 * 1024 || (total > 0 && received >= total) {
            last_emit = received;
            emit_update_progress(&app, "downloading", received, total);
        }
    }
    drop(file);

    // Sanity check: a real Windows executable starts with "MZ". Guards against
    // saving an HTML error page (captive portal, etc.) as the app.
    let mut magic = [0u8; 2];
    let ok_magic = std::fs::File::open(&tmp)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && &magic == b"MZ";
    if !ok_magic {
        let _ = std::fs::remove_file(&tmp);
        return Err("The downloaded file doesn't look like a valid application — update aborted.".into());
    }

    emit_update_progress(&app, "installing", total, total);
    self_replace::self_replace(&tmp).map_err(|e| format!("Failed to apply update: {}", e))?;
    let _ = std::fs::remove_file(&tmp);

    emit_update_progress(&app, "restarting", total, total);
    std::process::Command::new(&current)
        .spawn()
        .map_err(|e| format!("Update installed, but relaunch failed ({}). Please reopen YARGLE.", e))?;
    // Give the UI a moment to show "restarting" before this process vanishes.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    std::process::exit(0);
}
