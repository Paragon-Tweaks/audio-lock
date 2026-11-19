#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod flyout;
mod icon;
mod ipc;
mod popup_menu;
mod startup;
mod updater;
mod win32;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use audio_lock_core::ipc::TRAY_MUTEX_NAME;
use audio_lock_core::model::{AppState, RightClickAction};
use audio_lock_core::store;
use audio_lock_core::update::IDLE_RESTART_MS;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, Rect, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage, WM_TIMER, WM_USER,
};

use crate::audio::AudioEngine;
use crate::flyout::{FlyoutController, FlyoutEvent};
use crate::ipc::IpcContext;
use crate::popup_menu::{PopupAction, PopupMenu};
use crate::updater::Updater;
use crate::win32::{
    WM_APP_APPLY_UPDATE, WM_APP_CHECK_UPDATES, WM_APP_OPEN_FLYOUT, WM_APP_QUIT,
    WM_APP_REFRESH_TRAY, WM_APP_THEME_CHANGED, WM_APP_TRAY_EVENT, WM_APP_UPDATE_STAGED,
};

const POST_UPDATE_MUTEX_WAIT_MS: u64 = 20_000;
const REOPEN_DEBOUNCE: Duration = Duration::from_millis(350);

enum TrayClick {
    Left,
    Right(Rect),
}

struct TrayVisualCache {
    muted: bool,
    tooltip: String,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let is_post_update = args.iter().any(|arg| arg == "--post-update");
    let wait_pid = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--wait-pid="))
        .and_then(|raw| raw.parse::<u32>().ok());
    if let Some(pid) = wait_pid {
        win32::wait_for_pid_exit(pid, POST_UPDATE_MUTEX_WAIT_MS);
    }

    let Some(guard) = (if is_post_update {
        win32::SingleInstanceGuard::acquire_with_wait(TRAY_MUTEX_NAME, POST_UPDATE_MUTEX_WAIT_MS)
    } else {
        win32::SingleInstanceGuard::acquire(TRAY_MUTEX_NAME)
    }) else {
        return;
    };
    let mut instance_guard = Some(guard);

    win32::enable_dpi_awareness();
    win32::configure_low_impact_process();

    let state = Arc::new(Mutex::new(store::load_state()));

    let Ok(msg_window) = win32::MessageWindow::create() else {
        return;
    };
    let wake = msg_window.wake_handle();
    let updater = Updater::new(Arc::clone(&state), wake);
    let audio = Arc::new(AudioEngine::start(Arc::clone(&state), wake));
    let popup_menu = PopupMenu::new(Arc::clone(&state), wake);
    let mut flyout = FlyoutController::new(Arc::clone(&state), Arc::clone(&audio), wake);

