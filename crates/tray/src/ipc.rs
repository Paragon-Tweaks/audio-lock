use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use audio_lock_core::ipc::{AppAction, IpcRequest, IpcResponse, PIPE_NAME, QUIT_EVENT_NAME};
use audio_lock_core::model::AppState;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL,
    INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
    ReadFile, WriteFile,
};
use windows::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, INFINITE, OpenProcessToken, ResetEvent, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use crate::audio::AudioEngine;
use crate::win32::{
    self, WM_APP_APPLY_UPDATE, WM_APP_CHECK_UPDATES, WM_APP_OPEN_FLYOUT, WM_APP_QUIT, WakeHandle,
};

const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const IO_TIMEOUT_MS: u32 = 2_000;

pub struct IpcContext {
    pub state: Arc<Mutex<AppState>>,
    pub audio: Arc<AudioEngine>,
    pub last_anchor: Arc<Mutex<(i32, i32)>>,
    pub open_flyout_pending: Arc<AtomicBool>,
    pub quit_pending: Arc<AtomicBool>,
    pub check_updates_pending: Arc<AtomicBool>,
    pub apply_update_pending: Arc<AtomicBool>,
    pub wake: WakeHandle,
}

enum IpcAction {
    None,
    OpenFlyout,
    Quit,
    CheckUpdates,
    ApplyUpdate,
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct OwnedSecurityDescriptor {
    sd: PSECURITY_DESCRIPTOR,
}

impl Drop for OwnedSecurityDescriptor {
    fn drop(&mut self) {
        if !self.sd.0.is_null() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.sd.0)));
            }
        }
    }
}

pub fn spawn_server(ctx: IpcContext) {
    let _ = thread::Builder::new()
        .name("ipc-server".into())
        .spawn(move || run_server(ctx));
}

pub fn spawn_quit_event_listener(quit_pending: Arc<AtomicBool>, wake: WakeHandle) {
    let _ = thread::Builder::new()
        .name("quit-event-watcher".into())
        .spawn(move || {
            let name = win32::to_wide(QUIT_EVENT_NAME);
            let Ok(handle) = (unsafe { CreateEventW(None, true, false, PCWSTR(name.as_ptr())) })
            else {
                return;
            };

            let wait_result = unsafe { WaitForSingleObject(handle, INFINITE) };
            if wait_result == WAIT_OBJECT_0 {
                quit_pending.store(true, Ordering::Release);
                wake.post(WM_APP_QUIT);
            }

            // Intentionally keep `handle` open for the process lifetime so the named event
            // remains alive until process termination.
            let _ = handle;
        });
}

fn current_user_sid_string() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let token = OwnedHandle(token);

        let mut needed = 0u32;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut needed);
        if (needed as usize) < size_of::<TOKEN_USER>() {
            return None;
        }

        let mut buf = vec![0u8; needed as usize];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buf.as_mut_ptr().cast::<c_void>()),
            needed,
            &mut needed,
        )
        .ok()?;
        if (needed as usize) < size_of::<TOKEN_USER>() {
            return None;
        }

        let token_user = std::ptr::read_unaligned(buf.as_ptr().cast::<TOKEN_USER>());
        let mut sid_pwstr = PWSTR::null();
        ConvertSidToStringSidW(token_user.User.Sid, &mut sid_pwstr).ok()?;
        let sid_str = sid_pwstr.to_string().ok();
        if !sid_pwstr.0.is_null() {
            let _ = LocalFree(Some(HLOCAL(sid_pwstr.0.cast::<c_void>())));
        }
        sid_str
    }
}

fn build_pipe_security_descriptor() -> Option<OwnedSecurityDescriptor> {
    let user_sid = current_user_sid_string()?;
    let sddl = format!("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{user_sid})");
    let sddl_wide = win32::to_wide(&sddl);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_wide.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
        .ok()?;
    }
    if sd.0.is_null() {
        return None;
    }
    Some(OwnedSecurityDescriptor { sd })
}

