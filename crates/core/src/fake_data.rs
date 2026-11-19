use crate::model::{
    AppState, AudioDevice, AudioProfile, DeviceFlow, DeviceSnapshot, EnhancementsState, ListenConfig,
    RightClickAction, RoleAssignments, ThemeMode,
};

pub fn default_mock_state() -> AppState {
    let devices = vec![
        AudioDevice {
            id: "out-hd660s2".into(),
            name: "Headphones".into(),
            subtitle: "Sennheiser HD 660S2 · DAC Amp".into(),
            flow: DeviceFlow::Output,
            volume: 74,
            muted: false,
            pinned: true,
            disabled: false,
            enhancements: EnhancementsState {
                sys_fx_enabled: true,
                advanced_enabled: true,
                spatial_sound_enabled: true,
            },
            previous_enhancements: None,
            listen: None,
        },
        AudioDevice {
            id: "out-arctis-chat".into(),
            name: "Headset Earphone".into(),
            subtitle: "Arctis Nova Pro Wireless".into(),
            flow: DeviceFlow::Output,
            volume: 82,
            muted: false,
            pinned: false,
            disabled: false,
            enhancements: EnhancementsState {
                sys_fx_enabled: true,
                advanced_enabled: false,
                spatial_sound_enabled: false,
            },
            previous_enhancements: None,
            listen: None,
        },
        AudioDevice {
            id: "out-scarlett".into(),
            name: "Speakers".into(),
            subtitle: "Focusrite Scarlett 2i2".into(),
            flow: DeviceFlow::Output,
            volume: 55,
            muted: false,
            pinned: false,
            disabled: false,
            enhancements: EnhancementsState::all_disabled(),
            previous_enhancements: None,
            listen: None,
        },
        AudioDevice {
            id: "out-lg-hdmi".into(),
            name: "LG UltraGear 27\"".into(),
            subtitle: "NVIDIA High Definition Audio".into(),
            flow: DeviceFlow::Output,
            volume: 40,
            muted: true,
            pinned: false,
            disabled: false,
            enhancements: EnhancementsState {
                sys_fx_enabled: false,
                advanced_enabled: true,
                spatial_sound_enabled: true,
            },
            previous_enhancements: None,
            listen: None,
        },
        AudioDevice {
            id: "in-mv7".into(),
            name: "Shure MV7 Microphone".into(),
            subtitle: "USB Condenser".into(),
            flow: DeviceFlow::Input,
            volume: 86,
            muted: false,
            pinned: true,
            disabled: false,
            enhancements: EnhancementsState {
                sys_fx_enabled: true,
                advanced_enabled: true,
                spatial_sound_enabled: false,
            },
            previous_enhancements: None,
            listen: Some(ListenConfig::default()),
        },
        AudioDevice {
            id: "in-arctis-mic".into(),
            name: "Headset Microphone".into(),
            subtitle: "Arctis Nova Pro Wireless".into(),
            flow: DeviceFlow::Input,
            volume: 78,
            muted: false,
            pinned: false,
            disabled: false,
            enhancements: EnhancementsState {
                sys_fx_enabled: true,
                advanced_enabled: false,
                spatial_sound_enabled: false,
            },
            previous_enhancements: None,
            listen: Some(ListenConfig::default()),
        },
        AudioDevice {
            id: "in-scarlett-line".into(),
            name: "Line In".into(),
            subtitle: "Focusrite Scarlett 2i2".into(),
            flow: DeviceFlow::Input,
            volume: 65,
            muted: true,
            pinned: false,
            disabled: false,
            enhancements: EnhancementsState::all_disabled(),
            previous_enhancements: None,
            listen: Some(ListenConfig::default()),
        },
    ];

    let gaming_roles = RoleAssignments {
        default_output_id: "out-hd660s2".into(),
        comms_output_id: "out-hd660s2".into(),
        default_input_id: "in-mv7".into(),
        comms_input_id: "in-mv7".into(),
    };

    let desk_roles = RoleAssignments {
        default_output_id: "out-scarlett".into(),
        comms_output_id: "out-scarlett".into(),
        default_input_id: "in-mv7".into(),
        comms_input_id: "in-mv7".into(),
    };

    let work_roles = RoleAssignments {
        default_output_id: "out-arctis-chat".into(),
        comms_output_id: "out-arctis-chat".into(),
        default_input_id: "in-arctis-mic".into(),
        comms_input_id: "in-arctis-mic".into(),
    };

    let gaming_snapshots = snapshot_from_devices(&devices);

    let desk_snapshots = vec![
        DeviceSnapshot {
            device_id: "out-hd660s2".into(),
            volume: 60,
            muted: true,
            enhancements: EnhancementsState::all_disabled(),
            listen: None,
        },
        DeviceSnapshot {
            device_id: "out-arctis-chat".into(),
            volume: 70,
            muted: false,
            enhancements: EnhancementsState::all_disabled(),
            listen: None,
        },
        DeviceSnapshot {
            device_id: "out-scarlett".into(),
            volume: 64,
            muted: false,
            enhancements: EnhancementsState::all_disabled(),
            listen: None,
        },
        DeviceSnapshot {
            device_id: "out-lg-hdmi".into(),
            volume: 35,
            muted: true,
            enhancements: EnhancementsState::all_disabled(),
            listen: None,
        },
        DeviceSnapshot {
            device_id: "in-mv7".into(),
            volume: 88,
            muted: false,
            enhancements: EnhancementsState::all_disabled(),
            listen: Some(ListenConfig::default()),
        },
        DeviceSnapshot {
            device_id: "in-arctis-mic".into(),
            volume: 75,
            muted: true,
            enhancements: EnhancementsState::all_disabled(),
            listen: Some(ListenConfig::default()),
        },
        DeviceSnapshot {
            device_id: "in-scarlett-line".into(),
            volume: 65,
            muted: true,
            enhancements: EnhancementsState::all_disabled(),
            listen: Some(ListenConfig::default()),
        },
    ];

    let work_snapshots = vec![
        DeviceSnapshot {
            device_id: "out-hd660s2".into(),
            volume: 68,
            muted: false,
            enhancements: EnhancementsState::all_disabled(),
            listen: None,
        },
        DeviceSnapshot {
            device_id: "out-arctis-chat".into(),
            volume: 85,
            muted: false,
            enhancements: EnhancementsState {
                sys_fx_enabled: true,
                advanced_enabled: false,
                spatial_sound_enabled: false,
            },
            listen: None,
        },
        DeviceSnapshot {
            device_id: "out-scarlett".into(),
            volume: 45,
            muted: true,
            enhancements: EnhancementsState::all_disabled(),
            listen: None,
        },
        DeviceSnapshot {
            device_id: "out-lg-hdmi".into(),
            volume: 40,
            muted: true,
            enhancements: EnhancementsState::all_disabled(),
            listen: None,
        },
        DeviceSnapshot {
            device_id: "in-mv7".into(),
            volume: 80,
            muted: true,
            enhancements: EnhancementsState::all_disabled(),
            listen: Some(ListenConfig::default()),
        },
        DeviceSnapshot {
            device_id: "in-arctis-mic".into(),
            volume: 84,
            muted: false,
            enhancements: EnhancementsState {
                sys_fx_enabled: true,
                advanced_enabled: false,
                spatial_sound_enabled: false,
            },
            listen: Some(ListenConfig::default()),
        },
        DeviceSnapshot {
            device_id: "in-scarlett-line".into(),
            volume: 60,
            muted: true,
            enhancements: EnhancementsState::all_disabled(),
            listen: Some(ListenConfig::default()),
        },
    ];

    AppState {
        lock_enabled: true,
        theme: ThemeMode::Dark,
        separate_comms_role: false,
        right_click_action: RightClickAction::Menu,
        run_on_startup: false,
        auto_update: true,
        staged_update_version: None,
        auto_disable_enhancements: false,
        auto_disable_unused_devices: false,
        active_profile_id: Some("gaming".into()),
        roles: gaming_roles.clone(),
        devices,
        profiles: vec![
            AudioProfile {
                id: "gaming".into(),
                name: "Gaming".into(),
                roles: gaming_roles,
                device_settings: gaming_snapshots,
            },
            AudioProfile {
                id: "desk-speakers".into(),
                name: "Desk Speakers".into(),
                roles: desk_roles,
                device_settings: desk_snapshots,
            },
            AudioProfile {
                id: "work-calls".into(),
                name: "Work & Calls".into(),
                roles: work_roles,
                device_settings: work_snapshots,
            },
        ],
    }
}

fn snapshot_from_devices(devices: &[AudioDevice]) -> Vec<DeviceSnapshot> {
    devices
        .iter()
        .map(|d| DeviceSnapshot {
            device_id: d.id.clone(),
            volume: d.volume,
            muted: d.muted,
            enhancements: d.enhancements,
            listen: d.listen.clone(),
        })
        .collect()
}