    let (tray_tx, tray_rx) = mpsc::channel::<TrayClick>();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if let TrayIconEvent::Click {
            button,
            button_state: MouseButtonState::Up,
            rect,
            ..
        } = event
        {
            let click = match button {
                MouseButton::Left => TrayClick::Left,
                MouseButton::Right => TrayClick::Right(rect),
                _ => return,
            };
            if tray_tx.send(click).is_ok() {
                wake.post(WM_APP_TRAY_EVENT);
            }
        }
    }));

    let mut light_taskbar = win32::is_system_light_taskbar();
    let mut unmuted_icon = icon::unmuted_mic_icon(light_taskbar);
    let mut muted_icon = icon::muted_mic_icon(light_taskbar);

    let initial_state = state.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let initial_muted = initial_state.are_active_mics_muted();
    let initial_tooltip = build_tooltip(&initial_state);
    let Ok(tray) = TrayIconBuilder::new()
        .with_icon(if initial_muted {
            muted_icon.clone()
        } else {
            unmuted_icon.clone()
        })
        .with_tooltip(initial_tooltip.clone())
        .with_menu_on_left_click(false)
        .with_menu_on_right_click(false)
        .build()
    else {
        return;
    };
    let mut tray_cache = TrayVisualCache {
        muted: initial_muted,
        tooltip: initial_tooltip,
    };

    let initial_anchor = tray
        .rect()
        .map(anchor_from_rect)
        .or_else(win32::cursor_position)
        .unwrap_or((0, 0));
    let last_anchor = Arc::new(Mutex::new(initial_anchor));
    let mut tray_icon = Some(tray);

    let open_flyout_pending = Arc::new(AtomicBool::new(false));
    let quit_pending = Arc::new(AtomicBool::new(false));
    let check_updates_pending = Arc::new(AtomicBool::new(false));
    let apply_update_pending = Arc::new(AtomicBool::new(false));

    ipc::spawn_server(IpcContext {
        state: Arc::clone(&state),
        audio: Arc::clone(&audio),
        last_anchor: Arc::clone(&last_anchor),
        open_flyout_pending: Arc::clone(&open_flyout_pending),
        quit_pending: Arc::clone(&quit_pending),
        check_updates_pending: Arc::clone(&check_updates_pending),
        apply_update_pending: Arc::clone(&apply_update_pending),
        wake,
    });
    ipc::spawn_quit_event_listener(Arc::clone(&quit_pending), wake);

    win32::set_timer(
        msg_window.hwnd(),
        updater::TIMER_UPDATE_INITIAL,
        updater::INITIAL_CHECK_DELAY_MS,
    );
    let mut idle_timer_armed = false;
    let mut idle_tracker = win32::ControllerIdleTracker::new();
    sync_idle_timer(
        msg_window.hwnd(),
        &updater,
        initial_state.auto_update,
        &mut idle_timer_armed,
        &mut idle_tracker,
    );
    win32::trim_working_set();

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        if msg.hwnd != msg_window.hwnd() || msg.message < WM_USER {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if msg.hwnd == msg_window.hwnd() && msg.message == WM_TIMER {
            match msg.wParam.0 {
                updater::TIMER_UPDATE_INITIAL => {
                    win32::kill_timer(msg_window.hwnd(), updater::TIMER_UPDATE_INITIAL);
                    win32::set_timer(
                        msg_window.hwnd(),
                        updater::TIMER_UPDATE_PERIODIC,
                        updater::PERIODIC_CHECK_INTERVAL_MS,
                    );
                    updater.spawn_check(false);
                }
                updater::TIMER_UPDATE_PERIODIC => {
                    updater.spawn_check(false);
                }
                updater::TIMER_IDLE_CHECK => {
                    let auto_update = state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .auto_update;
                    if !updater.has_staged() || !auto_update || !updater.can_auto_apply() {
                        disarm_idle_timer(
                            msg_window.hwnd(),
                            &mut idle_timer_armed,
                            &mut idle_tracker,
                        );
                    } else if idle_tracker.sample_and_idle_ms() >= IDLE_RESTART_MS
                        && !win32::is_fullscreen_or_busy()
                        && !flyout.is_open()
                        && !popup_menu.is_open()
                    {
                        if updater.try_apply_staged(false) {
                            disarm_idle_timer(
                                msg_window.hwnd(),
                                &mut idle_timer_armed,
                                &mut idle_tracker,
                            );
                            quit_pending.store(true, Ordering::Release);
                        } else {
                            sync_idle_timer(
                                msg_window.hwnd(),
                                &updater,
                                auto_update,
                                &mut idle_timer_armed,
                                &mut idle_tracker,
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        if msg.hwnd == msg_window.hwnd() && msg.message == WM_APP_THEME_CHANGED {
            let current_light = win32::is_system_light_taskbar();
            if current_light != light_taskbar {
                light_taskbar = current_light;
                unmuted_icon = icon::unmuted_mic_icon(light_taskbar);
                muted_icon = icon::muted_mic_icon(light_taskbar);
                if let Some(tray) = tray_icon.as_ref() {
                    let icon = if tray_cache.muted {
                        muted_icon.clone()
                    } else {
                        unmuted_icon.clone()
                    };
                    let _ = tray.set_icon(Some(icon));
                }
            }
        }

        if msg.hwnd == msg_window.hwnd() && msg.message == WM_APP_UPDATE_STAGED {
            let auto_update = state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .auto_update;
            sync_idle_timer(
                msg_window.hwnd(),
                &updater,
                auto_update,
                &mut idle_timer_armed,
                &mut idle_tracker,
            );
            if flyout.is_open() {
                flyout.invalidate();
            }
        }

        if check_updates_pending.swap(false, Ordering::AcqRel)
            || (msg.hwnd == msg_window.hwnd() && msg.message == WM_APP_CHECK_UPDATES)
        {
            updater.spawn_check(true);
        }

        for event in flyout.take_events() {
            match event {
                FlyoutEvent::ApplyStagedUpdate => {
                    apply_update_pending.store(true, Ordering::Release);
                }
                FlyoutEvent::Quit => {
                    quit_pending.store(true, Ordering::Release);
                }
            }
        }

        if apply_update_pending.swap(false, Ordering::AcqRel)
            || (msg.hwnd == msg_window.hwnd() && msg.message == WM_APP_APPLY_UPDATE)
        {
            if updater.try_apply_staged(true) {
                disarm_idle_timer(msg_window.hwnd(), &mut idle_timer_armed, &mut idle_tracker);
                quit_pending.store(true, Ordering::Release);
            } else {
                if !updater.has_staged() {
                    disarm_idle_timer(msg_window.hwnd(), &mut idle_timer_armed, &mut idle_tracker);
                }
                if flyout.is_open() {
                    flyout.flash_status("Update failed - retry later");
                }
            }
        }

        for action in popup_menu.take_actions() {
            match action {
                PopupAction::OpenFlyout(anchor) => {
                    *last_anchor.lock().unwrap_or_else(|e| e.into_inner()) = anchor;
                    popup_menu.close();
                    flyout.open(anchor);
                }
                PopupAction::ApplyAction(app_action) => {
                    let snapshot = audio.apply_action(app_action);
                    sync_idle_timer(
                        msg_window.hwnd(),
                        &updater,
                        snapshot.auto_update,
                        &mut idle_timer_armed,
                        &mut idle_tracker,
                    );
                    if let Some(tray) = tray_icon.as_ref() {
                        refresh_tray(tray, &mut tray_cache, &snapshot, &unmuted_icon, &muted_icon);
                    }
                    if popup_menu.is_open() {
                        popup_menu.invalidate();
                    }
                    if flyout.is_open() {
                        flyout.invalidate();
                    }
                }
                PopupAction::Quit => {
                    quit_pending.store(true, Ordering::Release);
                }
            }
        }

        if quit_pending.swap(false, Ordering::AcqRel)
            || (msg.hwnd == msg_window.hwnd() && msg.message == WM_APP_QUIT)
        {
            win32::kill_timer(msg_window.hwnd(), updater::TIMER_UPDATE_INITIAL);
            win32::kill_timer(msg_window.hwnd(), updater::TIMER_UPDATE_PERIODIC);
            disarm_idle_timer(msg_window.hwnd(), &mut idle_timer_armed, &mut idle_tracker);
            flyout.close();
            popup_menu.close();
            audio.shutdown();
            drop(tray_icon.take());
            win32::post_quit();
            continue;
        }

        if msg.hwnd == msg_window.hwnd() && msg.message == WM_APP_REFRESH_TRAY {
            let snapshot = state.lock().unwrap_or_else(|e| e.into_inner()).clone();
            sync_idle_timer(
                msg_window.hwnd(),
                &updater,
                snapshot.auto_update,
                &mut idle_timer_armed,
                &mut idle_tracker,
            );
            if let Some(tray) = tray_icon.as_ref() {
                refresh_tray(tray, &mut tray_cache, &snapshot, &unmuted_icon, &muted_icon);
            }
            if flyout.is_open() {
                flyout.invalidate();
            }
            if popup_menu.is_open() {
                popup_menu.invalidate();
            }
        }

        while let Ok(click) = tray_rx.try_recv() {
            match click {
                TrayClick::Left => {
                    if popup_menu.is_open() {
                        popup_menu.close();
                    }
                    let snapshot = audio.toggle_active_mics_mute();
                    if let Some(tray) = tray_icon.as_ref() {
                        refresh_tray(tray, &mut tray_cache, &snapshot, &unmuted_icon, &muted_icon);
                    }
                    if flyout.is_open() {
                        flyout.invalidate();
                    }
                }
                TrayClick::Right(rect) => {
                    let anchor = anchor_from_rect(rect);
                    *last_anchor.lock().unwrap_or_else(|e| e.into_inner()) = anchor;
                    let action = state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .right_click_action;
                    match action {
                        RightClickAction::Menu => {
                            if popup_menu.is_open() {
                                popup_menu.close();
                            } else if !popup_menu.closed_within(REOPEN_DEBOUNCE) {
                                flyout.close();
                                popup_menu.open(anchor);
                            }
                        }
                        RightClickAction::Flyout => {
                            popup_menu.close();
                            flyout.open_or_toggle(anchor);
                        }
                    }
                }
            }
        }

        if open_flyout_pending.swap(false, Ordering::AcqRel)
            || (msg.hwnd == msg_window.hwnd() && msg.message == WM_APP_OPEN_FLYOUT)
        {
            popup_menu.close();
            let mut anchor = *last_anchor.lock().unwrap_or_else(|e| e.into_inner());
            if anchor == (0, 0) {
                anchor = tray_icon
                    .as_ref()
                    .and_then(TrayIcon::rect)
                    .map(anchor_from_rect)
                    .or_else(win32::cursor_position)
                    .unwrap_or((0, 0));
            }
            flyout.open(anchor);
        }
    }

    audio.shutdown();
    flyout.close();
    popup_menu.close();
    drop(tray_icon.take());
    drop(flyout);
    drop(popup_menu);
    drop(instance_guard.take());
}

fn disarm_idle_timer(
    hwnd: HWND,
    armed: &mut bool,
    idle_tracker: &mut win32::ControllerIdleTracker,
) {
    win32::kill_timer(hwnd, updater::TIMER_IDLE_CHECK);
    *armed = false;
    idle_tracker.reset();
}

fn sync_idle_timer(
    hwnd: HWND,
    updater: &Updater,
    auto_update: bool,
    armed: &mut bool,
    idle_tracker: &mut win32::ControllerIdleTracker,
) {
    let should_arm = auto_update && updater.has_staged() && updater.can_auto_apply();
    if should_arm != *armed {
        if should_arm {
            win32::set_timer(hwnd, updater::TIMER_IDLE_CHECK, updater::IDLE_CHECK_INTERVAL_MS);
            *armed = true;
        } else {
            disarm_idle_timer(hwnd, armed, idle_tracker);
        }
    }
}

fn anchor_from_rect(rect: Rect) -> (i32, i32) {
    let x = (rect.position.x + f64::from(rect.size.width) / 2.0).round() as i32;
    let y = rect.position.y.round() as i32;
    (x, y)
}

fn refresh_tray(
    tray: &TrayIcon,
    cache: &mut TrayVisualCache,
    state: &AppState,
    unmuted_icon: &Icon,
    muted_icon: &Icon,
) {
    let muted = state.are_active_mics_muted();
    if cache.muted != muted {
        cache.muted = muted;
        let icon = if muted {
            muted_icon.clone()
        } else {
            unmuted_icon.clone()
        };
        let _ = tray.set_icon(Some(icon));
    }
    let tooltip = build_tooltip(state);
    if cache.tooltip != tooltip {
        let _ = tray.set_tooltip(Some(tooltip.clone()));
        cache.tooltip = tooltip;
    }
}

fn build_tooltip(state: &AppState) -> String {
    let mic_status = if state.are_active_mics_muted() {
        "Mic Muted"
    } else {
        "Mic Live"
    };
    if state.lock_enabled {
        match state.active_profile() {
            Some(profile) => format!("Audio Lock - {mic_status} · Locked ({})", profile.name),
            None => format!("Audio Lock - {mic_status} · Locked"),
        }
    } else {
        format!("Audio Lock - {mic_status} · Unlocked")
    }
}