fn run_server(ctx: IpcContext) {
    let pipe_name = win32::to_wide(PIPE_NAME);
    let pipe_sd = build_pipe_security_descriptor();
    let Ok(raw_event) = (unsafe { CreateEventW(None, true, false, PCWSTR::null()) }) else {
        return;
    };
    let io_event = OwnedHandle(raw_event);

    loop {
        let security_attrs = pipe_sd.as_ref().map(|owned| SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: owned.sd.0,
            ..Default::default()
        });
        let sa_ptr = security_attrs.as_ref().map(std::ptr::from_ref);

        let pipe = unsafe {
            CreateNamedPipeW(
                PCWSTR(pipe_name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                PIPE_BUFFER_SIZE,
                PIPE_BUFFER_SIZE,
                0,
                sa_ptr,
            )
        };

        if pipe.is_invalid() || pipe == INVALID_HANDLE_VALUE {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        if connect_overlapped(pipe, io_event.0) {
            let (action, wrote_ok) = handle_client(pipe, io_event.0, &ctx);
            unsafe {
                if wrote_ok {
                    let _ = FlushFileBuffers(pipe);
                }
                let _ = DisconnectNamedPipe(pipe);
            }
            match action {
                IpcAction::None => {}
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

fn connect_overlapped(pipe: HANDLE, event: HANDLE) -> bool {
    unsafe {
        let _ = ResetEvent(event);
        let mut overlapped = OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };
        match ConnectNamedPipe(pipe, Some(&mut overlapped)) {
            Ok(()) => true,
            Err(err) if err.code() == ERROR_PIPE_CONNECTED.to_hresult() => true,
            Err(err) if err.code() == ERROR_IO_PENDING.to_hresult() => {
                if WaitForSingleObject(event, INFINITE) != WAIT_OBJECT_0 {
                    let _ = CancelIo(pipe);
                    let mut ignored = 0u32;
                    let _ = GetOverlappedResult(pipe, &overlapped, &mut ignored, true);
                    return false;
                }
                let mut transferred = 0u32;
                GetOverlappedResult(pipe, &overlapped, &mut transferred, false).is_ok()
            }
            Err(_) => false,
        }
    }
}

fn handle_client(pipe: HANDLE, event: HANDLE, ctx: &IpcContext) -> (IpcAction, bool) {
    let Some(line) = read_line(pipe, event) else {
        return (IpcAction::None, false);
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

    let wrote_ok = if let Ok(mut payload) = serde_json::to_vec(&response) {
        payload.push(b'\n');
        write_all(pipe, event, &payload)
    } else {
        false
    };

    (action, wrote_ok)
}

fn read_overlapped(pipe: HANDLE, event: HANDLE, buf: &mut [u8], timeout_ms: u32) -> Option<usize> {
    unsafe {
        let _ = ResetEvent(event);
        let mut overlapped = OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };
        match ReadFile(pipe, Some(buf), None, Some(&mut overlapped)) {
            Ok(()) => {
                let mut transferred = 0u32;
                if GetOverlappedResult(pipe, &overlapped, &mut transferred, false).is_ok() {
                    Some(transferred as usize)
                } else {
                    None
                }
            }
            Err(err) if err.code() == ERROR_IO_PENDING.to_hresult() => {
                if WaitForSingleObject(event, timeout_ms) == WAIT_OBJECT_0 {
                    let mut transferred = 0u32;
                    match GetOverlappedResult(pipe, &overlapped, &mut transferred, false) {
                        Ok(()) => Some(transferred as usize),
                        Err(e) if e.code() == ERROR_BROKEN_PIPE.to_hresult() => Some(0),
                        Err(_) => None,
                    }
                } else {
                    let _ = CancelIo(pipe);
                    let mut ignored = 0u32;
                    let _ = GetOverlappedResult(pipe, &overlapped, &mut ignored, true);
                    None
                }
            }
            Err(err) if err.code() == ERROR_BROKEN_PIPE.to_hresult() => Some(0),
            Err(_) => None,
        }
    }
}

fn write_overlapped(pipe: HANDLE, event: HANDLE, data: &[u8], timeout_ms: u32) -> Option<usize> {
    unsafe {
        let _ = ResetEvent(event);
        let mut overlapped = OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };
        match WriteFile(pipe, Some(data), None, Some(&mut overlapped)) {
            Ok(()) => {
                let mut transferred = 0u32;
                if GetOverlappedResult(pipe, &overlapped, &mut transferred, false).is_ok() {
                    Some(transferred as usize)
                } else {
                    None
                }
            }
            Err(err) if err.code() == ERROR_IO_PENDING.to_hresult() => {
                if WaitForSingleObject(event, timeout_ms) == WAIT_OBJECT_0 {
                    let mut transferred = 0u32;
                    if GetOverlappedResult(pipe, &overlapped, &mut transferred, false).is_ok() {
                        Some(transferred as usize)
                    } else {
                        None
                    }
                } else {
                    let _ = CancelIo(pipe);
                    let mut ignored = 0u32;
                    let _ = GetOverlappedResult(pipe, &overlapped, &mut ignored, true);
                    None
                }
            }
            Err(_) => None,
        }
    }
}

fn remaining_timeout_ms(deadline: Instant) -> Option<u32> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return None;
    }
    Some(remaining.as_millis().clamp(1, u128::from(u32::MAX)) as u32)
}

fn read_line(pipe: HANDLE, event: HANDLE) -> Option<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_millis(u64::from(IO_TIMEOUT_MS));
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];

    loop {
        let timeout_ms = remaining_timeout_ms(deadline)?;
        let bytes_read = read_overlapped(pipe, event, &mut chunk, timeout_ms)?;
        if bytes_read == 0 {
            break;
        }

        let slice = &chunk[..bytes_read];
        if let Some(pos) = slice.iter().position(|&b| b == b'\n') {
            if buffer.len() + pos > MAX_REQUEST_BYTES {
                return None;
            }
            buffer.extend_from_slice(&slice[..pos]);
            break;
        }
        if buffer.len() + slice.len() > MAX_REQUEST_BYTES {
            return None;
        }
        buffer.extend_from_slice(slice);
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

fn write_all(pipe: HANDLE, event: HANDLE, mut data: &[u8]) -> bool {
    let deadline = Instant::now() + Duration::from_millis(u64::from(IO_TIMEOUT_MS));
    while !data.is_empty() {
        let Some(timeout_ms) = remaining_timeout_ms(deadline) else {
            return false;
        };
        let Some(written) = write_overlapped(pipe, event, data, timeout_ms) else {
            return false;
        };
        if written == 0 {
            return false;
        }
        data = &data[written..];
    }
    true
}
