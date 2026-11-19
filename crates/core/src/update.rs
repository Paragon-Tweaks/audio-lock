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

pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64, Option<String>)> {
    let trimmed = raw.trim();
    let without_v = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    let without_build = without_v.split('+').next()?;
    let (core, pre) = match without_build.split_once('-') {
        Some((core, pre)) => {
            if pre.is_empty()
                || pre.split('.').any(|id| {
                    id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
            {
                return None;
            }
            (core, Some(pre.to_string()))
        }
        None => (without_build, None),
    };

    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, pre))
}

pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    let (Some(cur), Some(cand)) = (parse_semver(current), parse_semver(candidate)) else {
        return false;
    };
    compare_semver(&cand, &cur).is_gt()
}

pub fn is_valid_version_dir_name(raw: &str) -> bool {
    !raw.is_empty()
        && !raw.contains("..")
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && parse_semver(raw).is_some()
}

fn compare_semver(
    a: &(u64, u64, u64, Option<String>),
    b: &(u64, u64, u64, Option<String>),
) -> std::cmp::Ordering {
    (a.0, a.1, a.2)
        .cmp(&(b.0, b.1, b.2))
        .then_with(|| compare_prerelease(a.3.as_deref(), b.3.as_deref()))
}

fn compare_prerelease(a: Option<&str>, b: Option<&str>) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(pa), Some(pb)) => {
            let mut ia = pa.split('.');
            let mut ib = pb.split('.');
            loop {
                match (ia.next(), ib.next()) {
                    (None, None) => return Ordering::Equal,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some(xa), Some(xb)) => {
                        let ord = compare_prerelease_ident(xa, xb);
                        if ord != Ordering::Equal {
                            return ord;
                        }
                    }
                }
            }
        }
    }
}

fn compare_prerelease_ident(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(na), Ok(nb)) => na.cmp(&nb),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => a.cmp(b),
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
