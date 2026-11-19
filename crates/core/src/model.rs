use serde::{Deserialize, Serialize};

use crate::ipc::AppAction;

pub const LISTEN_TARGET_DEFAULT_OUTPUT: &str = "__default_output__";
pub const LISTEN_TARGET_COMMS_OUTPUT: &str = "__comms_output__";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceFlow {
    Output,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RightClickAction {
    #[default]
    Menu,
    Flyout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnhancementLayer {
    SysFx,
    Advanced,
    Spatial,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EnhancementsState {
    pub sys_fx_enabled: bool,
    pub advanced_enabled: bool,
    pub spatial_sound_enabled: bool,
    #[serde(default)]
    pub previous_spatial_format: Option<String>,
}

impl EnhancementsState {
    pub fn any_enabled(&self) -> bool {
        self.sys_fx_enabled || self.advanced_enabled || self.spatial_sound_enabled
    }

    pub fn disable_all(&mut self) {
        self.sys_fx_enabled = false;
        self.advanced_enabled = false;
        self.spatial_sound_enabled = false;
    }

    pub fn all_disabled() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ListenConfig {
    pub enabled: bool,
    pub target_device_id: Option<String>,
}

impl ListenConfig {
    pub fn is_actively_routing(&self) -> bool {
        self.enabled && self.target_device_id.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub flow: DeviceFlow,
    pub volume: u8,
    pub muted: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub disabled: bool,
    pub enhancements: EnhancementsState,
    #[serde(default)]
    pub previous_enhancements: Option<EnhancementsState>,
    pub listen: Option<ListenConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSnapshot {
    pub device_id: String,
    pub volume: u8,
    pub muted: bool,
    pub enhancements: EnhancementsState,
    pub listen: Option<ListenConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RoleAssignments {
    pub default_output_id: String,
    pub comms_output_id: String,
    pub default_input_id: String,
    pub comms_input_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioProfile {
    pub id: String,
    pub name: String,
    pub roles: RoleAssignments,
    pub device_settings: Vec<DeviceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppState {
    pub lock_enabled: bool,
    pub theme: ThemeMode,
    #[serde(default)]
    pub separate_comms_role: bool,
    #[serde(default)]
    pub right_click_action: RightClickAction,
    #[serde(default)]
    pub run_on_startup: bool,
    #[serde(default = "default_true")]
    pub auto_update: bool,
    #[serde(default)]
    pub staged_update_version: Option<String>,
    #[serde(default)]
    pub auto_disable_enhancements: bool,
    #[serde(default)]
    pub auto_disable_unused_devices: bool,
    pub active_profile_id: Option<String>,
    pub roles: RoleAssignments,
    pub devices: Vec<AudioDevice>,
    pub profiles: Vec<AudioProfile>,
}

fn default_true() -> bool {
    true
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            lock_enabled: true,
            theme: ThemeMode::Dark,
            separate_comms_role: false,
            right_click_action: RightClickAction::default(),
            run_on_startup: false,
            auto_update: true,
            staged_update_version: None,
            auto_disable_enhancements: false,
            auto_disable_unused_devices: false,
            active_profile_id: None,
            roles: RoleAssignments::default(),
            devices: Vec::new(),
            profiles: Vec::new(),
        }
    }
}

impl AppState {
    pub fn apply_action(&mut self, action: &AppAction) {
        match action {
            AppAction::SetLockEnabled(enabled) => self.lock_enabled = *enabled,
            AppAction::SetTheme(theme) => self.theme = *theme,
            AppAction::SetSeparateCommsRole(enabled) => {
                self.separate_comms_role = *enabled;
                if !*enabled {
                    self.unify_comms_roles();
                    self.sync_auto_disabled_flags();
                }
            }
            AppAction::SetRightClickAction(right_click) => self.right_click_action = *right_click,
            AppAction::SetRunOnStartup(enabled) => self.run_on_startup = *enabled,
            AppAction::SetAutoUpdate(enabled) => self.auto_update = *enabled,
            AppAction::CheckForUpdatesNow | AppAction::ApplyStagedUpdateNow => {}
            AppAction::SetAutoDisableEnhancements(enabled) => {
                self.set_auto_disable_enhancements(*enabled);
            }
            AppAction::ReenableAllEnhancements => self.reenable_all_enhancements(),
            AppAction::SetAutoDisableUnusedDevices(enabled) => {
                self.set_auto_disable_unused_devices(*enabled);
            }
            AppAction::SetDeviceDisabled {
                device_id,
                disabled,
            } => {
                if !self.auto_disable_unused_devices {
                    self.set_device_disabled(device_id, *disabled);
                }
            }
            AppAction::ReenableAllDevices => self.reenable_all_devices(),
            AppAction::SetDefault { flow, device_id } => self.set_default(*flow, device_id),
            AppAction::SetComms { flow, device_id } => self.set_comms(*flow, device_id),
            AppAction::SetVolume { device_id, volume } => self.set_volume(device_id, *volume),
            AppAction::ToggleMute { device_id } => self.toggle_mute(device_id),
            AppAction::SetMute { device_id, muted } => self.set_mute(device_id, *muted),
            AppAction::SetEnhancementsDisabled {
                device_id,
                disabled,
            } => self.set_enhancements_disabled(device_id, *disabled),
            AppAction::ToggleDevicePin { device_id } => self.toggle_device_pin(device_id),
            AppAction::SetDevicePinned { device_id, pinned } => {
                self.set_device_pinned(device_id, *pinned);
            }
            AppAction::SetListenEnabled { device_id, enabled } => {
                self.set_listen_enabled(device_id, *enabled);
            }
            AppAction::SetListenTarget {
                device_id,
                target_device_id,
            } => self.set_listen_target(device_id, target_device_id.clone()),
            AppAction::ApplyProfile { profile_id } => {
                self.apply_profile(profile_id);
            }
            AppAction::SaveProfile { name } => {
                self.save_current_as_profile(name);
            }
            AppAction::DeleteProfile { profile_id } => {
                self.delete_profile(profile_id);
            }
        }
    }

    pub fn reconcile_live_devices(
        &mut self,
        mut live_devices: Vec<AudioDevice>,
        live_roles: RoleAssignments,
    ) {
        for device in &mut live_devices {
            if let Some(existing) = self.device(&device.id) {
                device.pinned = existing.pinned;
                device.previous_enhancements = existing.previous_enhancements.clone();
                if device.enhancements.previous_spatial_format.is_none() {
                    device.enhancements.previous_spatial_format =
                        existing.enhancements.previous_spatial_format.clone();
                }
                if device.disabled {
                    device.volume = existing.volume;
                    device.muted = existing.muted;
                }
                if let (Some(existing_listen), Some(live_listen)) =
                    (existing.listen.as_ref(), device.listen.as_mut())
                {
                    if live_listen.enabled {
                        if existing_listen.target_device_id.as_deref()
                            == Some(LISTEN_TARGET_COMMS_OUTPUT)
                            && !self.roles.comms_output_id.is_empty()
                            && live_listen.target_device_id.as_deref()
                                == Some(self.roles.comms_output_id.as_str())
                        {
                            live_listen.target_device_id =
                                Some(LISTEN_TARGET_COMMS_OUTPUT.to_string());
                        }
                    } else if matches!(
                        existing_listen.target_device_id.as_deref(),
                        Some(LISTEN_TARGET_DEFAULT_OUTPUT | LISTEN_TARGET_COMMS_OUTPUT)
                    ) {
                        live_listen.target_device_id = existing_listen.target_device_id.clone();
                    } else if existing_listen.enabled && existing_listen.target_device_id.is_none()
                    {
                        live_listen.target_device_id = None;
                    }
                }
            }
        }

        self.devices = live_devices;
        let has_real_devices = self.devices.iter().any(|d| d.id.starts_with('{'));

        self.roles = RoleAssignments {
            default_output_id: self.reconcile_role(
                DeviceFlow::Output,
                &self.roles.default_output_id,
                &live_roles.default_output_id,
                has_real_devices,
            ),
            comms_output_id: self.reconcile_role(
                DeviceFlow::Output,
                &self.roles.comms_output_id,
                &live_roles.comms_output_id,
                has_real_devices,
            ),
            default_input_id: self.reconcile_role(
                DeviceFlow::Input,
                &self.roles.default_input_id,
                &live_roles.default_input_id,
                has_real_devices,
            ),
            comms_input_id: self.reconcile_role(
                DeviceFlow::Input,
                &self.roles.comms_input_id,
                &live_roles.comms_input_id,
                has_real_devices,
            ),
        };
        if !self.separate_comms_role {
            self.unify_comms_roles();
        }

        if has_real_devices
            && !self.profiles.is_empty()
            && self.profiles.iter().all(is_mock_only_profile)
        {
            self.profiles.clear();
            self.active_profile_id = None;
        }
    }

    pub fn set_device_audio_state(&mut self, device_id: &str, volume: u8, muted: bool) -> bool {
        let clamped = volume.min(100);
        let Some(device) = self.device_mut(device_id) else {
            return false;
        };
        if device.volume == clamped && device.muted == muted {
            return false;
        }
        device.volume = clamped;
        device.muted = muted;
        true
    }

    pub fn set_device_enhancements(
        &mut self,
        device_id: &str,
        mut enhancements: EnhancementsState,
    ) -> bool {
        let Some(device) = self.device_mut(device_id) else {
            return false;
        };
        if enhancements.previous_spatial_format.is_none() {
            enhancements.previous_spatial_format =
                device.enhancements.previous_spatial_format.clone();
        }
        if device.enhancements == enhancements {
            return false;
        }
        device.enhancements = enhancements;
        true
    }

    pub fn record_previous_spatial_format(&mut self, device_id: &str, format: String) -> bool {
        let Some(device) = self.device_mut(device_id) else {
            return false;
        };
        if device.enhancements.previous_spatial_format.is_some() {
            return false;
        }
        device.enhancements.previous_spatial_format = Some(format);
        true
    }

    pub fn clear_previous_spatial_format(&mut self, device_id: &str) -> bool {
        let Some(device) = self.device_mut(device_id) else {
            return false;
        };
        if device.enhancements.previous_spatial_format.is_none() {
            return false;
        }
        device.enhancements.previous_spatial_format = None;
        true
    }

    pub fn set_device_listen(&mut self, device_id: &str, listen: ListenConfig) -> bool {
        let Some(device) = self.device_mut(device_id) else {
            return false;
        };
        if device.flow != DeviceFlow::Input || device.listen.as_ref() == Some(&listen) {
            return false;
        }
        device.listen = Some(listen);
        true
    }

    pub fn device(&self, device_id: &str) -> Option<&AudioDevice> {
        self.devices.iter().find(|d| d.id == device_id)
    }

    pub fn has_device_with_flow(&self, flow: DeviceFlow, device_id: &str) -> bool {
        self.devices
            .iter()
            .any(|d| d.id == device_id && d.flow == flow)
    }

    pub fn set_default(&mut self, flow: DeviceFlow, device_id: &str) {
        if !self.has_device_with_flow(flow, device_id) {
            return;
        }
        match flow {
            DeviceFlow::Output => {
                self.roles.default_output_id = device_id.to_string();
                if !self.separate_comms_role {
                    self.roles.comms_output_id = device_id.to_string();
                }
            }
            DeviceFlow::Input => {
                self.roles.default_input_id = device_id.to_string();
                if !self.separate_comms_role {
                    self.roles.comms_input_id = device_id.to_string();
                }
            }
        }
        if let Some(device) = self.device_mut(device_id) {
            device.disabled = false;
        }
        if self.auto_disable_unused_devices {
            self.sync_auto_disabled_flags();
        }
    }

    pub fn set_comms(&mut self, flow: DeviceFlow, device_id: &str) {
        if !self.separate_comms_role || !self.has_device_with_flow(flow, device_id) {
            return;
        }
        match flow {
            DeviceFlow::Output => self.roles.comms_output_id = device_id.to_string(),
            DeviceFlow::Input => self.roles.comms_input_id = device_id.to_string(),
        }
        if let Some(device) = self.device_mut(device_id) {
            device.disabled = false;
        }
        if self.auto_disable_unused_devices {
            self.sync_auto_disabled_flags();
        }
    }

    pub fn set_volume(&mut self, device_id: &str, volume: u8) {
        if let Some(device) = self.device_mut(device_id) {
            device.volume = volume.min(100);
        }
    }

    pub fn toggle_mute(&mut self, device_id: &str) {
        if let Some(device) = self.device_mut(device_id) {
            device.muted = !device.muted;
        }
    }

    pub fn set_mute(&mut self, device_id: &str, muted: bool) {
        if let Some(device) = self.device_mut(device_id) {
            device.muted = muted;
        }
    }

    pub fn are_active_mics_muted(&self) -> bool {
        let mut active = self.devices.iter().filter(|d| {
            d.flow == DeviceFlow::Input
                && (d.id == self.roles.default_input_id || d.id == self.roles.comms_input_id)
        });
        let Some(first) = active.next() else {
            return false;
        };
        first.muted && active.all(|d| d.muted)
    }

    pub fn toggle_active_mics_mute(&mut self) -> bool {
        let target_muted = !self.are_active_mics_muted();
        for device in &mut self.devices {
            if device.flow == DeviceFlow::Input
                && (device.id == self.roles.default_input_id
                    || device.id == self.roles.comms_input_id)
            {
                device.muted = target_muted;
            }
        }
        target_muted
    }

    pub fn toggle_device_pin(&mut self, device_id: &str) {
        if let Some(device) = self.device_mut(device_id) {
            device.pinned = !device.pinned;
        }
        if self.auto_disable_unused_devices {
            self.sync_auto_disabled_flags();
        }
    }

    pub fn set_device_pinned(&mut self, device_id: &str, pinned: bool) {
        if let Some(device) = self.device_mut(device_id) {
            device.pinned = pinned;
        }
        if self.auto_disable_unused_devices {
            self.sync_auto_disabled_flags();
        }
    }

    pub fn is_device_in_use(&self, device: &AudioDevice) -> bool {
        if device.pinned || self.is_default(device) || self.is_comms(device) {
            return true;
        }
        if device.flow == DeviceFlow::Output {
            return self.devices.iter().any(|input| {
                input.flow == DeviceFlow::Input
                    && !input.disabled
                    && input.listen.as_ref().is_some_and(|listen| {
                        listen.enabled
                            && listen.target_device_id.as_deref() == Some(device.id.as_str())
                    })
            });
        }
        false
    }

    pub fn sync_auto_disabled_flags(&mut self) {
        if !self.auto_disable_unused_devices {
            return;
        }

        let active_listen_targets: Vec<String> = self
            .devices
            .iter()
            .filter(|device| {
                device.flow == DeviceFlow::Input
                    && (device.pinned || self.is_default(device) || self.is_comms(device))
            })
            .filter_map(|device| device.listen.as_ref())
            .filter(|listen| listen.enabled)
            .filter_map(|listen| listen.target_device_id.clone())
            .collect();

        let in_use_flags: Vec<bool> = self
            .devices
            .iter()
            .map(|device| {
                device.pinned
                    || self.is_default(device)
                    || self.is_comms(device)
                    || (device.flow == DeviceFlow::Output
                        && active_listen_targets.iter().any(|id| id == &device.id))
            })
            .collect();

        for (device, in_use) in self.devices.iter_mut().zip(in_use_flags) {
            device.disabled = !in_use;
        }
    }

    pub fn set_device_disabled(&mut self, device_id: &str, disabled: bool) -> bool {
        let Some(flow) = self.device(device_id).map(|d| d.flow) else {
            return false;
        };
        if disabled {
            let Some(replacement_id) = self
                .devices
                .iter()
                .find(|d| d.flow == flow && d.id != device_id && !d.disabled)
                .map(|d| d.id.clone())
            else {
                return false;
            };
            if let Some(device) = self.device_mut(device_id) {
                device.disabled = true;
            }
            match flow {
                DeviceFlow::Output => {
                    if self.roles.default_output_id == device_id {
                        self.roles.default_output_id = replacement_id.clone();
                    }
                    if self.roles.comms_output_id == device_id {
                        self.roles.comms_output_id = replacement_id;
                    }
                }
                DeviceFlow::Input => {
                    if self.roles.default_input_id == device_id {
                        self.roles.default_input_id = replacement_id.clone();
                    }
                    if self.roles.comms_input_id == device_id {
                        self.roles.comms_input_id = replacement_id;
                    }
                }
            }
        } else if let Some(device) = self.device_mut(device_id) {
            device.disabled = false;
        }
        true
    }

    pub fn set_auto_disable_unused_devices(&mut self, enabled: bool) {
        self.auto_disable_unused_devices = enabled;
        if enabled {
            self.sync_auto_disabled_flags();
        }
    }

    pub fn reenable_all_devices(&mut self) {
        for device in &mut self.devices {
            device.disabled = false;
        }
    }

    pub fn disable_all_enhancements(&mut self, device_id: &str) {
        if let Some(device) = self.device_mut(device_id) {
            device.enhancements.disable_all();
        }
    }

    pub fn set_enhancements_disabled(&mut self, device_id: &str, disabled: bool) {
        if let Some(device) = self.device_mut(device_id) {
            if disabled {
                disable_enhancements_preserving_undo(device);
            } else {
                reenable_enhancements_from_undo(device);
            }
        }
    }

    pub fn set_auto_disable_enhancements(&mut self, enabled: bool) {
        self.auto_disable_enhancements = enabled;
        if enabled {
            for device in &mut self.devices {
                disable_enhancements_preserving_undo(device);
            }
        }
    }

    pub fn reenable_all_enhancements(&mut self) {
        for device in &mut self.devices {
            if device.disabled {
                continue;
            }
            reenable_enhancements_from_undo(device);
        }
    }

    pub fn are_enhancements_disabled(&self, device: &AudioDevice) -> bool {
        !device.enhancements.any_enabled()
    }

    pub fn set_enhancement_flag(&mut self, device_id: &str, layer: EnhancementLayer, enabled: bool) {
        if let Some(device) = self.device_mut(device_id) {
            match layer {
                EnhancementLayer::SysFx => device.enhancements.sys_fx_enabled = enabled,
                EnhancementLayer::Advanced => device.enhancements.advanced_enabled = enabled,
                EnhancementLayer::Spatial => device.enhancements.spatial_sound_enabled = enabled,
            }
        }
    }

    pub fn set_listen_enabled(&mut self, device_id: &str, enabled: bool) {
        if let Some(device) = self.device_mut(device_id)
            && device.flow == DeviceFlow::Input
        {
            let listen = device.listen.get_or_insert_with(ListenConfig::default);
            listen.enabled = enabled;
        }
        if self.auto_disable_unused_devices {
            self.sync_auto_disabled_flags();
        }
    }

    pub fn set_listen_target(&mut self, device_id: &str, target_device_id: Option<String>) {
        if let Some(device) = self.device_mut(device_id)
            && device.flow == DeviceFlow::Input
        {
            let listen = device.listen.get_or_insert_with(ListenConfig::default);
            listen.target_device_id = target_device_id;
        }
        if self.auto_disable_unused_devices {
            self.sync_auto_disabled_flags();
        }
    }

    pub fn listen_target_label(&self, target_id: Option<&str>) -> String {
        match target_id {
            None => "None".to_string(),
            Some(LISTEN_TARGET_DEFAULT_OUTPUT) => "Default Playback Device".to_string(),
            Some(LISTEN_TARGET_COMMS_OUTPUT) => "Default Communications Device".to_string(),
            Some(id) => self
                .devices
                .iter()
                .find(|d| d.id == id)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| "None".to_string()),
        }
    }

    pub fn apply_profile(&mut self, profile_id: &str) -> bool {
        let Some(profile) = self.profiles.iter().find(|p| p.id == profile_id).cloned() else {
            return false;
        };

        self.roles = profile.roles;
        if !self.separate_comms_role {
            self.unify_comms_roles();
        }
        for role_id in [
            self.roles.default_output_id.clone(),
            self.roles.comms_output_id.clone(),
            self.roles.default_input_id.clone(),
            self.roles.comms_input_id.clone(),
        ] {
            if !role_id.is_empty()
                && let Some(device) = self.device_mut(&role_id)
            {
                device.disabled = false;
            }
        }
        for snapshot in profile.device_settings {
            if let Some(device) = self.device_mut(&snapshot.device_id) {
                device.volume = snapshot.volume.min(100);
                device.muted = snapshot.muted;
                let saved_spatial = device.enhancements.previous_spatial_format.clone();
                device.enhancements = snapshot.enhancements;
                if device.enhancements.previous_spatial_format.is_none() {
                    device.enhancements.previous_spatial_format = saved_spatial;
                }
                if device.flow == DeviceFlow::Input {
                    device.listen = Some(snapshot.listen.unwrap_or_default());
                } else {
                    device.listen = None;
                }
            }
        }
        self.active_profile_id = Some(profile.id);
        if self.auto_disable_unused_devices {
            self.sync_auto_disabled_flags();
        }
        true
    }

    pub fn save_current_as_profile(&mut self, name: &str) -> String {
        let trimmed = name.trim();
        let display_name = if trimmed.is_empty() {
            format!("Profile {}", self.profiles.len() + 1)
        } else {
            trimmed.to_string()
        };

        let snapshots = self.capture_snapshots();

        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case(&display_name))
        {
            existing.name = display_name;
            existing.roles = self.roles.clone();
            existing.device_settings = snapshots;
            let id = existing.id.clone();
            self.active_profile_id = Some(id.clone());
            return id;
        }

        let id = self.unique_profile_id(&display_name);
        self.profiles.push(AudioProfile {
            id: id.clone(),
            name: display_name,
            roles: self.roles.clone(),
            device_settings: snapshots,
        });
        self.active_profile_id = Some(id.clone());
        id
    }

    pub fn delete_profile(&mut self, profile_id: &str) -> bool {
        let before = self.profiles.len();
        self.profiles.retain(|p| p.id != profile_id);
        let removed = self.profiles.len() < before;
        if removed && self.active_profile_id.as_deref() == Some(profile_id) {
            self.active_profile_id = None;
        }
        removed
    }

    pub fn devices_for(&self, flow: DeviceFlow) -> impl Iterator<Item = &AudioDevice> {
        self.devices.iter().filter(move |d| d.flow == flow)
    }

    pub fn filtered_devices(&self, flow: DeviceFlow, query: &str) -> Vec<&AudioDevice> {
        let needle = query.trim().to_ascii_lowercase();
        let mut devices: Vec<&AudioDevice> = self
            .devices_for(flow)
            .filter(|device| {
                needle.is_empty()
                    || device.name.to_ascii_lowercase().contains(&needle)
                    || device.subtitle.to_ascii_lowercase().contains(&needle)
            })
            .collect();
        devices.sort_by_key(|device| !device.pinned);
        devices
    }

    pub fn is_default(&self, device: &AudioDevice) -> bool {
        match device.flow {
            DeviceFlow::Output => self.roles.default_output_id == device.id,
            DeviceFlow::Input => self.roles.default_input_id == device.id,
        }
    }

    pub fn is_comms(&self, device: &AudioDevice) -> bool {
        match device.flow {
            DeviceFlow::Output => self.roles.comms_output_id == device.id,
            DeviceFlow::Input => self.roles.comms_input_id == device.id,
        }
    }

    pub fn active_profile(&self) -> Option<&AudioProfile> {
        let id = self.active_profile_id.as_deref()?;
        self.profiles.iter().find(|p| p.id == id)
    }

    fn unify_comms_roles(&mut self) {
        self.roles.comms_output_id = self.roles.default_output_id.clone();
        self.roles.comms_input_id = self.roles.default_input_id.clone();
    }

    fn reconcile_role(
        &self,
        flow: DeviceFlow,
        stored_id: &str,
        live_id: &str,
        has_real_devices: bool,
    ) -> String {
        if self.lock_enabled && !stored_id.is_empty() {
            let is_legacy_mock = !stored_id.starts_with('{') && has_real_devices;
            if !is_legacy_mock {
                match self
                    .devices
                    .iter()
                    .find(|d| d.id == stored_id && d.flow == flow)
                {
                    Some(device) if !device.disabled => return stored_id.to_string(),
                    Some(_) => {}
                    None if stored_id.starts_with('{') => return stored_id.to_string(),
                    None => {}
                }
            }
        }
        if !live_id.is_empty() {
            return live_id.to_string();
        }
        self.devices_for(flow)
            .find(|d| !d.disabled)
            .or_else(|| self.devices_for(flow).next())
            .map(|d| d.id.clone())
            .unwrap_or_default()
    }

    fn device_mut(&mut self, device_id: &str) -> Option<&mut AudioDevice> {
        self.devices.iter_mut().find(|d| d.id == device_id)
    }

    fn capture_snapshots(&self) -> Vec<DeviceSnapshot> {
        self.devices
            .iter()
            .map(|d| DeviceSnapshot {
                device_id: d.id.clone(),
                volume: d.volume,
                muted: d.muted,
                enhancements: d.enhancements.clone(),
                listen: d.listen.clone(),
            })
            .collect()
    }

    fn unique_profile_id(&self, name: &str) -> String {
        let mut base: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        while base.contains("--") {
            base = base.replace("--", "-");
        }
        let base = base.trim_matches('-');
        let base = if base.is_empty() { "profile" } else { base };

        if !self.profiles.iter().any(|p| p.id == base) {
            return base.to_string();
        }
        let mut suffix = 2usize;
        loop {
            let candidate = format!("{base}-{suffix}");
            if !self.profiles.iter().any(|p| p.id == candidate) {
                return candidate;
            }
            suffix += 1;
        }
    }
}

fn disable_enhancements_preserving_undo(device: &mut AudioDevice) {
    if device.enhancements.any_enabled() {
        device.previous_enhancements = Some(device.enhancements.clone());
        device.enhancements.disable_all();
    }
}

fn reenable_enhancements_from_undo(device: &mut AudioDevice) {
    if !device.enhancements.any_enabled() {
        let had_spatial_format = device.enhancements.previous_spatial_format.is_some();
        let mut restored = device
            .previous_enhancements
            .take()
            .filter(|prev| prev.any_enabled())
            .unwrap_or(EnhancementsState {
                sys_fx_enabled: true,
                advanced_enabled: true,
                spatial_sound_enabled: false,
                previous_spatial_format: None,
            });
        if had_spatial_format {
            restored.spatial_sound_enabled = true;
        }
        restored.previous_spatial_format = None;
        device.enhancements = restored;
    }
}

fn is_mock_device_id(id: &str) -> bool {
    !id.starts_with('{') && (id.starts_with("out-") || id.starts_with("in-"))
}

fn is_mock_only_profile(profile: &AudioProfile) -> bool {
    let role_ids = [
        profile.roles.default_output_id.as_str(),
        profile.roles.comms_output_id.as_str(),
        profile.roles.default_input_id.as_str(),
        profile.roles.comms_input_id.as_str(),
    ];
    let all_ids: Vec<&str> = role_ids
        .into_iter()
        .chain(profile.device_settings.iter().map(|s| s.device_id.as_str()))
        .filter(|id| !id.is_empty())
        .collect();
    !all_ids.is_empty() && all_ids.into_iter().all(is_mock_device_id)
}
