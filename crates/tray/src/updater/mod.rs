pub mod apply;
pub mod http;
pub mod verify;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use audio_lock_core::model::AppState;
use audio_lock_core::store;
use audio_lock_core::update::{
    CURRENT_VERSION, UpdateManifest, is_newer_version, manifest_url, staging_root_dir,
};

use crate::win32::{self, WM_APP_UPDATE_STAGED, WakeHandle};

pub const TIMER_UPDATE_INITIAL: usize = 1;
pub const TIMER_UPDATE_PERIODIC: usize = 2;
pub const TIMER_IDLE_CHECK: usize = 3;

pub const INITIAL_CHECK_DELAY_MS: u32 = 60 * 1000;
pub const PERIODIC_CHECK_INTERVAL_MS: u32 = 6 * 60 * 60 * 1000;
pub const IDLE_CHECK_INTERVAL_MS: u32 = 30 * 1000;

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_ZIP_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct StagedUpdate {
    pub version: String,
    pub dir: PathBuf,
}

pub struct Updater {
    state: Arc<Mutex<AppState>>,
    wake: WakeHandle,
    checking_in_progress: Arc<AtomicBool>,
    staged: Arc<Mutex<Option<StagedUpdate>>>,
}

struct CheckingGuard(Arc<AtomicBool>);

impl Drop for CheckingGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Updater {
    pub fn new(state: Arc<Mutex<AppState>>, wake: WakeHandle) -> Self {
        apply::cleanup_old_binaries();

        let restored = apply::find_existing_staged_update().map(|(version, dir)| StagedUpdate {
            version,
            dir,
        });
        let restored_version = restored.as_ref().map(|u| u.version.clone());

        {
            let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
            if guard.staged_update_version != restored_version {
                guard.staged_update_version = restored_version;
                let _ = store::save_state(&guard);
            }
        }

        Self {
            state,
            wake,
            checking_in_progress: Arc::new(AtomicBool::new(false)),
            staged: Arc::new(Mutex::new(restored)),
        }
    }

    pub fn has_staged(&self) -> bool {
        self.staged
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    pub fn spawn_check(&self, force: bool) {
        if !force {
            let auto_update = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .auto_update;
            if !auto_update {
                return;
            }
        }

        let Some(url) = manifest_url() else {
            return;
        };

        if self
            .checking_in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let state = Arc::clone(&self.state);
        let staged = Arc::clone(&self.staged);
        let checking = Arc::clone(&self.checking_in_progress);
        let wake = self.wake;

        let spawned = thread::Builder::new()
            .name("updater-check".into())
            .spawn(move || {
                let _guard = CheckingGuard(checking);
                win32::set_current_thread_idle_priority();
                run_check(&url, &state, &staged, wake);
                win32::trim_working_set();
            });

        if spawned.is_err() {
            self.checking_in_progress.store(false, Ordering::Release);
        }
    }

    pub fn try_apply_staged(&self) -> bool {
        let Some(staged) = self
            .staged
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            return false;
        };

        match apply::apply_staged_and_spawn(&staged.dir) {
            Ok(()) => true,
            Err(_) => {
                if !apply::has_staged_binaries(&staged.dir) {
                    *self.staged.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    if guard.staged_update_version.is_some() {
                        guard.staged_update_version = None;
                        let _ = store::save_state(&guard);
                    }
                }
                false
            }
        }
    }
}

fn run_check(
    manifest_url: &str,
    state: &Arc<Mutex<AppState>>,
    staged: &Arc<Mutex<Option<StagedUpdate>>>,
    wake: WakeHandle,
) {
    let Ok(bytes) = http::fetch_bytes(manifest_url, MAX_MANIFEST_BYTES) else {
        return;
    };
    let Ok(manifest) = serde_json::from_slice::<UpdateManifest>(&bytes) else {
        return;
    };

    let Ok(version) = apply::validate_version_dir_name(&manifest.version) else {
        return;
    };
    if !is_newer_version(CURRENT_VERSION, version) {
        return;
    }

    let already_staged = {
        let guard = staged.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().is_some_and(|existing| {
            !is_newer_version(&existing.version, version) && apply::has_staged_binaries(&existing.dir)
        })
    };
    if already_staged {
        wake.post(WM_APP_UPDATE_STAGED);
        return;
    }

    let zip_path = staging_root_dir().join(format!("download-{version}.zip.tmp"));
    if http::download_to_file(&manifest.asset_url, &zip_path, MAX_ZIP_BYTES).is_err() {
        return;
    }

    if verify::verify_sha256(&zip_path, &manifest.sha256).is_err() {
        let _ = fs::remove_file(&zip_path);
        return;
    }

    let Ok(staged_dir) = apply::stage_update_zip(&zip_path, version) else {
        let _ = fs::remove_file(&zip_path);
        return;
    };

    *staged.lock().unwrap_or_else(|e| e.into_inner()) = Some(StagedUpdate {
        version: version.to_string(),
        dir: staged_dir,
    });

    {
        let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
        guard.staged_update_version = Some(version.to_string());
        let _ = store::save_state(&guard);
    }

    wake.post(WM_APP_UPDATE_STAGED);
}
