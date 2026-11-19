use serde::{Deserialize, Serialize};

use crate::model::{AppState, DeviceFlow, RightClickAction, ThemeMode};

pub const PIPE_NAME: &str = r"\\.\pipe\default-audio-lock-ipc";
pub const TRAY_MUTEX_NAME: &str = "Local\\DefaultAudioLockTrayDaemon";
pub const UI_MUTEX_NAME: &str = "Local\\DefaultAudioLockUiInstance";
pub const UI_CLOSE_EVENT_NAME: &str = "Local\\DefaultAudioLockUiCloseEvent";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppAction {
    SetLockEnabled(bool),
    SetTheme(ThemeMode),
    SetSeparateCommsRole(bool),
    SetRightClickAction(RightClickAction),
    SetRunOnStartup(bool),
    SetAutoUpdate(bool),
    CheckForUpdatesNow,
    ApplyStagedUpdateNow,
    SetAutoDisableEnhancements(bool),
    ReenableAllEnhancements,
    SetAutoDisableUnusedDevices(bool),
    SetDeviceDisabled {
        device_id: String,
        disabled: bool,
    },
    ReenableAllDevices,
    SetDefault {
        flow: DeviceFlow,
        device_id: String,
    },
    SetComms {
        flow: DeviceFlow,
        device_id: String,
    },
    SetVolume {
        device_id: String,
        volume: u8,
    },
    ToggleMute {
        device_id: String,
    },
    SetEnhancementsDisabled {
        device_id: String,
        disabled: bool,
    },
    ToggleDevicePin {
        device_id: String,
    },
    SetListenEnabled {
        device_id: String,
        enabled: bool,
    },
    SetListenTarget {
        device_id: String,
        target_device_id: Option<String>,
    },
    ApplyProfile {
        profile_id: String,
    },
    SaveProfile {
        name: String,
    },
    DeleteProfile {
        profile_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpcRequest {
    GetState,
    SyncState(AppState),
    ApplyAction(AppAction),
    Ping,
    OpenFlyout {
        anchor_x: Option<i32>,
        anchor_y: Option<i32>,
    },
    QuitDaemon,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpcResponse {
    State(AppState),
    Ok,
    Error(String),
}
