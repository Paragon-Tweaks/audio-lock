use std::ffi::c_void;
use std::mem::size_of;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, FreeLibrary, GetLastError, HANDLE, HMODULE,
    HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{SETPROCESSWORKINGSETSIZEEX_FLAGS, SetProcessWorkingSetSizeEx};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    BELOW_NORMAL_PRIORITY_CLASS, CreateMutexW, GetCurrentProcess, GetCurrentThread, OpenProcess,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE, PROCESS_SYNCHRONIZE, ProcessPowerThrottling, SetPriorityClass,
    SetProcessInformation, SetThreadPriority, THREAD_PRIORITY_IDLE, WaitForSingleObject,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    QUNS_ACCEPTS_NOTIFICATIONS, QUNS_NOT_PRESENT, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, KillTimer, PostMessageW,
    PostQuitMessage, RegisterClassW, SetTimer, WM_SETTINGCHANGE, WM_USER, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, s, w};

pub const WM_APP_REFRESH_TRAY: u32 = WM_USER + 1;
pub const WM_APP_TRAY_EVENT: u32 = WM_USER + 2;
pub const WM_APP_OPEN_FLYOUT: u32 = WM_USER + 3;
pub const WM_APP_QUIT: u32 = WM_USER + 4;
pub const WM_APP_POPUP_ACTION: u32 = WM_USER + 5;
pub const WM_APP_UPDATE_STAGED: u32 = WM_USER + 6;
pub const WM_APP_APPLY_UPDATE: u32 = WM_USER + 7;
pub const WM_APP_CHECK_UPDATES: u32 = WM_USER + 8;
pub const WM_APP_FLYOUT_ACTION: u32 = WM_USER + 9;
pub const WM_APP_THEME_CHANGED: u32 = WM_USER + 10;

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
        let diff = unsafe { GetTickCount() }.wrapping_sub(info.dwTime) as i32;
        if diff <= 0 { 0 } else { diff as u32 }
    } else {
        0
    }
}

pub fn is_fullscreen_or_busy() -> bool {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(state) => state != QUNS_ACCEPTS_NOTIFICATIONS && state != QUNS_NOT_PRESENT,
        Err(_) => true,
    }
}

#[repr(C)]
#[derive(Default)]
struct XInputState {
    dw_packet_number: u32,
    _gamepad: [u8; 12],
}

type XInputGetStateFn = unsafe extern "system" fn(u32, *mut XInputState) -> u32;

pub struct ControllerIdleTracker {
    loaded: bool,
    module: Option<HMODULE>,
    get_state: Option<XInputGetStateFn>,
    prev_packets: [Option<u32>; 4],
    last_controller_activity: Option<Instant>,
}

impl Default for ControllerIdleTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ControllerIdleTracker {
    pub fn new() -> Self {
        Self {
            loaded: false,
            module: None,
            get_state: None,
            prev_packets: [None; 4],
            last_controller_activity: None,
        }
    }

    pub fn sample_and_idle_ms(&mut self) -> u32 {
        let kb_mouse_ms = idle_duration_ms();
        self.ensure_loaded();

        if let Some(get_state) = self.get_state {
            let now = Instant::now();
            for (slot, prev_packet) in self.prev_packets.iter_mut().enumerate() {
                let mut state = XInputState::default();
                let rc = unsafe { get_state(slot as u32, &mut state) };
                if rc == ERROR_SUCCESS.0 {
                    if *prev_packet != Some(state.dw_packet_number) {
                        *prev_packet = Some(state.dw_packet_number);
                        self.last_controller_activity = Some(now);
                    }
                } else {
                    *prev_packet = None;
                }
            }
        }

        let ctrl_idle_ms = self.last_controller_activity.map_or(u32::MAX, |at| {
            at.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
        });
        kb_mouse_ms.min(ctrl_idle_ms)
    }

    pub fn reset(&mut self) {
        self.prev_packets = [None; 4];
        self.last_controller_activity = None;
    }

    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;

        for dll in [w!("xinput1_4.dll"), w!("xinput9_1_0.dll")] {
            let Ok(module) = (unsafe { LoadLibraryW(dll) }) else {
                continue;
            };
            let proc = unsafe { GetProcAddress(module, s!("XInputGetState")) };
            if let Some(func) = proc {
                let get_state = unsafe {
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, XInputGetStateFn>(
                        func,
                    )
                };
                self.module = Some(module);
                self.get_state = Some(get_state);
                return;
            }
            unsafe {
                let _ = FreeLibrary(module);
            }
        }
    }
}

impl Drop for ControllerIdleTracker {
    fn drop(&mut self) {
        if let Some(module) = self.module.take() {
            unsafe {
                let _ = FreeLibrary(module);
            }
        }
    }
}

pub fn wait_for_pid_exit(pid: u32, timeout_ms: u64) {
    if pid == 0 || pid == std::process::id() {
        return;
    }
    let timeout = u32::try_from(timeout_ms).unwrap_or(u32::MAX - 1);
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            let _ = WaitForSingleObject(handle, timeout);
            let _ = CloseHandle(handle);
        }
    }
}

pub fn is_system_light_taskbar() -> bool {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast::<c_void>()),
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS && value == 1
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
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
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
    if msg == WM_SETTINGCHANGE {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_THEME_CHANGED, WPARAM(0), LPARAM(0));
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
