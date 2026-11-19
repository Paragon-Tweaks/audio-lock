use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const IDLE_RESTART_MS: u32 = 5 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    #[serde(default)]
    pub published_at: Option<String>,
    pub sha256: String,
    pub asset_url: String,
}

pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim();
    let without_v = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    let core = without_v.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    match (parse_semver(current), parse_semver(candidate)) {
        (Some(cur), Some(cand)) => cand > cur,
        _ => false,
    }
}

pub fn manifest_url() -> Option<String> {
    if let Some(url) = non_empty_env("AUDIO_LOCK_UPDATE_MANIFEST_URL") {
        return Some(url);
    }
    if let Some(url) = option_env!("AUDIO_LOCK_UPDATE_MANIFEST_URL")
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(url.to_string());
    }

    let repo = non_empty_env("AUDIO_LOCK_GITHUB_REPO").or_else(|| {
        option_env!("AUDIO_LOCK_GITHUB_REPO")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });

    if let Some(repo) = repo {
        let repo = repo.trim_matches('/');
        if !repo.is_empty() && !repo.starts_with("OWNER/") {
            return Some(format!(
                "https://github.com/{repo}/releases/latest/download/manifest.json"
            ));
        }
    }

    None
}

pub fn staging_root_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("default-audio-lock").join("staging")
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
