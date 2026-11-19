use std::cell::{Cell, RefCell};
use std::f32::consts::PI;
use std::ffi::c_void;
use std::mem::size_of;
use std::rc::Rc;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use audio_lock_core::ipc::AppAction;
use audio_lock_core::model::{AppState, ThemeMode};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET,
    DEFAULT_PITCH, DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, EndPaint, FF_SWISS, FW_BOLD, FW_NORMAL,
    FW_SEMIBOLD, GdiFlush, GetMonitorInfoW, GetTextExtentPoint32W, HBITMAP, HDC, HFONT, HGDIOBJ,
    IntersectClipRect, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
    OUT_TT_PRECIS, PAINTSTRUCT, RestoreDC, SRCCOPY, SaveDC, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetWindowLongPtrW, HTCLIENT, HWND_TOPMOST, IDC_ARROW, IDC_HAND, LoadCursorW, RegisterClassW,
    SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetCursor, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, WA_INACTIVE, WHEEL_DELTA, WM_ACTIVATE, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT,
    WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSW, WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::PCWSTR;

use crate::win32::{self, WM_APP_POPUP_ACTION, WakeHandle};

const POPUP_CLASS_NAME: &str = "DefaultAudioLockPopupMenu";

const MENU_WIDTH_LOGICAL: f32 = 224.0;
const OUTER_PAD_LOGICAL: f32 = 6.0;
const ROW_GAP_LOGICAL: f32 = 2.0;
const ROW_PAD_X_LOGICAL: f32 = 8.0;
const ROW_RADIUS_LOGICAL: f32 = 7.0;
const ACTION_ROW_HEIGHT_LOGICAL: f32 = 34.0;
const PROFILE_ROW_HEIGHT_LOGICAL: f32 = 30.0;
const DIVIDER_HEIGHT_LOGICAL: f32 = 9.0;
const DIVIDER_INSET_X_LOGICAL: f32 = 4.0;
const HEADER_HEIGHT_LOGICAL: f32 = 20.0;
const ICON_SIZE_LOGICAL: f32 = 13.5;
const CHECK_ICON_SIZE_LOGICAL: f32 = 12.5;
const ICON_TEXT_GAP_LOGICAL: f32 = 8.0;
const PILL_HEIGHT_LOGICAL: f32 = 18.0;
const PILL_PAD_X_LOGICAL: f32 = 6.5;
const EDGE_MARGIN_LOGICAL: f32 = 8.0;
const MAX_VISIBLE_PROFILES: usize = 8;

static REGISTER_CLASS: Once = Once::new();

pub enum PopupAction {
    OpenFlyout((i32, i32)),
    ApplyAction(AppAction),
    Quit,
}

pub struct PopupMenu {
    shared: Rc<PopupShared>,
}

struct PopupShared {
    state: Arc<Mutex<AppState>>,
    wake: WakeHandle,
    active_hwnd: Cell<Option<HWND>>,
    last_closed_at: Cell<Option<Instant>>,
    pending_actions: RefCell<Vec<PopupAction>>,
}

impl PopupShared {
    fn push_action(&self, action: PopupAction) {
        self.pending_actions.borrow_mut().push(action);
        self.wake.post(WM_APP_POPUP_ACTION);
    }

    fn on_destroyed(&self, hwnd: HWND) {
        if self.active_hwnd.get() == Some(hwnd) {
            self.active_hwnd.set(None);
        }
        self.last_closed_at.set(Some(Instant::now()));
    }
}

impl PopupMenu {
    pub fn new(state: Arc<Mutex<AppState>>, wake: WakeHandle) -> Self {
        Self {
            shared: Rc::new(PopupShared {
                state,
                wake,
                active_hwnd: Cell::new(None),
                last_closed_at: Cell::new(None),
                pending_actions: RefCell::new(Vec::new()),
            }),
        }
    }

    pub fn is_open(&self) -> bool {
        self.shared.active_hwnd.get().is_some()
    }

    pub fn closed_within(&self, duration: Duration) -> bool {
        self.shared
            .last_closed_at
            .get()
            .is_some_and(|closed_at| closed_at.elapsed() < duration)
    }

