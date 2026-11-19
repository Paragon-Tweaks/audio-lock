use std::fs::{self, File};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use audio_lock_core::update::{
    CURRENT_VERSION, is_newer_version, parse_semver, staging_root_dir,
};
use windows::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MoveFileExW};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::core::PCWSTR;
use zip::ZipArchive;

use crate::updater::verify;

pub const TRAY_EXE_NAME: &str = "default-audio-lock.exe";
const LEGACY_UI_EXE_NAME: &str = "default-audio-lock-ui.exe";

const OLD_BINARY_DELETE_ATTEMPTS: usize = 5;
const OLD_BINARY_DELETE_RETRY_DELAY: Duration = Duration::from_millis(50);

pub fn has_staged_binaries(dir: &Path) -> bool {
    is_non_empty_file(&dir.join(TRAY_EXE_NAME))
}

pub fn cleanup_old_binaries() {
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(install_dir) = current_exe.parent()
    {
        for name in [TRAY_EXE_NAME, LEGACY_UI_EXE_NAME] {
            remove_file_best_effort(&install_dir.join(format!("{name}.old")));
        }
    }

    let staging_root = staging_root_dir();
    let Ok(entries) = fs::read_dir(&staging_root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };

        if path.is_dir() {
            if parse_semver(name_str).is_some() && !is_newer_version(CURRENT_VERSION, name_str) {
                let _ = fs::remove_dir_all(&path);
            }
        } else if name_str.ends_with(".zip.tmp") {
            let _ = fs::remove_file(&path);
        }
    }
}

pub fn find_existing_staged_update() -> Option<(String, PathBuf)> {
    let entries = fs::read_dir(staging_root_dir()).ok()?;
    let mut best: Option<((u64, u64, u64), String, PathBuf)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(ver) = parse_semver(&name) else {
            continue;
        };
        if !is_newer_version(CURRENT_VERSION, &name) || !has_staged_binaries(&path) {
            continue;
        }
        if best.as_ref().is_none_or(|(best_ver, _, _)| ver > *best_ver) {
            best = Some((ver, name, path));
        }
    }

    best.map(|(_, version, dir)| (version, dir))
}

pub fn stage_update_zip(zip_path: &Path, version: &str) -> Result<PathBuf, String> {
    let version = validate_version_dir_name(version)?;
    let staged_dir = staging_root_dir().join(version);

    let _ = fs::remove_dir_all(&staged_dir);
    fs::create_dir_all(&staged_dir)
        .map_err(|err| format!("Failed to create staging directory: {err}"))?;

    let extract_result = (|| -> Result<(), String> {
        let file = File::open(zip_path)
            .map_err(|err| format!("Failed to open update archive: {err}"))?;
        let mut archive = ZipArchive::new(file)
            .map_err(|err| format!("Failed to read update archive: {err}"))?;

        for idx in 0..archive.len() {
            let mut entry = archive
                .by_index(idx)
                .map_err(|err| format!("Failed to read zip entry {idx}: {err}"))?;
            if entry.is_dir() {
                continue;
            }
            let Some(enclosed) = entry.enclosed_name() else {
                continue;
            };
            let Some(file_name) = enclosed.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            if !file_name.eq_ignore_ascii_case(TRAY_EXE_NAME) {
                continue;
            }

            let dest_path = staged_dir.join(TRAY_EXE_NAME);
            let mut out = File::create(&dest_path)
                .map_err(|err| format!("Failed to create {TRAY_EXE_NAME}: {err}"))?;
            io::copy(&mut entry, &mut out)
                .map_err(|err| format!("Failed to extract {TRAY_EXE_NAME}: {err}"))?;
        }

        let staged_tray = staged_dir.join(TRAY_EXE_NAME);
        if !is_non_empty_file(&staged_tray) {
            return Err(format!("Update archive missing {TRAY_EXE_NAME}"));
        }

        verify::verify_authenticode_if_required(&staged_tray)?;
        Ok(())
    })();

    if let Err(err) = extract_result {
        let _ = fs::remove_dir_all(&staged_dir);
        return Err(err);
    }

    let _ = fs::remove_file(zip_path);
    Ok(staged_dir)
}

