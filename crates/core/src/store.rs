use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::fake_data::default_mock_state;
use crate::model::AppState;

pub fn state_file_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("default-audio-lock").join("state.json")
}

pub fn load_state() -> AppState {
    load_state_from(&state_file_path())
}

pub fn save_state(state: &AppState) -> io::Result<()> {
    save_state_to(&state_file_path(), state)
}

pub fn reset_to_mock_state() -> io::Result<AppState> {
    let state = default_mock_state();
    save_state(&state)?;
    Ok(state)
}

pub fn load_state_from(path: &Path) -> AppState {
    let Ok(bytes) = fs::read(path) else {
        return default_mock_state();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|_| default_mock_state())
}

pub fn save_state_to(path: &Path, state: &AppState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(state)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(&tmp_path, json)?;

    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp_path, path)
}