    pub fn open(&self, anchor: (i32, i32)) {
        if self.is_open() {
            self.close();
        }
        register_window_class();

        let monitor = unsafe {
            MonitorFromPoint(
                POINT {
                    x: anchor.0,
                    y: anchor.1,
                },
                MONITOR_DEFAULTTOPRIMARY,
            )
        };

        let dpi = monitor_dpi(monitor);
        let scale = dpi as f32 / 96.0;
        let snapshot = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let layout = MenuLayout::compute(&snapshot, scale, 0);
        let (x, y) = compute_window_position(monitor, anchor, layout.width, layout.height, scale);

        let class_name = win32::to_wide(POPUP_CLASS_NAME);
        let hinstance = unsafe { GetModuleHandleW(None) }.ok();

        let Ok(hwnd) = (unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WS_POPUP | WS_CLIPCHILDREN,
                x,
                y,
                layout.width,
                layout.height,
                None,
                None,
                hinstance.map(Into::into),
                None,
            )
        }) else {
            return;
        };

        let actual_dpi = unsafe { GetDpiForWindow(hwnd) };
        let (final_scale, final_layout, final_pos) = if actual_dpi > 0 && actual_dpi != dpi {
            let s = actual_dpi as f32 / 96.0;
            let l = MenuLayout::compute(&snapshot, s, 0);
            let p = compute_window_position(monitor, anchor, l.width, l.height, s);
            (s, l, p)
        } else {
            (scale, layout, (x, y))
        };

        let palette = MenuPalette::from_mode(snapshot.theme);
        let dwm_border_ok = apply_dwm_chrome(hwnd, palette.border_strong);

        let data = Box::new(WindowData {
            shared: Rc::clone(&self.shared),
            anchor,
            scale: final_scale,
            layout: final_layout,
            fonts: MenuFonts::new(final_scale),
            backbuffer: DibBackbuffer::empty(),
            scroll_offset: 0,
            hovered: None,
            pressed: None,
            confirm_quit: false,
            tracking_mouse: false,
            dwm_border_ok,
            is_closing: false,
        });
        let width = data.layout.width;
        let height = data.layout.height;
        let ptr = Box::into_raw(data);

        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        }
        self.shared.active_hwnd.set(Some(hwnd));

        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                final_pos.0,
                final_pos.1,
                width,
                height,
                SWP_NOACTIVATE,
            );
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    pub fn close(&self) {
        if let Some(hwnd) = self.shared.active_hwnd.get() {
            request_close(hwnd);
        }
    }

    pub fn invalidate(&self) {
        let Some(hwnd) = self.shared.active_hwnd.get() else {
            return;
        };
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowData;
        if ptr.is_null() {
            return;
        }
        let (border_color, resize_info) = {
            let data = unsafe { &mut *ptr };
            let old_w = data.layout.width;
            let old_h = data.layout.height;
            let snapshot = data
                .shared
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            data.layout = MenuLayout::compute(&snapshot, data.scale, data.scroll_offset);
            data.scroll_offset = data.scroll_offset.clamp(0, data.layout.max_scroll);
            let new_w = data.layout.width;
            let new_h = data.layout.height;
            let resize_info = (new_w != old_w || new_h != old_h)
                .then_some((data.anchor, data.scale, new_w, new_h));
            (MenuPalette::from_mode(snapshot.theme).border_strong, resize_info)
        };
        if let Some((anchor, scale, new_w, new_h)) = resize_info {
            let monitor = unsafe {
                MonitorFromPoint(
                    POINT {
                        x: anchor.0,
                        y: anchor.1,
                    },
                    MONITOR_DEFAULTTOPRIMARY,
                )
            };
            let (x, y) = compute_window_position(monitor, anchor, new_w, new_h, scale);
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    new_w,
                    new_h,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
        let _ = apply_dwm_chrome(hwnd, border_color);
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    pub fn take_actions(&self) -> Vec<PopupAction> {
        std::mem::take(&mut *self.shared.pending_actions.borrow_mut())
    }
}

impl Drop for PopupMenu {
    fn drop(&mut self) {
        self.close();
    }
}

struct MenuFonts {
    regular: HFONT,
    semibold: HFONT,
    profile_regular: HFONT,
    profile_active: HFONT,
    header: HFONT,
    badge: HFONT,
}

impl MenuFonts {
    fn new(scale: f32) -> Self {
        Self {
            regular: create_ui_font(12.5, scale, FW_NORMAL.0 as i32),
            semibold: create_ui_font(12.5, scale, FW_SEMIBOLD.0 as i32),
            profile_regular: create_ui_font(12.0, scale, FW_NORMAL.0 as i32),
            profile_active: create_ui_font(12.0, scale, FW_SEMIBOLD.0 as i32),
            header: create_ui_font(10.0, scale, FW_SEMIBOLD.0 as i32),
            badge: create_ui_font(9.5, scale, FW_BOLD.0 as i32),
        }
    }

    fn release(&mut self) {
        for font in [
            &mut self.regular,
            &mut self.semibold,
            &mut self.profile_regular,
            &mut self.profile_active,
            &mut self.header,
            &mut self.badge,
        ] {
            if !font.is_invalid() {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(font.0));
                }
                *font = HFONT::default();
            }
        }
    }
}

struct DibBackbuffer {
    mem_dc: HDC,
    dib: HBITMAP,
    old_obj: HGDIOBJ,
    pixels: *mut u32,
    buf_w: i32,
    buf_h: i32,
}

impl DibBackbuffer {
    fn empty() -> Self {
        Self {
            mem_dc: HDC::default(),
            dib: HBITMAP::default(),
            old_obj: HGDIOBJ::default(),
            pixels: std::ptr::null_mut(),
            buf_w: 0,
            buf_h: 0,
        }
    }

    fn ensure(&mut self, hdc: HDC, width: i32, height: i32) -> bool {
        if width <= 0 || height <= 0 {
            return false;
        }
        if !self.mem_dc.is_invalid()
            && !self.dib.0.is_null()
            && !self.pixels.is_null()
            && self.buf_w == width
            && self.buf_h == height
        {
            return true;
        }

        self.release();

        let mem_dc = unsafe { CreateCompatibleDC(Some(hdc)) };
        if mem_dc.is_invalid() {
            return false;
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(dib) =
            (unsafe { CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) })
        else {
            unsafe {
                let _ = DeleteDC(mem_dc);
            }
            return false;
        };

        if bits.is_null() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(dib.0));
                let _ = DeleteDC(mem_dc);
            }
            return false;
        }

        let old_obj = unsafe { SelectObject(mem_dc, HGDIOBJ(dib.0)) };
        unsafe {
            SetBkMode(mem_dc, TRANSPARENT);
        }

        self.mem_dc = mem_dc;
        self.dib = dib;
        self.old_obj = old_obj;
        self.pixels = bits.cast::<u32>();
        self.buf_w = width;
        self.buf_h = height;
        true
    }

    fn release(&mut self) {
        if !self.mem_dc.is_invalid() {
            unsafe {
                if !self.old_obj.0.is_null() {
                    SelectObject(self.mem_dc, self.old_obj);
                }
                if !self.dib.0.is_null() {
                    let _ = DeleteObject(HGDIOBJ(self.dib.0));
                }
                let _ = DeleteDC(self.mem_dc);
            }
        } else if !self.dib.0.is_null() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.dib.0));
            }
        }
        self.mem_dc = HDC::default();
        self.dib = HBITMAP::default();
        self.old_obj = HGDIOBJ::default();
        self.pixels = std::ptr::null_mut();
        self.buf_w = 0;
        self.buf_h = 0;
    }
}

