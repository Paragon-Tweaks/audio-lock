use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use audio_lock_core::update::{
    CURRENT_VERSION, is_newer_version, is_valid_version_dir_name, parse_semver, staging_root_dir,
};
use windows::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MoveFileExW};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::core::PCWSTR;
use zip::ZipArchive;

use crate::updater::verify;

pub const TRAY_EXE_NAME: &str = "default-audio-lock.exe";
const LEGACY_UI_EXE_NAME: &str = "default-audio-lock-ui.exe";
const STAGING_SHA256_FILE: &str = "staging_sha256.txt";
const TMP_STAGING_PREFIX: &str = ".tmp-";

const MAX_ZIP_ENTRIES: usize = 32;
const MAX_EXTRACTED_EXE_BYTES: u64 = 32 * 1024 * 1024;

const OLD_BINARY_DELETE_ATTEMPTS: usize = 5;
const OLD_BINARY_DELETE_RETRY_DELAY: Duration = Duration::from_millis(50);

pub fn has_staged_binaries(dir: &Path) -> bool {
    is_non_empty_file(&dir.join(TRAY_EXE_NAME))
        && is_non_empty_file(&dir.join(STAGING_SHA256_FILE))
}

pub fn verify_staged_binary(staged_dir: &Path) -> Result<PathBuf, String> {
    let staged_tray = staged_dir.join(TRAY_EXE_NAME);
    if !is_non_empty_file(&staged_tray) {
        return Err("Staged update binary is missing".to_string());
    }
    let expected_hex = fs::read_to_string(staged_dir.join(STAGING_SHA256_FILE))
        .map_err(|err| format!("Failed to read staging hash marker: {err}"))?;
    verify::verify_sha256(&staged_tray, &expected_hex)?;
    Ok(staged_tray)
}

pub fn cleanup_old_binaries() {
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(install_dir) = current_exe.parent()
    {
        if let Some(cur_name) = current_exe.file_name() {
            let mut old_name = cur_name.to_os_string();
            old_name.push(".old");
            remove_file_best_effort(&install_dir.join(old_name));

            let mut new_name = cur_name.to_os_string();
            new_name.push(".new");
            remove_file_best_effort(&install_dir.join(new_name));
        }

        for name in [TRAY_EXE_NAME, LEGACY_UI_EXE_NAME] {
            remove_file_best_effort(&install_dir.join(format!("{name}.old")));
            remove_file_best_effort(&install_dir.join(format!("{name}.new")));
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
            if name_str.starts_with(TMP_STAGING_PREFIX)
                || (parse_semver(name_str).is_some()
                    && !is_newer_version(CURRENT_VERSION, name_str))
            {
                let _ = fs::remove_dir_all(&path);
            }
        } else if name_str.ends_with(".zip.tmp") {
            let _ = fs::remove_file(&path);
        }
    }
}

pub fn find_existing_staged_update() -> Option<(String, PathBuf)> {
    let entries = fs::read_dir(staging_root_dir()).ok()?;
    let mut best: Option<(String, PathBuf)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !is_valid_version_dir_name(&name) {
            continue;
        }
        if !is_newer_version(CURRENT_VERSION, &name) || !has_staged_binaries(&path) {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(best_version, _)| is_newer_version(best_version, &name))
        {
            best = Some((name, path));
        }
    }

    best
}