pub fn apply_staged_and_spawn(staged_dir: &Path) -> Result<(), String> {
    let staged_tray = staged_dir.join(TRAY_EXE_NAME);
    if !is_non_empty_file(&staged_tray) {
        return Err("Staged update binary is missing".to_string());
    }

    let current_exe = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve current executable: {err}"))?;
    let install_dir = current_exe
        .parent()
        .ok_or_else(|| "Current executable has no parent directory".to_string())?;

    let cur_tray = install_dir.join(TRAY_EXE_NAME);
    let old_tray = install_dir.join(format!("{TRAY_EXE_NAME}.old"));

    move_file_replace(&cur_tray, &old_tray)
        .map_err(|err| format!("Failed to stage current binary aside: {err}"))?;

    let rollback = || {
        let _ = fs::remove_file(&cur_tray);
        let _ = move_file_replace(&old_tray, &cur_tray);
    };

    if let Err(err) = fs::copy(&staged_tray, &cur_tray) {
        rollback();
        return Err(format!("Failed to copy new binary: {err}"));
    }

    if let Err(err) = Command::new(&cur_tray)
        .arg("--post-update")
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
    {
        rollback();
        return Err(format!("Failed to spawn updated binary: {err}"));
    }

    Ok(())
}

pub fn validate_version_dir_name(version: &str) -> Result<&str, String> {
    let trimmed = version.trim();
    if parse_semver(trimmed).is_none() {
        return Err(format!("Invalid version string: {version}"));
    }
    let mut components = Path::new(trimmed).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(trimmed),
        _ => Err(format!("Unsafe version directory name: {version}")),
    }
}

fn move_file_replace(from: &Path, to: &Path) -> Result<(), String> {
    let from_wide: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(from_wide.as_ptr()),
            PCWSTR(to_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING,
        )
    }
    .map_err(|err| format!("{err}"))
}

fn is_non_empty_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

fn remove_file_best_effort(path: &Path) {
    for attempt in 0..OLD_BINARY_DELETE_ATTEMPTS {
        match fs::remove_file(path) {
            Ok(()) => return,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return,
            Err(_) if attempt + 1 < OLD_BINARY_DELETE_ATTEMPTS => {
                thread::sleep(OLD_BINARY_DELETE_RETRY_DELAY);
            }
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn stages_zip_safely_and_ignores_zip_slip() {
        let zip_path = std::env::temp_dir().join(format!(
            "audio-lock-stage-test-{}.zip",
            std::process::id()
        ));
        {
            let file = File::create(&zip_path).expect("create zip");
            let mut writer = ZipWriter::new(file);
            let opts = SimpleFileOptions::default();

            writer.start_file("../evil.exe", opts).expect("start evil");
            writer.write_all(b"evil").expect("write evil");

            writer
                .start_file(format!("release/{TRAY_EXE_NAME}"), opts)
                .expect("start tray");
            writer.write_all(b"tray-bin").expect("write tray");

            writer.finish().expect("finish zip");
        }

        let version = format!("999.0.{}-test", std::process::id());
        let staged_dir = stage_update_zip(&zip_path, &version).expect("stage zip");

        assert!(!zip_path.exists());
        assert_eq!(fs::read(staged_dir.join(TRAY_EXE_NAME)).unwrap(), b"tray-bin");
        assert!(!staging_root_dir().join("evil.exe").exists());

        let _ = fs::remove_dir_all(&staged_dir);
    }

    #[test]
    fn rejects_unsafe_version_dir_name() {
        assert!(validate_version_dir_name("../1.2.3").is_err());
        assert!(validate_version_dir_name("1.2.3/../../x").is_err());
        assert!(validate_version_dir_name("1.2.3").is_ok());
    }
}