struct WindowData {
    shared: Rc<PopupShared>,
    anchor: (i32, i32),
    scale: f32,
    layout: MenuLayout,
    fonts: MenuFonts,
    backbuffer: DibBackbuffer,
    scroll_offset: i32,
    hovered: Option<HitItem>,
    pressed: Option<HitItem>,
    confirm_quit: bool,
    tracking_mouse: bool,
    dwm_border_ok: bool,
    is_closing: bool,
}

impl Drop for WindowData {
    fn drop(&mut self) {
        self.backbuffer.release();
        self.fonts.release();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HitItem {
    OpenFlyout,
    ToggleLock,
    Profile(String),
    Quit,
}

#[derive(Clone, Copy, Default)]
struct RectPx {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl RectPx {
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    fn width(self) -> i32 {
        (self.right - self.left).max(0)
    }

    fn height(self) -> i32 {
        (self.bottom - self.top).max(0)
    }

    fn to_win32(self) -> RECT {
        RECT {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
        }
    }
}

struct ProfileRowLayout {
    id: String,
    name: String,
    active: bool,
    rect: RectPx,
}

struct MenuLayout {
    width: i32,
    height: i32,
    open_row: RectPx,
    lock_row: RectPx,
    divider_1_y: i32,
    header_rect: RectPx,
    profiles_clip: RectPx,
    profile_rows: Vec<ProfileRowLayout>,
    empty_profiles_rect: Option<RectPx>,
    divider_2_y: i32,
    quit_row: RectPx,
    max_scroll: i32,
}

impl MenuLayout {
    fn compute(state: &AppState, scale: f32, scroll_offset: i32) -> Self {
        let width = px(MENU_WIDTH_LOGICAL, scale);
        let pad = px(OUTER_PAD_LOGICAL, scale);
        let gap = px(ROW_GAP_LOGICAL, scale);
        let action_h = px(ACTION_ROW_HEIGHT_LOGICAL, scale);
        let profile_h = px(PROFILE_ROW_HEIGHT_LOGICAL, scale);
        let div_h = px(DIVIDER_HEIGHT_LOGICAL, scale);
        let header_h = px(HEADER_HEIGHT_LOGICAL, scale);

        let left = pad;
        let right = width - pad;
        let mut y = pad;

        let open_row = RectPx {
            left,
            top: y,
            right,
            bottom: y + action_h,
        };
        y = open_row.bottom + gap;

        let lock_row = RectPx {
            left,
            top: y,
            right,
            bottom: y + action_h,
        };
        y = lock_row.bottom + gap;

        let divider_1_y = y + div_h / 2;
        y += div_h + gap;

        let header_rect = RectPx {
            left,
            top: y,
            right,
            bottom: y + header_h,
        };
        y = header_rect.bottom + gap;

        let profile_count = state.profiles.len();
        let visible_count = profile_count.clamp(1, MAX_VISIBLE_PROFILES);
        let total_profiles_h = if profile_count == 0 {
            profile_h
        } else {
            profile_count as i32 * profile_h + (profile_count.saturating_sub(1) as i32) * gap
        };
        let visible_profiles_h =
            visible_count as i32 * profile_h + (visible_count.saturating_sub(1) as i32) * gap;
        let max_scroll = (total_profiles_h - visible_profiles_h).max(0);
        let clamped_scroll = scroll_offset.clamp(0, max_scroll);

        let profiles_clip = RectPx {
            left,
            top: y,
            right,
            bottom: y + visible_profiles_h,
        };

        let mut profile_rows = Vec::with_capacity(profile_count);
        let empty_profiles_rect = if profile_count == 0 {
            Some(RectPx {
                left,
                top: y,
                right,
                bottom: y + profile_h,
            })
        } else {
            let mut py = y - clamped_scroll;
            let active_id = state.active_profile_id.as_deref();
            for profile in &state.profiles {
                let rect = RectPx {
                    left,
                    top: py,
                    right,
                    bottom: py + profile_h,
                };
                profile_rows.push(ProfileRowLayout {
                    id: profile.id.clone(),
                    name: profile.name.clone(),
                    active: active_id == Some(profile.id.as_str()),
                    rect,
                });
                py += profile_h + gap;
            }
            None
        };

        y = profiles_clip.bottom + gap;

        let divider_2_y = y + div_h / 2;
        y += div_h + gap;

        let quit_row = RectPx {
            left,
            top: y,
            right,
            bottom: y + action_h,
        };
        let height = quit_row.bottom + pad;

        Self {
            width,
            height,
            open_row,
            lock_row,
            divider_1_y,
            header_rect,
            profiles_clip,
            profile_rows,
            empty_profiles_rect,
            divider_2_y,
            quit_row,
            max_scroll,
        }
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<HitItem> {
        if self.open_row.contains(x, y) {
            return Some(HitItem::OpenFlyout);
        }
        if self.lock_row.contains(x, y) {
            return Some(HitItem::ToggleLock);
        }
        if self.profiles_clip.contains(x, y) {
            for row in &self.profile_rows {
                if row.rect.contains(x, y) {
                    return Some(HitItem::Profile(row.id.clone()));
                }
            }
        }
        if self.quit_row.contains(x, y) {
            return Some(HitItem::Quit);
        }
        None
    }
}

#[derive(Clone, Copy)]
struct Rgb(u8, u8, u8);

impl Rgb {
    const fn from_hex(hex: u32) -> Self {
        Self(
            ((hex >> 16) & 0xFF) as u8,
            ((hex >> 8) & 0xFF) as u8,
            (hex & 0xFF) as u8,
        )
    }

    const fn colorref(self) -> COLORREF {
        COLORREF((self.0 as u32) | ((self.1 as u32) << 8) | ((self.2 as u32) << 16))
    }
}

#[derive(Clone, Copy)]
struct MenuPalette {
    bg: Rgb,
    bg_elev: Rgb,
    bg_hover: Rgb,
    border: Rgb,
    border_strong: Rgb,
    divider: Rgb,
    text: Rgb,
    text_sec: Rgb,
    text_muted: Rgb,
    inv_bg: Rgb,
    inv_text: Rgb,
}

impl MenuPalette {
    const fn from_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self {
                bg: Rgb::from_hex(0x121212),
                bg_elev: Rgb::from_hex(0x191919),
                bg_hover: Rgb::from_hex(0x212121),
                border: Rgb::from_hex(0x262626),
                border_strong: Rgb::from_hex(0x383838),
                divider: Rgb::from_hex(0x202020),
                text: Rgb::from_hex(0xF5F5F5),
                text_sec: Rgb::from_hex(0xA3A3A3),
                text_muted: Rgb::from_hex(0x737373),
                inv_bg: Rgb::from_hex(0xF5F5F5),
                inv_text: Rgb::from_hex(0x0A0A0A),
            },
            ThemeMode::Light => Self {
                bg: Rgb::from_hex(0xFFFFFF),
                bg_elev: Rgb::from_hex(0xF7F7F7),
                bg_hover: Rgb::from_hex(0xEFEFEF),
                border: Rgb::from_hex(0xDEDEDE),
                border_strong: Rgb::from_hex(0xC6C6C6),
                divider: Rgb::from_hex(0xEAEAEA),
                text: Rgb::from_hex(0x111111),
                text_sec: Rgb::from_hex(0x525252),
                text_muted: Rgb::from_hex(0x737373),
                inv_bg: Rgb::from_hex(0x141414),
                inv_text: Rgb::from_hex(0xFFFFFF),
            },
        }
    }
}

#[derive(Clone, Copy)]
enum IconKind {
    Window,
    Lock,
    Unlock,
    Check,
    Power,
}

fn px(logical: f32, scale: f32) -> i32 {
    (logical * scale).round() as i32
}

fn register_window_class() {
    REGISTER_CLASS.call_once(|| {
        let class_name = win32::to_wide(POPUP_CLASS_NAME);
        unsafe {
            let Ok(hinstance) = GetModuleHandleW(None) else {
                return;
            };
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(popup_wnd_proc),
                hInstance: hinstance.into(),
                hCursor: cursor,
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            RegisterClassW(&wc);
        }
    });
}

fn monitor_dpi(monitor: windows::Win32::Graphics::Gdi::HMONITOR) -> u32 {
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    let ok = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    if ok.is_ok() && dpi_x > 0 { dpi_x } else { 96 }
}

fn compute_window_position(
    monitor: windows::Win32::Graphics::Gdi::HMONITOR,
    anchor: (i32, i32),
    win_w: i32,
    win_h: i32,
    scale: f32,
) -> (i32, i32) {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return (anchor.0 - win_w / 2, anchor.1 - win_h);
    }

    let work = info.rcWork;
    let margin = px(EDGE_MARGIN_LOGICAL, scale);

    let min_x = work.left + margin;
    let max_x = (work.right - margin - win_w).max(min_x);
    let x = (anchor.0 - win_w / 2).clamp(min_x, max_x);

    let min_y = work.top + margin;
    let max_y = (work.bottom - margin - win_h).max(min_y);
    let raw_y = if anchor.1 <= work.top {
        work.top + margin
    } else if anchor.1 >= work.bottom {
        work.bottom - margin - win_h
    } else if anchor.1 - win_h - margin >= work.top {
        anchor.1 - win_h - margin
    } else {
        anchor.1 + margin
    };
    let y = raw_y.clamp(min_y, max_y);

    (x, y)
}

fn apply_dwm_chrome(hwnd: HWND, border_color: Rgb) -> bool {
    let corner = DWMWCP_ROUND;
    let border = border_color.colorref();
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const corner).cast::<c_void>(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&raw const border).cast::<c_void>(),
            size_of::<COLORREF>() as u32,
        )
        .is_ok()
    }
}