pub fn stage_update_zip(zip_path: &Path, version: &str) -> Result<PathBuf, String> {
    let version = validate_version_dir_name(version)?;
    let staging_root = staging_root_dir();
    let tmp_dir = staging_root.join(format!("{TMP_STAGING_PREFIX}{version}"));
    let final_dir = staging_root.join(version);

    let _ = fs::remove_dir_all(&tmp_dir);
    fs::create_dir_all(&tmp_dir)
        .map_err(|err| format!("Failed to create temporary staging directory: {err}"))?;

    let extract_result = (|| -> Result<(), String> {
        let file = File::open(zip_path)
            .map_err(|err| format!("Failed to open update archive: {err}"))?;
        let mut archive = ZipArchive::new(file)
            .map_err(|err| format!("Failed to read update archive: {err}"))?;

        if archive.len() > MAX_ZIP_ENTRIES {
            return Err(format!(
                "Update archive exceeds maximum entry count ({} > {MAX_ZIP_ENTRIES})",
                archive.len()
            ));
        }

        let mut found_tray = false;

        for idx in 0..archive.len() {
            let entry = archive
                .by_index(idx)
                .map_err(|err| format!("Failed to read zip entry {idx}: {err}"))?;
            let Some(enclosed) = entry.enclosed_name() else {
                continue;
            };
            let Some(file_name) = enclosed.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            if !file_name.eq_ignore_ascii_case(TRAY_EXE_NAME) {
                continue;
            }

            if !entry.is_file() {
                return Err(format!(
                    "Archive entry for {TRAY_EXE_NAME} is not a regular file"
                ));
            }
            if found_tray {
                return Err(format!(
                    "Update archive contains duplicate {TRAY_EXE_NAME} entries"
                ));
            }
            found_tray = true;

            let dest_path = tmp_dir.join(TRAY_EXE_NAME);
            let mut out = File::create(&dest_path)
                .map_err(|err| format!("Failed to create {TRAY_EXE_NAME}: {err}"))?;
            let mut limited = entry.take(MAX_EXTRACTED_EXE_BYTES + 1);
            let copied = io::copy(&mut limited, &mut out)
                .map_err(|err| format!("Failed to extract {TRAY_EXE_NAME}: {err}"))?;
            if copied > MAX_EXTRACTED_EXE_BYTES {
                return Err(format!(
                    "Extracted {TRAY_EXE_NAME} exceeds size limit of {MAX_EXTRACTED_EXE_BYTES} bytes"
                ));
            }
            out.flush()
                .map_err(|err| format!("Failed to flush {TRAY_EXE_NAME}: {err}"))?;
        }

        let staged_tray = tmp_dir.join(TRAY_EXE_NAME);
        if !found_tray || !is_non_empty_file(&staged_tray) {
            return Err(format!("Update archive missing {TRAY_EXE_NAME}"));
        }

        verify::verify_authenticode_if_required(&staged_tray)?;

        let staged_hash = verify::sha256_file_hex(&staged_tray)?;
        fs::write(tmp_dir.join(STAGING_SHA256_FILE), staged_hash.as_bytes())
            .map_err(|err| format!("Failed to write {STAGING_SHA256_FILE}: {err}"))?;

        Ok(())
    })();

    if let Err(err) = extract_result {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err(err);
    }

    if final_dir.exists()
        && let Err(err) = fs::remove_dir_all(&final_dir)
    {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err(format!("Failed to clear existing staging directory: {err}"));
    }

    if let Err(err) = fs::rename(&tmp_dir, &final_dir) {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err(format!("Failed to finalize staging directory: {err}"));
    }

    let _ = fs::remove_file(zip_path);
    Ok(final_dir)
}

pub fn apply_staged_and_spawn(staged_dir: &Path) -> Result<(), String> {
    let staged_tray = verify_staged_binary(staged_dir)?;

    let current_exe = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve current executable: {err}"))?;
    let install_dir = current_exe
        .parent()
        .ok_or_else(|| "Current executable has no parent directory".to_string())?;
    let cur_file_name = current_exe
        .file_name()
        .ok_or_else(|| "Current executable has no file name".to_string())?;

    let mut new_name = cur_file_name.to_os_string();
    new_name.push(".new");
    let new_tray = install_dir.join(new_name);

    let mut old_name = cur_file_name.to_os_string();
    old_name.push(".old");
    let old_tray = install_dir.join(old_name);

    let cur_tray = current_exe;

    if let Err(err) = fs::copy(&staged_tray, &new_tray) {
        let _ = fs::remove_file(&new_tray);
        return Err(format!(
            "Failed to copy staged binary to install directory: {err}"
        ));
    }

    if let Err(err) = move_file_replace(&cur_tray, &old_tray) {
        let _ = fs::remove_file(&new_tray);
        return Err(format!("Failed to stage current binary aside: {err}"));
    }

    if let Err(err) = move_file_replace(&new_tray, &cur_tray) {
        let _ = move_file_replace(&old_tray, &cur_tray);
        let _ = fs::remove_file(&new_tray);
        return Err(format!("Failed to place updated binary: {err}"));
    }

    if let Err(err) = Command::new(&cur_tray)
        .arg("--post-update")
        .arg(format!("--wait-pid={}", std::process::id()))
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
    {
        let _ = fs::remove_file(&cur_tray);
        let _ = move_file_replace(&old_tray, &cur_tray);
        return Err(format!("Failed to spawn updated binary: {err}"));
    }

    Ok(())
}

