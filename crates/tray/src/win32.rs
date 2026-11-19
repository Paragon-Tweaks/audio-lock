use std::ffi::c_void;
use std::mem::size_of;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{SETPROCESSWORKINGSETSIZEEX_FLAGS, SetProcessWorkingSetSizeEx};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    BELOW_NORMAL_PRIORITY_CLASS, CreateMutexW, GetCurrentProcess, GetCurrentThread,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, SetPriorityClass,
    SetProcessInformation, SetThreadPriority, THREAD_PRIORITY_IDLE,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, HWND_MESSAGE, KillTimer,
    PostMessageW, PostQuitMessage, RegisterClassW, SetTimer, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_USER, WNDCLASSW,
};
use windows::core::PCWSTR;

pub const WM_APP_REFRESH_TRAY: u32 = WM_USER + 1;
pub const WM_APP_TRAY_EVENT: u32 = WM_USER + 2;
pub const WM_APP_OPEN_FLYOUT: u32 = WM_USER + 3;
pub const WM_APP_QUIT: u32 = WM_USER + 4;
pub const WM_APP_POPUP_ACTION: u32 = WM_USER + 5;
pub const WM_APP_UPDATE_STAGED: u32 = WM_USER + 6;
pub const WM_APP_APPLY_UPDATE: u32 = WM_USER + 7;
pub const WM_APP_CHECK_UPDATES: u32 = WM_USER + 8;
pub const WM_APP_FLYOUT_ACTION: u32 = WM_USER + 9;

const MUTEX_WAIT_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct SingleInstanceGuard(HANDLE);

impl SingleInstanceGuard {
    pub fn acquire(name: &str) -> Option<Self> {
        let wide = to_wide(name);
        unsafe {
            let handle = CreateMutexW(None, false, PCWSTR(wide.as_ptr())).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return None;
            }
            Some(Self(handle))
        }
    }

    pub fn acquire_with_wait(name: &str, wait_ms: u64) -> Option<Self> {
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        loop {
            if let Some(guard) = Self::acquire(name) {
                return Some(guard);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(MUTEX_WAIT_POLL_INTERVAL);
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn enable_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn configure_low_impact_process() {
    unsafe {
        let process = GetCurrentProcess();
        let _ = SetPriorityClass(process, BELOW_NORMAL_PRIORITY_CLASS);

        let throttling = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        };
        let _ = SetProcessInformation(
            process,
            ProcessPowerThrottling,
            (&raw const throttling).cast::<c_void>(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

pub fn trim_working_set() {
    unsafe {
        let _ = SetProcessWorkingSetSizeEx(
            GetCurrentProcess(),
            usize::MAX,
            usize::MAX,
            SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
        );
    }
}

pub fn set_current_thread_idle_priority() {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_IDLE);
    }
}

pub fn idle_duration_ms() -> u32 {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        unsafe { GetTickCount() }.wrapping_sub(info.dwTime)
    } else {
        0
    }
}

pub fn set_timer(hwnd: HWND, id: usize, interval_ms: u32) {
    unsafe {
        let _ = SetTimer(Some(hwnd), id, interval_ms, None);
    }
}

pub fn kill_timer(hwnd: HWND, id: usize) {
    unsafe {
        let _ = KillTimer(Some(hwnd), id);
    }
}

pub fn cursor_position() -> Option<(i32, i32)> {
    let mut pt = POINT::default();
    unsafe { GetCursorPos(&mut pt).ok().map(|()| (pt.x, pt.y)) }
}

pub fn post_quit() {
    unsafe {
        PostQuitMessage(0);
    }
}

#[derive(Clone, Copy)]
pub struct WakeHandle(isize);

impl WakeHandle {
    pub fn post(self, message: u32) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.0 as *mut c_void)),
                message,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

pub struct MessageWindow {
    hwnd: HWND,
}

impl MessageWindow {
    pub fn create() -> windows::core::Result<Self> {
        let class_name = to_wide("DefaultAudioLockTrayMessageWindow");
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(message_wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance.into()),
                None,
            )?;
            Ok(Self { hwnd })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn wake_handle(&self) -> WakeHandle {
        WakeHandle(self.hwnd.0 as isize)
    }
}

impl Drop for MessageWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn message_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