fn request_close(hwnd: HWND) {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowData;
    if ptr.is_null() {
        return;
    }
    let should_destroy = {
        let data = unsafe { &mut *ptr };
        if data.is_closing {
            false
        } else {
            data.is_closing = true;
            data.confirm_quit = false;
            true
        }
    };
    if should_destroy {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn lparam_point(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam.0 & 0xFFFF) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
    (x, y)
}

unsafe extern "system" fn popup_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowData;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }

    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint_menu(hwnd, unsafe { &mut *ptr });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_point(lparam);
            let (need_track, hovered_changed) = {
                let data = unsafe { &mut *ptr };
                let need_track = !data.tracking_mouse;
                if need_track {
                    data.tracking_mouse = true;
                }
                let next_hovered = data.layout.hit_test(x, y);
                let changed = next_hovered != data.hovered;
                if changed {
                    data.hovered = next_hovered;
                }
                (need_track, changed)
            };
            if need_track {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut tme);
                }
            }
            if hovered_changed {
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let hovered_changed = {
                let data = unsafe { &mut *ptr };
                data.tracking_mouse = false;
                let had_hover = data.hovered.is_some();
                data.hovered = None;
                data.pressed = None;
                had_hover
            };
            if hovered_changed {
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                let is_interactive = unsafe { &*ptr }.hovered.is_some();
                let cursor_id = if is_interactive { IDC_HAND } else { IDC_ARROW };
                unsafe {
                    if let Ok(cursor) = LoadCursorW(None, cursor_id) {
                        SetCursor(Some(cursor));
                    }
                }
                return LRESULT(1);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_point(lparam);
            let data = unsafe { &mut *ptr };
            data.pressed = data.layout.hit_test(x, y);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_point(lparam);
            let (should_close, should_repaint) = {
                let data = unsafe { &mut *ptr };
                let hit = data.layout.hit_test(x, y);
                let pressed = data.pressed.take();
                let matched = match (&pressed, &hit) {
                    (None, Some(item)) => Some(item.clone()),
                    (Some(p), Some(h)) if p == h => Some(h.clone()),
                    _ => None,
                };
                match matched {
                    Some(HitItem::OpenFlyout) => {
                        data.confirm_quit = false;
                        data.shared.push_action(PopupAction::OpenFlyout(data.anchor));
                        (true, false)
                    }
                    Some(HitItem::ToggleLock) => {
                        let was_confirming = std::mem::replace(&mut data.confirm_quit, false);
                        let next = !data
                            .shared
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .lock_enabled;
                        data.shared
                            .push_action(PopupAction::ApplyAction(AppAction::SetLockEnabled(next)));
                        (false, was_confirming)
                    }
                    Some(HitItem::Profile(id)) => {
                        data.confirm_quit = false;
                        data.shared
                            .push_action(PopupAction::ApplyAction(AppAction::ApplyProfile {
                                profile_id: id,
                            }));
                        (true, false)
                    }
                    Some(HitItem::Quit) => {
                        if data.confirm_quit {
                            data.confirm_quit = false;
                            data.shared.push_action(PopupAction::Quit);
                            (true, false)
                        } else {
                            data.confirm_quit = true;
                            (false, true)
                        }
                    }
                    None => {
                        let was_confirming = std::mem::replace(&mut data.confirm_quit, false);
                        (false, was_confirming)
                    }
                }
            };
            if should_close {
                request_close(hwnd);
            } else if should_repaint {
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            request_close(hwnd);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let changed = {
                let data = unsafe { &mut *ptr };
                if data.layout.max_scroll == 0 {
                    false
                } else {
                    let step = px(PROFILE_ROW_HEIGHT_LOGICAL + ROW_GAP_LOGICAL, data.scale);
                    let ticks = -(delta as f32 / WHEEL_DELTA as f32);
                    let next = (data.scroll_offset + (ticks * step as f32).round() as i32)
                        .clamp(0, data.layout.max_scroll);
                    if next != data.scroll_offset {
                        data.scroll_offset = next;
                        let snapshot = data
                            .shared
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone();
                        data.layout = MenuLayout::compute(&snapshot, data.scale, data.scroll_offset);
                        true
                    } else {
                        false
                    }
                }
            };
            if changed {
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if (wparam.0 as u16) == VK_ESCAPE.0 {
                let cancelled_confirm = {
                    let data = unsafe { &mut *ptr };
                    std::mem::replace(&mut data.confirm_quit, false)
                };
                if cancelled_confirm {
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                } else {
                    request_close(hwnd);
                }
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                request_close(hwnd);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let data = Box::from_raw(ptr);
                data.shared.on_destroyed(hwnd);
                drop(data);
            }
            win32::trim_working_set();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn paint_menu(hwnd: HWND, data: &mut WindowData) {
    let snapshot = data
        .shared
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let palette = MenuPalette::from_mode(snapshot.theme);
    let scale = data.scale;
    let width = data.layout.width;
    let height = data.layout.height;

    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    if hdc.is_invalid() || width <= 0 || height <= 0 {
        unsafe {
            let _ = EndPaint(hwnd, &ps);
        }
        return;
    }

    if !data.backbuffer.ensure(hdc, width, height) {
        unsafe {
            let _ = EndPaint(hwnd, &ps);
        }
        return;
    }

    let mem_dc = data.backbuffer.mem_dc;
    let pixels_ptr = data.backbuffer.pixels.cast::<u8>();
    let layout = &data.layout;
    let fonts = &data.fonts;

    let badge_text = if snapshot.lock_enabled { "ON" } else { "OFF" };
    let badge_rect = compute_badge_rect(mem_dc, fonts.badge, badge_text, layout.lock_row, scale);

    unsafe {
        let _ = GdiFlush();
    }
    {
        let byte_len = (width as usize) * (height as usize) * 4;
        let pixels = unsafe { std::slice::from_raw_parts_mut(pixels_ptr, byte_len) };
        let mut canvas = Canvas {
            pixels,
            width,
            height,
        };

        let pad_x = px(ROW_PAD_X_LOGICAL, scale);
        let radius = ROW_RADIUS_LOGICAL * scale;
        let border_w = scale.max(1.0);
        let icon_size = ICON_SIZE_LOGICAL * scale;
        let check_size = CHECK_ICON_SIZE_LOGICAL * scale;

        canvas.fill_all(palette.bg);
        if !data.dwm_border_ok {
            canvas.stroke_rect_1px(
                RectPx {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                },
                palette.border_strong,
            );
        }

        if data.hovered == Some(HitItem::OpenFlyout) {
            canvas.fill_rounded_rect(layout.open_row, radius, Some(palette.bg_hover), None);
        }
        draw_row_left_icon(
            &mut canvas,
            layout.open_row,
            pad_x,
            icon_size,
            IconKind::Window,
            palette.text_sec,
        );

        if data.hovered == Some(HitItem::ToggleLock) {
            canvas.fill_rounded_rect(layout.lock_row, radius, Some(palette.bg_hover), None);
        }
        draw_row_left_icon(
            &mut canvas,
            layout.lock_row,
            pad_x,
            icon_size,
            if snapshot.lock_enabled {
                IconKind::Lock
            } else {
                IconKind::Unlock
            },
            palette.text_sec,
        );

        let pill_radius = badge_rect.height() as f32 * 0.5;
        if snapshot.lock_enabled {
            canvas.fill_rounded_rect(badge_rect, pill_radius, Some(palette.inv_bg), None);
        } else {
            canvas.fill_rounded_rect(
                badge_rect,
                pill_radius,
                None,
                Some((palette.border_strong, border_w)),
            );
        }

        let div_inset = px(OUTER_PAD_LOGICAL + DIVIDER_INSET_X_LOGICAL, scale);
        let div_thickness = scale.round().max(1.0) as i32;
        canvas.fill_rect(
            RectPx {
                left: div_inset,
                top: layout.divider_1_y,
                right: width - div_inset,
                bottom: layout.divider_1_y + div_thickness,
            },
            palette.divider,
        );

        for row in &layout.profile_rows {
            if row.rect.bottom <= layout.profiles_clip.top
                || row.rect.top >= layout.profiles_clip.bottom
            {
                continue;
            }
            let hovered = matches!(&data.hovered, Some(HitItem::Profile(id)) if id == &row.id);
            let fill = if hovered {
                Some(palette.bg_hover)
            } else if row.active {
                Some(palette.bg_elev)
            } else {
                None
            };
            let border = row.active.then_some((palette.border, border_w));
            if fill.is_some() || border.is_some() {
                canvas.fill_rounded_rect_clipped(
                    row.rect,
                    layout.profiles_clip,
                    radius,
                    fill,
                    border,
                );
            }
            if row.active {
                let cx = row.rect.right as f32 - pad_x as f32 - check_size;
                let cy = row.rect.top as f32 + (row.rect.height() as f32 - check_size) * 0.5;
                canvas.draw_icon_clipped(
                    IconKind::Check,
                    cx,
                    cy,
                    check_size,
                    palette.text_sec,
                    layout.profiles_clip,
                );
            }
        }

        canvas.fill_rect(
            RectPx {
                left: div_inset,
                top: layout.divider_2_y,
                right: width - div_inset,
                bottom: layout.divider_2_y + div_thickness,
            },
            palette.divider,
        );

        if data.confirm_quit {
            canvas.fill_rounded_rect(layout.quit_row, radius, Some(palette.inv_bg), None);
        } else if data.hovered == Some(HitItem::Quit) {
            canvas.fill_rounded_rect(layout.quit_row, radius, Some(palette.bg_hover), None);
        }
        draw_row_left_icon(
            &mut canvas,
            layout.quit_row,
            pad_x,
            icon_size,
            IconKind::Power,
            if data.confirm_quit {
                palette.inv_text
            } else {
                palette.text_sec
            },
        );
    }

    let old_font = unsafe { SelectObject(mem_dc, HGDIOBJ(fonts.regular.0)) };

    let pad_x = px(ROW_PAD_X_LOGICAL, scale);
    let icon_advance = px(ICON_SIZE_LOGICAL + ICON_TEXT_GAP_LOGICAL, scale);
    let check_reserve = px(CHECK_ICON_SIZE_LOGICAL + ICON_TEXT_GAP_LOGICAL, scale);

    draw_text_line(
        mem_dc,
        fonts.regular,
        "Open Audio Lock",
        RectPx {
            left: layout.open_row.left + pad_x + icon_advance,
            top: layout.open_row.top,
            right: layout.open_row.right - pad_x,
            bottom: layout.open_row.bottom,
        },
        palette.text,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
    );

    draw_text_line(
        mem_dc,
        fonts.regular,
        "Lock Active",
        RectPx {
            left: layout.lock_row.left + pad_x + icon_advance,
            top: layout.lock_row.top,
            right: badge_rect.left - px(6.0, scale),
            bottom: layout.lock_row.bottom,
        },
        palette.text,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
    );

    draw_text_line(
        mem_dc,
        fonts.badge,
        badge_text,
        badge_rect,
        if snapshot.lock_enabled {
            palette.inv_text
        } else {
            palette.text_muted
        },
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );

    draw_text_line(
        mem_dc,
        fonts.header,
        "PROFILES",
        RectPx {
            left: layout.header_rect.left + pad_x,
            top: layout.header_rect.top + px(2.0, scale),
            right: layout.header_rect.right - pad_x,
            bottom: layout.header_rect.bottom,
        },
        palette.text_muted,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );

    let saved = unsafe { SaveDC(mem_dc) };
    unsafe {
        let _ = IntersectClipRect(
            mem_dc,
            layout.profiles_clip.left,
            layout.profiles_clip.top,
            layout.profiles_clip.right,
            layout.profiles_clip.bottom,
        );
    }

    if let Some(empty_rect) = layout.empty_profiles_rect {
        draw_text_line(
            mem_dc,
            fonts.profile_regular,
            "No profiles",
            RectPx {
                left: empty_rect.left + pad_x,
                top: empty_rect.top,
                right: empty_rect.right - pad_x,
                bottom: empty_rect.bottom,
            },
            palette.text_muted,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
    } else {
        for row in &layout.profile_rows {
            if row.rect.bottom <= layout.profiles_clip.top
                || row.rect.top >= layout.profiles_clip.bottom
            {
                continue;
            }
            let hovered = matches!(&data.hovered, Some(HitItem::Profile(id)) if id == &row.id);
            let font = if row.active {
                fonts.profile_active
            } else {
                fonts.profile_regular
            };
            let color = if row.active || hovered {
                palette.text
            } else {
                palette.text_sec
            };
            let right_inset = if row.active {
                pad_x + check_reserve
            } else {
                pad_x
            };
            draw_text_line(
                mem_dc,
                font,
                &row.name,
                RectPx {
                    left: row.rect.left + pad_x,
                    top: row.rect.top,
                    right: row.rect.right - right_inset,
                    bottom: row.rect.bottom,
                },
                color,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }
    }

    if saved != 0 {
        unsafe {
            let _ = RestoreDC(mem_dc, saved);
        }
    }

    let (quit_font, quit_label, quit_text_color) = if data.confirm_quit {
        (fonts.semibold, "Confirm Quit?", palette.inv_text)
    } else {
        (
            fonts.regular,
            "Quit",
            if data.hovered == Some(HitItem::Quit) {
                palette.text
            } else {
                palette.text_sec
            },
        )
    };
    draw_text_line(
        mem_dc,
        quit_font,
        quit_label,
        RectPx {
            left: layout.quit_row.left + pad_x + icon_advance,
            top: layout.quit_row.top,
            right: layout.quit_row.right - pad_x,
            bottom: layout.quit_row.bottom,
        },
        quit_text_color,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
    );

    unsafe {
        let _ = BitBlt(hdc, 0, 0, width, height, Some(mem_dc), 0, 0, SRCCOPY);
        SelectObject(mem_dc, old_font);
        let _ = EndPaint(hwnd, &ps);
    }
}

fn create_ui_font(logical_px: f32, scale: f32, weight: i32) -> HFONT {
    let height = -((logical_px * scale).round() as i32).max(1);
    let face = win32::to_wide("Segoe UI");
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
            PCWSTR(face.as_ptr()),
        )
    }
}

fn compute_badge_rect(
    hdc: HDC,
    font: HFONT,
    text: &str,
    row: RectPx,
    scale: f32,
) -> RectPx {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    unsafe {
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
        SelectObject(hdc, old);
    }
    let pill_h = px(PILL_HEIGHT_LOGICAL, scale);
    let pad_x = px(PILL_PAD_X_LOGICAL, scale);
    let min_w = px(30.0, scale);
    let pill_w = (size.cx + pad_x * 2).max(min_w);
    let right = row.right - px(ROW_PAD_X_LOGICAL, scale);
    let top = row.top + (row.height() - pill_h) / 2;
    RectPx {
        left: right - pill_w,
        top,
        right,
        bottom: top + pill_h,
    }
}

fn draw_text_line(
    hdc: HDC,
    font: HFONT,
    text: &str,
    rect: RectPx,
    color: Rgb,
    flags: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
) {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return;
    }
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut win_rect = rect.to_win32();
    unsafe {
        SelectObject(hdc, HGDIOBJ(font.0));
        SetTextColor(hdc, color.colorref());
        let _ = DrawTextW(hdc, &mut wide, &mut win_rect, flags);
    }
}

fn draw_row_left_icon(
    canvas: &mut Canvas<'_>,
    row: RectPx,
    pad_x: i32,
    icon_size: f32,
    kind: IconKind,
    color: Rgb,
) {
    let ix = (row.left + pad_x) as f32;
    let iy = row.top as f32 + (row.height() as f32 - icon_size) * 0.5;
    canvas.draw_icon(kind, ix, iy, icon_size, color);
}

struct Canvas<'a> {
    pixels: &'a mut [u8],
    width: i32,
    height: i32,
}

impl Canvas<'_> {
    fn fill_all(&mut self, color: Rgb) {
        let (chunks, _) = self.pixels.as_chunks_mut::<4>();
        for chunk in chunks {
            chunk[0] = color.2;
            chunk[1] = color.1;
            chunk[2] = color.0;
            chunk[3] = 255;
        }
    }

    fn blend_pixel(&mut self, x: i32, y: i32, color: Rgb, alpha: f32) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height || alpha <= 0.0 {
            return;
        }
        let idx = ((y * self.width + x) * 4) as usize;
        if alpha >= 1.0 {
            self.pixels[idx] = color.2;
            self.pixels[idx + 1] = color.1;
            self.pixels[idx + 2] = color.0;
            self.pixels[idx + 3] = 255;
            return;
        }
        let inv = 1.0 - alpha;
        self.pixels[idx] = (f32::from(color.2) * alpha + f32::from(self.pixels[idx]) * inv)
            .round() as u8;
        self.pixels[idx + 1] = (f32::from(color.1) * alpha + f32::from(self.pixels[idx + 1]) * inv)
            .round() as u8;
        self.pixels[idx + 2] = (f32::from(color.0) * alpha + f32::from(self.pixels[idx + 2]) * inv)
            .round() as u8;
        self.pixels[idx + 3] = 255;
    }

    fn fill_rect(&mut self, rect: RectPx, color: Rgb) {
        let x0 = rect.left.max(0);
        let y0 = rect.top.max(0);
        let x1 = rect.right.min(self.width);
        let y1 = rect.bottom.min(self.height);
        for y in y0..y1 {
            for x in x0..x1 {
                let idx = ((y * self.width + x) * 4) as usize;
                self.pixels[idx] = color.2;
                self.pixels[idx + 1] = color.1;
                self.pixels[idx + 2] = color.0;
                self.pixels[idx + 3] = 255;
            }
        }
    }

    fn stroke_rect_1px(&mut self, rect: RectPx, color: Rgb) {
        self.fill_rect(
            RectPx {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.top + 1,
            },
            color,
        );
        self.fill_rect(
            RectPx {
                left: rect.left,
                top: rect.bottom - 1,
                right: rect.right,
                bottom: rect.bottom,
            },
            color,
        );
        self.fill_rect(
            RectPx {
                left: rect.left,
                top: rect.top,
                right: rect.left + 1,
                bottom: rect.bottom,
            },
            color,
        );
        self.fill_rect(
            RectPx {
                left: rect.right - 1,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
            color,
        );
    }

    fn fill_rounded_rect(
        &mut self,
        rect: RectPx,
        radius: f32,
        fill: Option<Rgb>,
        border: Option<(Rgb, f32)>,
    ) {
        let clip = RectPx {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height,
        };
        self.fill_rounded_rect_clipped(rect, clip, radius, fill, border);
    }

    fn fill_rounded_rect_clipped(
        &mut self,
        rect: RectPx,
        clip: RectPx,
        radius: f32,
        fill: Option<Rgb>,
        border: Option<(Rgb, f32)>,
    ) {
        let w = rect.width() as f32;
        let h = rect.height() as f32;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let cx = rect.left as f32 + w * 0.5;
        let cy = rect.top as f32 + h * 0.5;
        let hx = w * 0.5;
        let hy = h * 0.5;
        let r = radius.min(hx).min(hy);

        let x0 = rect.left.max(clip.left).max(0);
        let y0 = rect.top.max(clip.top).max(0);
        let x1 = rect.right.min(clip.right).min(self.width);
        let y1 = rect.bottom.min(clip.bottom).min(self.height);

        for y in y0..y1 {
            let py = y as f32 + 0.5;
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let dist = rounded_box_sdf(px, py, cx, cy, hx, hy, r);
                let outer = (0.5 - dist).clamp(0.0, 1.0);
                if outer <= 0.0 {
                    continue;
                }
                match (fill, border) {
                    (Some(fill_col), Some((border_col, bw))) => {
                        let inner = (0.5 - (dist + bw)).clamp(0.0, 1.0);
                        let ring = (outer - inner).clamp(0.0, 1.0);
                        if inner > 0.0 {
                            self.blend_pixel(x, y, fill_col, inner);
                        }
                        if ring > 0.0 {
                            self.blend_pixel(x, y, border_col, ring);
                        }
                    }
                    (Some(fill_col), None) => {
                        self.blend_pixel(x, y, fill_col, outer);
                    }
                    (None, Some((border_col, bw))) => {
                        let inner = (0.5 - (dist + bw)).clamp(0.0, 1.0);
                        let ring = (outer - inner).clamp(0.0, 1.0);
                        if ring > 0.0 {
                            self.blend_pixel(x, y, border_col, ring);
                        }
                    }
                    (None, None) => {}
                }
            }
        }
    }

    fn draw_icon(&mut self, kind: IconKind, ox: f32, oy: f32, size: f32, color: Rgb) {
        let clip = RectPx {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height,
        };
        self.draw_icon_clipped(kind, ox, oy, size, color, clip);
    }

    fn draw_icon_clipped(
        &mut self,
        kind: IconKind,
        ox: f32,
        oy: f32,
        size: f32,
        color: Rgb,
        clip: RectPx,
    ) {
        if size <= 0.0 {
            return;
        }
        let x0 = (ox.floor() as i32).max(clip.left).max(0);
        let y0 = (oy.floor() as i32).max(clip.top).max(0);
        let x1 = ((ox + size).ceil() as i32).min(clip.right).min(self.width);
        let y1 = ((oy + size).ceil() as i32).min(clip.bottom).min(self.height);
        let inv_size = 16.0 / size;
        let samples = 4usize;
        let total = (samples * samples) as f32;

        for y in y0..y1 {
            for x in x0..x1 {
                let mut hits = 0.0f32;
                for sy in 0..samples {
                    let wy = (y as f32 + (sy as f32 + 0.5) / samples as f32 - oy) * inv_size;
                    for sx in 0..samples {
                        let wx = (x as f32 + (sx as f32 + 0.5) / samples as f32 - ox) * inv_size;
                        if sample_icon(kind, wx, wy) {
                            hits += 1.0;
                        }
                    }
                }
                if hits > 0.0 {
                    self.blend_pixel(x, y, color, hits / total);
                }
            }
        }
    }
}