pub fn validate_version_dir_name(version: &str) -> Result<&str, String> {
    let trimmed = version.trim();
    if !is_valid_version_dir_name(trimmed) {
        return Err(format!("Unsafe or invalid version directory name: {version}"));
    }
    Ok(trimmed)
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
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn stages_zip_safely_ignores_zip_slip_and_detects_tampering() {
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

        let version = format!("999.0.{}-stage", std::process::id());
        let staged_dir = stage_update_zip(&zip_path, &version).expect("stage zip");

        assert!(!zip_path.exists());
        assert!(has_staged_binaries(&staged_dir));
        assert!(!staging_root_dir().join(format!("{TMP_STAGING_PREFIX}{version}")).exists());
        assert_eq!(fs::read(staged_dir.join(TRAY_EXE_NAME)).unwrap(), b"tray-bin");
        assert!(!staging_root_dir().join("evil.exe").exists());
        assert!(verify_staged_binary(&staged_dir).is_ok());

        // Tampering with the staged binary must fail pre-swap SHA-256 verification.
        fs::write(staged_dir.join(TRAY_EXE_NAME), b"tampered-bin").expect("tamper binary");
        assert!(verify_staged_binary(&staged_dir).is_err());
        assert!(apply_staged_and_spawn(&staged_dir).is_err());

        let _ = fs::remove_dir_all(&staged_dir);
    }

    #[test]
    fn rejects_duplicate_exe_entries() {
        let zip_path = std::env::temp_dir().join(format!(
            "audio-lock-dup-test-{}.zip",
            std::process::id()
        ));
        {
            let file = File::create(&zip_path).expect("create zip");
            let mut writer = ZipWriter::new(file);
            let opts = SimpleFileOptions::default();

            writer
                .start_file(format!("a/{TRAY_EXE_NAME}"), opts)
                .expect("start first");
            writer.write_all(b"first").expect("write first");

            writer
                .start_file(format!("b/{TRAY_EXE_NAME}"), opts)
                .expect("start second");
            writer.write_all(b"second").expect("write second");

            writer.finish().expect("finish zip");
        }

        let version = format!("999.0.{}-dup", std::process::id());
        let err = stage_update_zip(&zip_path, &version).unwrap_err();
        assert!(err.contains("duplicate"));
        assert!(!staging_root_dir().join(&version).exists());
        assert!(!staging_root_dir().join(format!("{TMP_STAGING_PREFIX}{version}")).exists());
        let _ = fs::remove_file(&zip_path);
    }

    #[test]
    fn rejects_oversized_exe_entry() {
        let zip_path = std::env::temp_dir().join(format!(
            "audio-lock-oversize-test-{}.zip",
            std::process::id()
        ));
        {
            let file = File::create(&zip_path).expect("create zip");
            let mut writer = ZipWriter::new(file);
            let opts = SimpleFileOptions::default();

            writer.start_file(TRAY_EXE_NAME, opts).expect("start big");
            let chunk = vec![0u8; 64 * 1024];
            let mut remaining = MAX_EXTRACTED_EXE_BYTES + 1;
            while remaining > 0 {
                let to_write = remaining.min(chunk.len() as u64) as usize;
                writer.write_all(&chunk[..to_write]).expect("write chunk");
                remaining -= to_write as u64;
            }

            writer.finish().expect("finish zip");
        }

        let version = format!("999.0.{}-big", std::process::id());
        let err = stage_update_zip(&zip_path, &version).unwrap_err();
        assert!(err.contains("size limit"));
        assert!(!staging_root_dir().join(&version).exists());
        assert!(!staging_root_dir().join(format!("{TMP_STAGING_PREFIX}{version}")).exists());
        let _ = fs::remove_file(&zip_path);
    }

    #[test]
    fn rejects_too_many_zip_entries() {
        let zip_path = std::env::temp_dir().join(format!(
            "audio-lock-many-test-{}.zip",
            std::process::id()
        ));
        {
            let file = File::create(&zip_path).expect("create zip");
            let mut writer = ZipWriter::new(file);
            let opts = SimpleFileOptions::default();

            for i in 0..=MAX_ZIP_ENTRIES {
                writer
                    .start_file(format!("file-{i}.txt"), opts)
                    .expect("start file");
                writer.write_all(b"x").expect("write file");
            }

            writer.finish().expect("finish zip");
        }

        let version = format!("999.0.{}-many", std::process::id());
        let err = stage_update_zip(&zip_path, &version).unwrap_err();
        assert!(err.contains("maximum entry count"));
        let _ = fs::remove_file(&zip_path);
    }

    #[test]
    fn rejects_unsafe_version_dir_name() {
        assert!(validate_version_dir_name("../1.2.3").is_err());
        assert!(validate_version_dir_name("1.2.3/../../x").is_err());
        assert!(validate_version_dir_name("1.2.3").is_ok());
    }
}
