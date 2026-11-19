use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use audio_lock_core::ipc::{AppAction, IpcRequest, IpcResponse, PIPE_NAME};
use audio_lock_core::model::AppState;
use audio_lock_core::store;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::core::PCWSTR;

use crate::audio::AudioEngine;
use crate::win32::{
    self, WM_APP_APPLY_UPDATE, WM_APP_CHECK_UPDATES, WM_APP_OPEN_FLYOUT, WM_APP_QUIT,
    WM_APP_REFRESH_TRAY, WakeHandle,
};

const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

pub struct IpcContext {
    pub state: Arc<Mutex<AppState>>,
    pub audio: Arc<AudioEngine>,
    pub last_anchor: Arc<Mutex<(i32, i32)>>,
    pub refresh_pending: Arc<AtomicBool>,
    pub open_flyout_pending: Arc<AtomicBool>,
    pub quit_pending: Arc<AtomicBool>,
    pub check_updates_pending: Arc<AtomicBool>,
    pub apply_update_pending: Arc<AtomicBool>,
    pub wake: WakeHandle,
}

enum IpcAction {
    None,
    RefreshTray,
    OpenFlyout,
    Quit,
    CheckUpdates,
    ApplyUpdate,
}

pub fn spawn_server(ctx: IpcContext) {
    let _ = thread::Builder::new()
        .name("ipc-server".into())
        .spawn(move || run_server(ctx));
}

fn run_server(ctx: IpcContext) {
    let pipe_name = win32::to_wide(PIPE_NAME);

    loop {
        let pipe = unsafe {
            CreateNamedPipeW(
                PCWSTR(pipe_name.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                PIPE_BUFFER_SIZE,
                PIPE_BUFFER_SIZE,
                0,
                None,
            )
        };

        if pipe.is_invalid() || pipe == INVALID_HANDLE_VALUE {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let connected = unsafe {
            match ConnectNamedPipe(pipe, None) {
                Ok(()) => true,
                Err(err) if err.code() == ERROR_PIPE_CONNECTED.to_hresult() => true,
                Err(_) => false,
            }
        };

        if connected {
            let action = handle_client(pipe, &ctx);
            unsafe {
                let _ = FlushFileBuffers(pipe);
                let _ = DisconnectNamedPipe(pipe);
            }
            match action {
                IpcAction::None => {}
                IpcAction::RefreshTray => {
                    ctx.refresh_pending.store(true, Ordering::Release);
                    ctx.wake.post(WM_APP_REFRESH_TRAY);
                    win32::trim_working_set();
                }
                IpcAction::OpenFlyout => {
                    ctx.open_flyout_pending.store(true, Ordering::Release);
                    ctx.wake.post(WM_APP_OPEN_FLYOUT);
                }
                IpcAction::Quit => {
                    ctx.quit_pending.store(true, Ordering::Release);
                    ctx.wake.post(WM_APP_QUIT);
                }
                IpcAction::CheckUpdates => {
                    ctx.check_updates_pending.store(true, Ordering::Release);
                    ctx.wake.post(WM_APP_CHECK_UPDATES);
                }
                IpcAction::ApplyUpdate => {
                    ctx.apply_update_pending.store(true, Ordering::Release);
                    ctx.wake.post(WM_APP_APPLY_UPDATE);
                }
            }
        }

        unsafe {
            let _ = CloseHandle(pipe);
        }
    }
}

fn handle_client(pipe: HANDLE, ctx: &IpcContext) -> IpcAction {
    let Some(line) = read_line(pipe) else {
        return IpcAction::None;
    };

    let mut action = IpcAction::None;
    let response = match serde_json::from_slice::<IpcRequest>(&line) {
        Ok(IpcRequest::GetState) => {
            let snapshot = ctx.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
            IpcResponse::State(snapshot)
        }
        Ok(IpcRequest::ApplyAction(AppAction::CheckForUpdatesNow)) => {
            action = IpcAction::CheckUpdates;
            let snapshot = ctx.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
            IpcResponse::State(snapshot)
        }
        Ok(IpcRequest::ApplyAction(AppAction::ApplyStagedUpdateNow)) => {
            action = IpcAction::ApplyUpdate;
            let snapshot = ctx.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
            IpcResponse::State(snapshot)
        }
        Ok(IpcRequest::ApplyAction(app_action)) => {
            let updated = ctx.audio.apply_action(app_action);
            IpcResponse::State(updated)
        }
        Ok(IpcRequest::SyncState(new_state)) => {
            let save_result = store::save_state(&new_state);
            *ctx.state.lock().unwrap_or_else(|e| e.into_inner()) = new_state;
            action = IpcAction::RefreshTray;
            match save_result {
                Ok(()) => IpcResponse::Ok,
                Err(err) => IpcResponse::Error(format!("Failed to save state: {err}")),
            }
        }
        Ok(IpcRequest::OpenFlyout { anchor_x, anchor_y }) => {
            if anchor_x.is_some() || anchor_y.is_some() {
                let mut anchor = ctx.last_anchor.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(x) = anchor_x {
                    anchor.0 = x;
                }
                if let Some(y) = anchor_y {
                    anchor.1 = y;
                }
            }
            action = IpcAction::OpenFlyout;
            IpcResponse::Ok
        }
        Ok(IpcRequest::QuitDaemon) => {
            action = IpcAction::Quit;
            IpcResponse::Ok
        }
        Ok(IpcRequest::Ping) => IpcResponse::Ok,
        Err(err) => IpcResponse::Error(format!("Invalid IPC request: {err}")),
    };

    if let Ok(mut payload) = serde_json::to_vec(&response) {
        payload.push(b'\n');
        write_all(pipe, &payload);
    }

    action
}

fn read_line(pipe: HANDLE) -> Option<Vec<u8>> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];

    loop {
        let mut bytes_read = 0u32;
        let result = unsafe { ReadFile(pipe, Some(&mut chunk), Some(&mut bytes_read), None) };

        if bytes_read > 0 {
            let slice = &chunk[..bytes_read as usize];
            if let Some(pos) = slice.iter().position(|&b| b == b'\n') {
                buffer.extend_from_slice(&slice[..pos]);
                break;
            }
            buffer.extend_from_slice(slice);
            if buffer.len() >= MAX_REQUEST_BYTES {
                return None;
            }
        }

        if result.is_err() || bytes_read == 0 {
            break;
        }
    }

    if buffer.last() == Some(&b'\r') {
        buffer.pop();
    }

    if buffer.is_empty() {
        None
    } else {
        Some(buffer)
    }
}

fn write_all(pipe: HANDLE, mut data: &[u8]) {
    while !data.is_empty() {
        let mut written = 0u32;
        let ok = unsafe { WriteFile(pipe, Some(data), Some(&mut written), None).is_ok() };
        if !ok || written == 0 {
            break;
        }
        data = &data[written as usize..];
    }
}