fn rounded_box_sdf(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hx - r);
    let qy = (py - cy).abs() - (hy - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let abx = bx - ax;
    let aby = by - ay;
    let len_sq = abx * abx + aby * aby;
    let t = if len_sq > 0.0 {
        ((px - ax) * abx + (py - ay) * aby) / len_sq
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
    (px - (ax + t * abx)).hypot(py - (ay + t * aby))
}

fn sample_icon(kind: IconKind, x: f32, y: f32) -> bool {
    match kind {
        IconKind::Window => {
            let d_box = rounded_box_sdf(x, y, 8.0, 8.0, 5.7, 5.2, 2.0).abs();
            let d_bar = dist_to_segment(x, y, 2.3, 5.8, 13.7, 5.8);
            d_box.min(d_bar) <= 0.78
        }
        IconKind::Lock => {
            let d_body = rounded_box_sdf(x, y, 8.0, 10.5, 5.0, 3.5, 1.8).abs();
            let d_left = dist_to_segment(x, y, 5.2, 5.0, 5.2, 7.0);
            let d_right = dist_to_segment(x, y, 10.8, 5.0, 10.8, 7.0);
            let d_arc = if y <= 5.0 {
                ((x - 8.0).hypot(y - 5.0) - 2.8).abs()
            } else {
                (x - 5.2)
                    .hypot(y - 5.0)
                    .min((x - 10.8).hypot(y - 5.0))
            };
            d_body.min(d_left).min(d_right).min(d_arc) <= 0.78
        }
        IconKind::Unlock => {
            let d_body = rounded_box_sdf(x, y, 8.0, 10.5, 5.0, 3.5, 1.8).abs();
            let d_left = dist_to_segment(x, y, 5.2, 5.0, 5.2, 7.0);
            let end_angle = -0.45f32;
            let end_x = 8.0 + 2.8 * end_angle.cos();
            let end_y = 5.0 + 2.8 * end_angle.sin();
            let angle = (y - 5.0).atan2(x - 8.0);
            let d_arc = if angle <= end_angle && angle >= -PI {
                ((x - 8.0).hypot(y - 5.0) - 2.8).abs()
            } else {
                (x - 5.2)
                    .hypot(y - 5.0)
                    .min((x - end_x).hypot(y - end_y))
            };
            d_body.min(d_left).min(d_arc) <= 0.78
        }
        IconKind::Check => {
            let d1 = dist_to_segment(x, y, 3.2, 8.4, 6.5, 11.7);
            let d2 = dist_to_segment(x, y, 6.5, 11.7, 13.0, 4.8);
            d1.min(d2) <= 0.95
        }
        IconKind::Power => {
            let d_stem = dist_to_segment(x, y, 8.0, 2.4, 8.0, 7.2);
            let cy = 8.33f32;
            let r = 5.2f32;
            let angle = (y - cy).atan2(x - 8.0);
            let d_arc = if !(-2.284..=-0.858).contains(&angle) {
                ((x - 8.0).hypot(y - cy) - r).abs()
            } else {
                (x - 4.6)
                    .hypot(y - 4.4)
                    .min((x - 11.4).hypot(y - 4.4))
            };
            d_stem.min(d_arc) <= 0.78
        }
    }
}
