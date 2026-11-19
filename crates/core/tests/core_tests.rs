use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use audio_lock_core::fake_data::default_mock_state;
use audio_lock_core::ipc::{AppAction, IpcRequest, IpcResponse, QUIT_EVENT_NAME};
use audio_lock_core::model::{
    AppState, AudioDevice, AudioProfile, DeviceFlow, EnhancementLayer, EnhancementsState,
    LISTEN_TARGET_COMMS_OUTPUT, LISTEN_TARGET_DEFAULT_OUTPUT, ListenConfig, RightClickAction,
    RoleAssignments, ThemeMode,
};
use audio_lock_core::store::{load_state_from, save_state_to};
use audio_lock_core::update::{
    CURRENT_VERSION, IDLE_RESTART_MS, UpdateManifest, is_newer_version, is_valid_version_dir_name,
    parse_semver, staging_root_dir,
};

#[test]
fn profile_hot_swap_updates_roles_and_device_settings() {
    let mut state = default_mock_state();
    assert_eq!(state.active_profile_id.as_deref(), Some("gaming"));
    assert_eq!(state.roles.default_output_id, "out-hd660s2");
    assert_eq!(state.roles.comms_output_id, "out-hd660s2");
    assert_eq!(state.roles.default_input_id, "in-mv7");
    assert_eq!(state.roles.comms_input_id, "in-mv7");

    assert!(state.apply_profile("desk-speakers"));
    assert_eq!(state.active_profile_id.as_deref(), Some("desk-speakers"));
    assert_eq!(state.roles.default_output_id, "out-scarlett");
    assert_eq!(state.roles.comms_output_id, "out-scarlett");
    assert_eq!(state.active_profile().map(|p| p.name.as_str()), Some("Desk Speakers"));

    let hd660 = state.devices.iter().find(|d| d.id == "out-hd660s2").unwrap();
    assert_eq!(hd660.volume, 60);
    assert!(hd660.muted);
    assert!(!hd660.enhancements.any_enabled());

    assert!(!state.apply_profile("does-not-exist"));
}

#[test]
fn save_and_delete_profiles_respect_case_insensitive_names() {
    let mut state = default_mock_state();
    state.set_default(DeviceFlow::Output, "out-lg-hdmi");
    state.set_volume("out-lg-hdmi", 91);

    let updated_id = state.save_current_as_profile("gaming");
    assert_eq!(updated_id, "gaming");
    assert_eq!(state.profiles.len(), 3);
    assert_eq!(state.active_profile_id.as_deref(), Some("gaming"));

    let new_id = state.save_current_as_profile("Late Night");
    assert_eq!(state.profiles.len(), 4);
    assert_eq!(state.active_profile_id.as_deref(), Some(new_id.as_str()));

    assert!(state.delete_profile(&new_id));
    assert_eq!(state.active_profile_id, None);
    assert_eq!(state.profiles.len(), 3);
    assert!(!state.delete_profile(&new_id));
}

#[test]
fn separate_default_and_comms_roles_by_flow() {
    let mut state = default_mock_state();
    assert!(!state.separate_comms_role);

    // Unified mode (`separate_comms_role = false`): `set_default` updates both default and comms.
    state.set_default(DeviceFlow::Output, "out-scarlett");
    state.set_default(DeviceFlow::Input, "in-arctis-mic");
    assert_eq!(state.roles.default_output_id, "out-scarlett");
    assert_eq!(state.roles.comms_output_id, "out-scarlett");
    assert_eq!(state.roles.default_input_id, "in-arctis-mic");
    assert_eq!(state.roles.comms_input_id, "in-arctis-mic");

    // Separate mode (`separate_comms_role = true`): `set_default` leaves comms independent.
    state.separate_comms_role = true;
    state.set_default(DeviceFlow::Output, "out-hd660s2");
    assert_eq!(state.roles.default_output_id, "out-hd660s2");
    assert_eq!(state.roles.comms_output_id, "out-scarlett");

    state.set_default(DeviceFlow::Output, "out-scarlett");
    state.set_comms(DeviceFlow::Output, "out-arctis-chat");
    state.set_default(DeviceFlow::Input, "in-mv7");
    state.set_comms(DeviceFlow::Input, "in-arctis-mic");

    // Cross-flow or unknown assignments are ignored.
    state.set_default(DeviceFlow::Output, "in-mv7");
    state.set_comms(DeviceFlow::Input, "out-hd660s2");
    state.set_default(DeviceFlow::Input, "unknown-id");

    let scarlett = state.devices.iter().find(|d| d.id == "out-scarlett").unwrap();
    let arctis_chat = state.devices.iter().find(|d| d.id == "out-arctis-chat").unwrap();
    let mv7 = state.devices.iter().find(|d| d.id == "in-mv7").unwrap();
    let arctis_mic = state.devices.iter().find(|d| d.id == "in-arctis-mic").unwrap();

    assert!(state.is_default(scarlett));
    assert!(!state.is_comms(scarlett));
    assert!(!state.is_default(arctis_chat));
    assert!(state.is_comms(arctis_chat));

    assert!(state.is_default(mv7));
    assert!(!state.is_comms(mv7));
    assert!(!state.is_default(arctis_mic));
    assert!(state.is_comms(arctis_mic));

    assert_eq!(state.devices_for(DeviceFlow::Output).count(), 4);
    assert_eq!(state.devices_for(DeviceFlow::Input).count(), 3);
}

#[test]
fn enhancements_can_be_toggled_and_disabled_in_one_click() {
    let mut state = default_mock_state();
    let hd660 = state.devices.iter().find(|d| d.id == "out-hd660s2").unwrap();
    assert!(hd660.enhancements.any_enabled());

    state.disable_all_enhancements("out-hd660s2");
    let hd660 = state.devices.iter().find(|d| d.id == "out-hd660s2").unwrap();
    assert!(!hd660.enhancements.any_enabled());

    state.set_enhancement_flag("out-hd660s2", EnhancementLayer::Spatial, true);
    let hd660 = state.devices.iter().find(|d| d.id == "out-hd660s2").unwrap();
    assert!(hd660.enhancements.any_enabled());
    assert!(hd660.enhancements.spatial_sound_enabled);
    assert!(!hd660.enhancements.sys_fx_enabled);
    assert!(!hd660.enhancements.advanced_enabled);
}

#[test]
fn listen_config_requires_target_device_to_actively_route() {
    let default_listen = ListenConfig::default();
    assert!(!default_listen.enabled);
    assert_eq!(default_listen.target_device_id, None);
    assert!(!default_listen.is_actively_routing());

    let mut state = default_mock_state();

    // Enabling listen alone must NOT activate routing until a target is chosen.
    state.set_listen_enabled("in-mv7", true);
    let mv7 = state.devices.iter().find(|d| d.id == "in-mv7").unwrap();
    let listen = mv7.listen.as_ref().unwrap();
    assert!(listen.enabled);
    assert_eq!(listen.target_device_id, None);
    assert!(!listen.is_actively_routing());

    // Setting a target completes active routing.
    state.set_listen_target("in-mv7", Some("out-hd660s2".into()));
    let mv7 = state.devices.iter().find(|d| d.id == "in-mv7").unwrap();
    assert!(mv7.listen.as_ref().unwrap().is_actively_routing());

    // Output devices never gain a ListenConfig.
    state.set_listen_enabled("out-hd660s2", true);
    let out = state.devices.iter().find(|d| d.id == "out-hd660s2").unwrap();
    assert!(out.listen.is_none());
}

#[test]
fn json_round_trip_persistence_and_ipc_messages() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("audio-lock-core-test-{unique}"));
    let path = dir.join("state.json");

    // Missing file falls back to empty default AppState.
    let fallback = load_state_from(&path);
    assert_eq!(fallback, AppState::default());
    assert!(fallback.lock_enabled);
    assert!(fallback.auto_update);
    assert_eq!(fallback.theme, ThemeMode::Dark);
    assert!(fallback.devices.is_empty());
    assert!(fallback.profiles.is_empty());

    // Save, overwrite, and reload.
    let mut state = default_mock_state();
    state.set_volume("out-hd660s2", 120);
    state.toggle_mute("out-hd660s2");
    save_state_to(&path, &state).unwrap();

    state.set_volume("out-hd660s2", 42);
    save_state_to(&path, &state).unwrap();

    let loaded = load_state_from(&path);
    assert_eq!(loaded, state);
    assert_eq!(
        loaded.devices.iter().find(|d| d.id == "out-hd660s2").unwrap().volume,
        42
    );

    // Invalid JSON falls back cleanly to empty default AppState.
    fs::write(&path, b"{not valid json").unwrap();
    assert_eq!(load_state_from(&path), AppState::default());

    assert_eq!(QUIT_EVENT_NAME, "Local\\DefaultAudioLockQuitEvent");
    for req in [IpcRequest::GetState, IpcRequest::Ping] {
        let req_json = serde_json::to_string(&req).unwrap();
        let req_back: IpcRequest = serde_json::from_str(&req_json).unwrap();
        assert_eq!(req_back, req);
    }

    let resp = IpcResponse::State(state);
    let resp_json = serde_json::to_string(&resp).unwrap();
    let resp_back: IpcResponse = serde_json::from_str(&resp_json).unwrap();
    assert_eq!(resp_back, resp);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn toggle_device_pin_and_filtered_devices_preserve_stable_pinned_order() {
    let mut state = default_mock_state();

    let output_ids: Vec<&str> = state
        .filtered_devices(DeviceFlow::Output, "")
        .into_iter()
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(
        output_ids,
        vec!["out-hd660s2", "out-arctis-chat", "out-scarlett", "out-lg-hdmi"]
    );

    // Pinning an unpinned device moves it into the pinned group while preserving relative order.
    state.toggle_device_pin("out-scarlett");
    let output_ids: Vec<&str> = state
        .filtered_devices(DeviceFlow::Output, "")
        .into_iter()
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(
        output_ids,
        vec!["out-hd660s2", "out-scarlett", "out-arctis-chat", "out-lg-hdmi"]
    );

    // Unpinning returns it to the unpinned group in its original slice order.
    state.toggle_device_pin("out-hd660s2");
    let output_ids: Vec<&str> = state
        .filtered_devices(DeviceFlow::Output, "")
        .into_iter()
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(
        output_ids,
        vec!["out-scarlett", "out-hd660s2", "out-arctis-chat", "out-lg-hdmi"]
    );

    // Case-insensitive filtering matches both device name and subtitle.
    let by_name: Vec<&str> = state
        .filtered_devices(DeviceFlow::Output, "  speakers ")
        .into_iter()
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(by_name, vec!["out-scarlett"]);

    let by_subtitle: Vec<&str> = state
        .filtered_devices(DeviceFlow::Output, "sennheiser")
        .into_iter()
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(by_subtitle, vec!["out-hd660s2"]);

    let input_by_subtitle: Vec<&str> = state
        .filtered_devices(DeviceFlow::Input, "FOCUSRITE")
        .into_iter()
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(input_by_subtitle, vec!["in-scarlett-line"]);
}

#[test]
fn set_enhancements_disabled_undoes_and_restores_previous_flags() {
    let mut state = default_mock_state();

    // `out-arctis-chat` starts with only sys_fx_enabled = true.
    state.set_enhancements_disabled("out-arctis-chat", true);
    let arctis = state.devices.iter().find(|d| d.id == "out-arctis-chat").unwrap();
    assert!(state.are_enhancements_disabled(arctis));
    assert!(arctis.previous_enhancements.is_some());

    // Unchecking restores the exact prior flags and clears previous_enhancements.
    state.set_enhancements_disabled("out-arctis-chat", false);
    let arctis = state.devices.iter().find(|d| d.id == "out-arctis-chat").unwrap();
    assert!(!state.are_enhancements_disabled(arctis));
    assert!(arctis.enhancements.sys_fx_enabled);
    assert!(!arctis.enhancements.advanced_enabled);
    assert!(!arctis.enhancements.spatial_sound_enabled);
    assert_eq!(arctis.previous_enhancements, None);

    // Device with all enhancements already off restores default visible enhancements when unchecked.
    let scarlett = state.devices.iter().find(|d| d.id == "out-scarlett").unwrap();
    assert!(state.are_enhancements_disabled(scarlett));
    state.set_enhancements_disabled("out-scarlett", false);
    let scarlett = state.devices.iter().find(|d| d.id == "out-scarlett").unwrap();
    assert!(scarlett.enhancements.sys_fx_enabled);
    assert!(scarlett.enhancements.advanced_enabled);
    assert!(!scarlett.enhancements.spatial_sound_enabled);
}

#[test]
fn ipc_flyout_and_quit_requests_round_trip_json() {
    let requests = [
        IpcRequest::OpenFlyout {
            anchor_x: Some(1280),
            anchor_y: Some(980),
        },
        IpcRequest::OpenFlyout {
            anchor_x: None,
            anchor_y: None,
        },
        IpcRequest::QuitDaemon,
    ];

    for req in requests {
        let json = serde_json::to_string(&req).unwrap();
        let back: IpcRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }
}

#[test]
fn toggle_active_mics_mute_handles_shared_and_split_roles() {
    // Shared default + comms mic
    let mut state = default_mock_state();
    assert!(!state.are_active_mics_muted());

    assert!(state.toggle_active_mics_mute());
    assert!(state.are_active_mics_muted());
    assert!(state.devices.iter().find(|d| d.id == "in-mv7").unwrap().muted);

    assert!(!state.toggle_active_mics_mute());
    assert!(!state.are_active_mics_muted());
    assert!(!state.devices.iter().find(|d| d.id == "in-mv7").unwrap().muted);

    // Split default + comms mics where one is unmuted and one is muted
    state.separate_comms_role = true;
    state.set_default(DeviceFlow::Input, "in-mv7");
    state.set_comms(DeviceFlow::Input, "in-scarlett-line");
    assert!(!state.devices.iter().find(|d| d.id == "in-mv7").unwrap().muted);
    assert!(state.devices.iter().find(|d| d.id == "in-scarlett-line").unwrap().muted);
    assert!(!state.are_active_mics_muted());

    // First toggle mutes both
    assert!(state.toggle_active_mics_mute());
    assert!(state.are_active_mics_muted());
    assert!(state.devices.iter().find(|d| d.id == "in-mv7").unwrap().muted);
    assert!(state.devices.iter().find(|d| d.id == "in-scarlett-line").unwrap().muted);

    // Second toggle unmutes both
    assert!(!state.toggle_active_mics_mute());
    assert!(!state.are_active_mics_muted());
    assert!(!state.devices.iter().find(|d| d.id == "in-mv7").unwrap().muted);
    assert!(!state.devices.iter().find(|d| d.id == "in-scarlett-line").unwrap().muted);
}

#[test]
fn legacy_state_json_without_new_settings_deserializes_with_defaults() {
    let state = default_mock_state();
    let mut value = serde_json::to_value(&state).unwrap();
    let obj = value.as_object_mut().unwrap();
    obj.remove("separate_comms_role");
    obj.remove("right_click_action");
    obj.remove("run_on_startup");
    obj.remove("auto_update");
    obj.remove("staged_update_version");
    obj.remove("auto_disable_enhancements");

    let legacy_json = serde_json::to_string(&value).unwrap();
    let loaded: AppState = serde_json::from_str(&legacy_json).unwrap();
    assert!(!loaded.separate_comms_role);
    assert_eq!(loaded.right_click_action, RightClickAction::Menu);
    assert!(!loaded.run_on_startup);
    assert!(loaded.auto_update);
    assert_eq!(loaded.staged_update_version, None);
    assert!(!loaded.auto_disable_enhancements);
    assert_eq!(loaded, state);
}

#[test]
fn set_auto_disable_enhancements_disables_all_devices_and_preserves_undo() {
    let mut state = default_mock_state();
    assert!(!state.auto_disable_enhancements);

    state.set_auto_disable_enhancements(true);
    assert!(state.auto_disable_enhancements);
    assert!(state.devices.iter().all(|d| !d.enhancements.any_enabled()));

    let hd660 = state.devices.iter().find(|d| d.id == "out-hd660s2").unwrap();
    assert_eq!(
        hd660.previous_enhancements,
        Some(EnhancementsState {
            sys_fx_enabled: true,
            advanced_enabled: true,
            spatial_sound_enabled: true,
            previous_spatial_format: None,
        })
    );

    let scarlett = state.devices.iter().find(|d| d.id == "out-scarlett").unwrap();
    assert_eq!(scarlett.previous_enhancements, None);

    state.set_enhancements_disabled("out-hd660s2", false);
    let hd660 = state.devices.iter().find(|d| d.id == "out-hd660s2").unwrap();
    assert_eq!(
        hd660.enhancements,
        EnhancementsState {
            sys_fx_enabled: true,
            advanced_enabled: true,
            spatial_sound_enabled: true,
            previous_spatial_format: None,
        }
    );
    assert_eq!(hd660.previous_enhancements, None);

    state.set_auto_disable_enhancements(false);
    assert!(!state.auto_disable_enhancements);
}

#[test]
fn listen_target_label_resolves_sentinels_and_devices() {
    let state = default_mock_state();

    assert_eq!(state.listen_target_label(None), "None");
    assert_eq!(
        state.listen_target_label(Some(LISTEN_TARGET_DEFAULT_OUTPUT)),
        "Default Playback Device"
    );
    assert_eq!(
        state.listen_target_label(Some(LISTEN_TARGET_COMMS_OUTPUT)),
        "Default Communications Device"
    );
    assert_eq!(state.listen_target_label(Some("out-hd660s2")), "Headphones");
    assert_eq!(state.listen_target_label(Some("unknown-device")), "None");

    let listen = ListenConfig {
        enabled: true,
        target_device_id: Some(LISTEN_TARGET_DEFAULT_OUTPUT.to_string()),
    };
    assert!(listen.is_actively_routing());
}

#[test]
fn reenable_all_enhancements_restores_prior_flags_across_all_devices() {
    let mut state = default_mock_state();
    let expected: Vec<(String, EnhancementsState)> = state
        .devices
        .iter()
        .map(|d| {
            let target = if d.enhancements.any_enabled() {
                d.enhancements.clone()
            } else {
                EnhancementsState {
                    sys_fx_enabled: true,
                    advanced_enabled: true,
                    spatial_sound_enabled: false,
                    previous_spatial_format: None,
                }
            };
            (d.id.clone(), target)
        })
        .collect();

    state.set_auto_disable_enhancements(true);
    assert!(state.devices.iter().all(|d| !d.enhancements.any_enabled()));

    state.set_auto_disable_enhancements(false);
    state.reenable_all_enhancements();

    assert!(!state.auto_disable_enhancements);
    for (id, expected_enhancements) in &expected {
        let device = state.devices.iter().find(|d| &d.id == id).unwrap();
        assert_eq!(device.enhancements, *expected_enhancements, "device {id}");
        assert_eq!(device.previous_enhancements, None, "device {id}");
    }

    // Calling again leaves devices with already-enabled enhancements untouched.
    state.set_enhancement_flag("out-hd660s2", EnhancementLayer::SysFx, false);
    state.reenable_all_enhancements();
    let hd660 = state.devices.iter().find(|d| d.id == "out-hd660s2").unwrap();
    assert_eq!(
        hd660.enhancements,
        EnhancementsState {
            sys_fx_enabled: false,
            advanced_enabled: true,
            spatial_sound_enabled: true,
            previous_spatial_format: None,
        }
    );
}

fn live_device(id: &str, name: &str, flow: DeviceFlow) -> AudioDevice {
    AudioDevice {
        id: id.to_string(),
        name: name.to_string(),
        subtitle: "Live Endpoint".to_string(),
        flow,
        volume: 50,
        muted: false,
        pinned: false,
        disabled: false,
        enhancements: EnhancementsState {
            sys_fx_enabled: true,
            advanced_enabled: false,
            spatial_sound_enabled: false,
            previous_spatial_format: None,
        },
        previous_enhancements: None,
        listen: (flow == DeviceFlow::Input).then(ListenConfig::default),
    }
}

#[test]
fn reconcile_live_devices_preserves_metadata_and_migrates_mock_profiles() {
    let mut state = default_mock_state();
    assert!(!state.profiles.is_empty());
    assert_eq!(state.active_profile_id.as_deref(), Some("gaming"));

    let real_out_1 = "{0.0.0.00000000}.{11111111-1111-1111-1111-111111111111}";
    let real_out_2 = "{0.0.0.00000000}.{22222222-2222-2222-2222-222222222222}";
    let real_in_1 = "{0.0.1.00000000}.{33333333-3333-3333-3333-333333333333}";
    let real_in_2 = "{0.0.1.00000000}.{44444444-4444-4444-4444-444444444444}";

    // First transition from mock state to live Windows endpoints clears mock-only profiles.
    state.reconcile_live_devices(
        vec![
            live_device(real_out_1, "Real Headphones", DeviceFlow::Output),
            live_device(real_out_2, "Real Speakers", DeviceFlow::Output),
            live_device(real_in_1, "Real Mic", DeviceFlow::Input),
            live_device(real_in_2, "Real Line In", DeviceFlow::Input),
        ],
        RoleAssignments {
            default_output_id: real_out_1.into(),
            comms_output_id: real_out_2.into(),
            default_input_id: real_in_1.into(),
            comms_input_id: String::new(),
        },
    );

    assert!(state.profiles.is_empty());
    assert_eq!(state.active_profile_id, None);
    assert_eq!(state.roles.default_output_id, real_out_1);
    assert_eq!(state.roles.comms_output_id, real_out_2);
    assert_eq!(state.roles.default_input_id, real_in_1);
    assert_eq!(state.roles.comms_input_id, real_in_1);

    // Configure user metadata + real profile before next reconcile.
    let undo_enh = EnhancementsState {
        sys_fx_enabled: true,
        advanced_enabled: true,
        spatial_sound_enabled: true,
        previous_spatial_format: None,
    };
    state.toggle_device_pin(real_out_2);
    if let Some(d) = state.devices.iter_mut().find(|d| d.id == real_out_2) {
        d.previous_enhancements = Some(undo_enh.clone());
    }
    state.set_listen_enabled(real_in_1, true);
    state.set_listen_target(real_in_1, Some(LISTEN_TARGET_COMMS_OUTPUT.into()));
    state.set_listen_enabled(real_in_2, true);
    state.set_listen_target(real_in_2, None);

    let saved_profile = state.save_current_as_profile("Studio");

    // On subsequent reconcile with `lock_enabled = true`, locked real roles are preserved,
    // pinned/previous_enhancements/listen targets are preserved, and real profiles remain intact.
    let mut updated_in_1 = live_device(real_in_1, "Real Mic", DeviceFlow::Input);
    updated_in_1.listen = Some(ListenConfig {
        enabled: true,
        target_device_id: Some(real_out_2.into()),
    });
    let mut updated_in_2 = live_device(real_in_2, "Real Line In", DeviceFlow::Input);
    updated_in_2.listen = Some(ListenConfig {
        enabled: false,
        target_device_id: Some(real_out_1.into()),
    });

    state.reconcile_live_devices(
        vec![
            live_device(real_out_1, "Real Headphones", DeviceFlow::Output),
            live_device(real_out_2, "Real Speakers", DeviceFlow::Output),
            updated_in_1,
            updated_in_2,
        ],
        RoleAssignments {
            default_output_id: real_out_2.into(),
            comms_output_id: real_out_1.into(),
            default_input_id: real_in_2.into(),
            comms_input_id: real_in_2.into(),
        },
    );

    assert_eq!(state.profiles.len(), 1);
    assert_eq!(state.active_profile_id.as_deref(), Some(saved_profile.as_str()));
    assert_eq!(state.roles.default_output_id, real_out_1);
    assert_eq!(state.roles.comms_output_id, real_out_2);
    assert_eq!(state.roles.default_input_id, real_in_1);
    assert_eq!(state.roles.comms_input_id, real_in_1);

    let out2 = state.device(real_out_2).unwrap();
    assert!(out2.pinned);
    assert_eq!(out2.previous_enhancements, Some(undo_enh));

    let in1 = state.device(real_in_1).unwrap();
    assert_eq!(
        in1.listen,
        Some(ListenConfig {
            enabled: true,
            target_device_id: Some(LISTEN_TARGET_COMMS_OUTPUT.into()),
        })
    );

    let in2 = state.device(real_in_2).unwrap();
    assert_eq!(
        in2.listen,
        Some(ListenConfig {
            enabled: false,
            target_device_id: None,
        })
    );

    // When `lock_enabled = false`, reconcile adopts `live_roles`.
    state.lock_enabled = false;
    state.reconcile_live_devices(
        vec![
            live_device(real_out_1, "Real Headphones", DeviceFlow::Output),
            live_device(real_out_2, "Real Speakers", DeviceFlow::Output),
            live_device(real_in_1, "Real Mic", DeviceFlow::Input),
            live_device(real_in_2, "Real Line In", DeviceFlow::Input),
        ],
        RoleAssignments {
            default_output_id: real_out_2.into(),
            comms_output_id: real_out_1.into(),
            default_input_id: real_in_2.into(),
            comms_input_id: real_in_2.into(),
        },
    );

    assert_eq!(state.roles.default_output_id, real_out_2);
    assert_eq!(state.roles.comms_output_id, real_out_1);
    assert_eq!(state.roles.default_input_id, real_in_2);
    assert_eq!(state.roles.comms_input_id, real_in_2);
}

#[test]
fn apply_action_updates_all_state_dimensions_consistently() {
    let mut state = default_mock_state();

    state.apply_action(&AppAction::SetLockEnabled(false));
    assert!(!state.lock_enabled);

    state.apply_action(&AppAction::SetTheme(ThemeMode::Light));
    assert_eq!(state.theme, ThemeMode::Light);

    state.apply_action(&AppAction::SetSeparateCommsRole(true));
    assert!(state.separate_comms_role);

    state.apply_action(&AppAction::SetRightClickAction(RightClickAction::Flyout));
    assert_eq!(state.right_click_action, RightClickAction::Flyout);

    state.apply_action(&AppAction::SetRunOnStartup(true));
    assert!(state.run_on_startup);

    assert!(state.auto_update);
    state.apply_action(&AppAction::SetAutoUpdate(false));
    assert!(!state.auto_update);
    state.apply_action(&AppAction::SetAutoUpdate(true));
    assert!(state.auto_update);

    state.staged_update_version = Some("0.2.0".into());
    let before_noop = state.clone();
    state.apply_action(&AppAction::CheckForUpdatesNow);
    state.apply_action(&AppAction::ApplyStagedUpdateNow);
    assert_eq!(state, before_noop);
    state.staged_update_version = None;

    state.apply_action(&AppAction::SetDefault {
        flow: DeviceFlow::Output,
        device_id: "out-scarlett".into(),
    });
    state.apply_action(&AppAction::SetComms {
        flow: DeviceFlow::Output,
        device_id: "out-arctis-chat".into(),
    });
    assert_eq!(state.roles.default_output_id, "out-scarlett");
    assert_eq!(state.roles.comms_output_id, "out-arctis-chat");

    state.apply_action(&AppAction::SetVolume {
        device_id: "out-scarlett".into(),
        volume: 135,
    });
    state.apply_action(&AppAction::ToggleMute {
        device_id: "out-scarlett".into(),
    });
    let scarlett = state.device("out-scarlett").unwrap();
    assert_eq!(scarlett.volume, 100);
    assert!(scarlett.muted);

    state.apply_action(&AppAction::SetMute {
        device_id: "out-scarlett".into(),
        muted: false,
    });
    assert!(!state.device("out-scarlett").unwrap().muted);
    state.apply_action(&AppAction::SetMute {
        device_id: "out-scarlett".into(),
        muted: false,
    });
    assert!(!state.device("out-scarlett").unwrap().muted);

    state.apply_action(&AppAction::ToggleDevicePin {
        device_id: "out-scarlett".into(),
    });
    assert!(state.device("out-scarlett").unwrap().pinned);

    state.apply_action(&AppAction::SetDevicePinned {
        device_id: "out-scarlett".into(),
        pinned: false,
    });
    assert!(!state.device("out-scarlett").unwrap().pinned);

    state.apply_action(&AppAction::SetEnhancementsDisabled {
        device_id: "out-hd660s2".into(),
        disabled: true,
    });
    assert!(!state.device("out-hd660s2").unwrap().enhancements.any_enabled());

    state.apply_action(&AppAction::SetAutoDisableEnhancements(true));
    assert!(state.auto_disable_enhancements);
    assert!(state.devices.iter().all(|d| !d.enhancements.any_enabled()));

    state.apply_action(&AppAction::ReenableAllEnhancements);
    assert!(state.device("out-hd660s2").unwrap().enhancements.any_enabled());

    state.apply_action(&AppAction::SetListenEnabled {
        device_id: "in-mv7".into(),
        enabled: true,
    });
    state.apply_action(&AppAction::SetListenTarget {
        device_id: "in-mv7".into(),
        target_device_id: Some(LISTEN_TARGET_DEFAULT_OUTPUT.into()),
    });
    assert_eq!(
        state.device("in-mv7").unwrap().listen,
        Some(ListenConfig {
            enabled: true,
            target_device_id: Some(LISTEN_TARGET_DEFAULT_OUTPUT.into()),
        })
    );

    state.apply_action(&AppAction::SaveProfile {
        name: "Action Profile".into(),
    });
    assert_eq!(state.active_profile_id.as_deref(), Some("action-profile"));

    state.apply_action(&AppAction::ApplyProfile {
        profile_id: "desk-speakers".into(),
    });
    assert_eq!(state.active_profile_id.as_deref(), Some("desk-speakers"));

    state.apply_action(&AppAction::DeleteProfile {
        profile_id: "action-profile".into(),
    });
    assert!(!state.profiles.iter().any(|p| p.id == "action-profile"));

    assert!(state.has_device_with_flow(DeviceFlow::Output, "out-scarlett"));
    assert!(!state.has_device_with_flow(DeviceFlow::Input, "out-scarlett"));
    assert!(state.set_device_audio_state("out-scarlett", 42, false));
    assert!(!state.set_device_audio_state("out-scarlett", 42, false));
    assert!(state.set_device_enhancements(
        "out-scarlett",
        EnhancementsState {
            sys_fx_enabled: true,
            advanced_enabled: false,
            spatial_sound_enabled: false,
            previous_spatial_format: None,
        }
    ));
    assert!(!state.set_device_enhancements(
        "out-scarlett",
        EnhancementsState {
            sys_fx_enabled: true,
            advanced_enabled: false,
            spatial_sound_enabled: false,
            previous_spatial_format: None,
        }
    ));
    assert!(state.set_device_listen(
        "in-mv7",
        ListenConfig {
            enabled: true,
            target_device_id: Some("out-scarlett".into()),
        }
    ));
    assert!(!state.set_device_listen(
        "in-mv7",
        ListenConfig {
            enabled: true,
            target_device_id: Some("out-scarlett".into()),
        }
    ));
    assert!(!state.set_device_listen(
        "out-scarlett",
        ListenConfig {
            enabled: true,
            target_device_id: None,
        }
    ));
}

#[test]
fn ipc_apply_action_round_trips_json() {
    let requests = [
        IpcRequest::ApplyAction(AppAction::SetLockEnabled(false)),
        IpcRequest::ApplyAction(AppAction::SetTheme(ThemeMode::Light)),
        IpcRequest::ApplyAction(AppAction::SetSeparateCommsRole(true)),
        IpcRequest::ApplyAction(AppAction::SetRightClickAction(RightClickAction::Flyout)),
        IpcRequest::ApplyAction(AppAction::SetRunOnStartup(true)),
        IpcRequest::ApplyAction(AppAction::SetAutoUpdate(false)),
        IpcRequest::ApplyAction(AppAction::CheckForUpdatesNow),
        IpcRequest::ApplyAction(AppAction::ApplyStagedUpdateNow),
        IpcRequest::ApplyAction(AppAction::SetAutoDisableEnhancements(true)),
        IpcRequest::ApplyAction(AppAction::ReenableAllEnhancements),
        IpcRequest::ApplyAction(AppAction::SetAutoDisableUnusedDevices(true)),
        IpcRequest::ApplyAction(AppAction::SetDeviceDisabled {
            device_id: "out-scarlett".into(),
            disabled: true,
        }),
        IpcRequest::ApplyAction(AppAction::ReenableAllDevices),
        IpcRequest::ApplyAction(AppAction::SetDefault {
            flow: DeviceFlow::Output,
            device_id: "{0.0.0.00000000}.{abc}".into(),
        }),
        IpcRequest::ApplyAction(AppAction::SetComms {
            flow: DeviceFlow::Input,
            device_id: "{0.0.1.00000000}.{def}".into(),
        }),
        IpcRequest::ApplyAction(AppAction::SetVolume {
            device_id: "out-hd660s2".into(),
            volume: 75,
        }),
        IpcRequest::ApplyAction(AppAction::ToggleMute {
            device_id: "in-mv7".into(),
        }),
        IpcRequest::ApplyAction(AppAction::SetMute {
            device_id: "in-mv7".into(),
            muted: true,
        }),
        IpcRequest::ApplyAction(AppAction::SetEnhancementsDisabled {
            device_id: "out-hd660s2".into(),
            disabled: true,
        }),
        IpcRequest::ApplyAction(AppAction::ToggleDevicePin {
            device_id: "out-scarlett".into(),
        }),
        IpcRequest::ApplyAction(AppAction::SetDevicePinned {
            device_id: "out-scarlett".into(),
            pinned: true,
        }),
        IpcRequest::ApplyAction(AppAction::SetListenEnabled {
            device_id: "in-mv7".into(),
            enabled: true,
        }),
        IpcRequest::ApplyAction(AppAction::SetListenTarget {
            device_id: "in-mv7".into(),
            target_device_id: Some(LISTEN_TARGET_COMMS_OUTPUT.into()),
        }),
        IpcRequest::ApplyAction(AppAction::ApplyProfile {
            profile_id: "gaming".into(),
        }),
        IpcRequest::ApplyAction(AppAction::SaveProfile {
            name: "Competitive".into(),
        }),
        IpcRequest::ApplyAction(AppAction::DeleteProfile {
            profile_id: "gaming".into(),
        }),
    ];

    for req in requests {
        let json = serde_json::to_string(&req).unwrap();
        let back: IpcRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }
}

#[test]
fn semver_parsing_and_comparison_handle_prefixes_suffixes_and_invalid() {
    assert_eq!(parse_semver("0.1.0"), Some((0, 1, 0, None)));
    assert_eq!(parse_semver("v0.1.1"), Some((0, 1, 1, None)));
    assert_eq!(
        parse_semver("V1.2.3-rc.1+build.42"),
        Some((1, 2, 3, Some("rc.1".into())))
    );
    assert_eq!(parse_semver(" 2.10.4+sha "), Some((2, 10, 4, None)));

    assert_eq!(parse_semver(""), None);
    assert_eq!(parse_semver("0.1"), None);
    assert_eq!(parse_semver("0.1.0.1"), None);
    assert_eq!(parse_semver("vv0.1.0"), None);
    assert_eq!(parse_semver("0.1.x"), None);
    assert_eq!(parse_semver("-0.1.0"), None);
    assert_eq!(parse_semver("0.1.0-"), None);
    assert_eq!(parse_semver("0.1.0-rc..1"), None);

    assert!(is_newer_version("0.1.0", "v0.1.1"));
    assert!(is_newer_version("0.1.0", "0.2.0"));
    assert!(is_newer_version("0.1.9", "0.1.10"));
    assert!(is_newer_version("0.1.9", "0.2.0-beta.1"));
    assert!(is_newer_version("0.2.0-beta.1", "0.2.0-rc.1"));
    assert!(is_newer_version("0.2.0-rc.1", "0.2.0"));
    assert!(is_newer_version("0.2.0-alpha.1", "0.2.0-alpha.2"));
    assert!(is_newer_version("0.2.0-beta.2", "0.2.0-beta.11"));
    assert!(is_newer_version("0.2.0-alpha.1", "0.2.0-alpha.beta"));

    assert!(!is_newer_version("0.1.0", "0.1.0"));
    assert!(!is_newer_version("0.1.0", "v0.1.0"));
    assert!(!is_newer_version("0.2.0", "0.1.9"));
    assert!(!is_newer_version("0.2.0", "0.2.0-rc.1"));
    assert!(!is_newer_version("0.2.0-rc.1", "0.2.0-beta.1"));
    assert!(!is_newer_version("0.2.0-alpha.2", "0.2.0-alpha.1"));
    assert!(!is_newer_version("0.1.0", "invalid"));
    assert!(!is_newer_version("invalid", "0.2.0"));

    assert!(is_valid_version_dir_name("0.2.0"));
    assert!(is_valid_version_dir_name("v0.2.0-rc.1"));
    assert!(is_valid_version_dir_name("0.2.0-alpha-1"));
    assert!(!is_valid_version_dir_name("0.2.0-alpha_1"));
    assert!(!is_valid_version_dir_name(""));
    assert!(!is_valid_version_dir_name(".."));
    assert!(!is_valid_version_dir_name("../0.2.0"));
    assert!(!is_valid_version_dir_name("0.2.0/foo"));
    assert!(!is_valid_version_dir_name("0.2.0\\foo"));
    assert!(!is_valid_version_dir_name("0.2.0+build1"));
    assert!(!is_valid_version_dir_name("not-a-version"));
}

#[test]
fn update_manifest_round_trips_json_and_constants_are_valid() {
    assert!(parse_semver(CURRENT_VERSION).is_some());
    assert_eq!(IDLE_RESTART_MS, 300_000);
    assert!(staging_root_dir().ends_with("staging"));

    let manifest = UpdateManifest {
        version: "0.2.0".into(),
        published_at: Some("2026-11-18T12:00:00Z".into()),
        sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
        asset_url: "https://github.com/example/repo/releases/download/v0.2.0/app.exe".into(),
    };
    let json = serde_json::to_string(&manifest).unwrap();
    let back: UpdateManifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back, manifest);

    let without_published = r#"{
        "version": "0.2.1",
        "sha256": "deadbeef",
        "asset_url": "https://example.com/app.exe"
    }"#;
    let parsed: UpdateManifest = serde_json::from_str(without_published).unwrap();
    assert_eq!(parsed.version, "0.2.1");
    assert_eq!(parsed.published_at, None);
    assert_eq!(parsed.sha256, "deadbeef");
    assert_eq!(parsed.asset_url, "https://example.com/app.exe");
}

#[test]
fn auto_disable_unused_devices_keeps_roles_pins_and_listen_targets_enabled() {
    let mut state = default_mock_state();
    state.separate_comms_role = true;
    state.set_comms(DeviceFlow::Output, "out-arctis-chat");
    state.set_listen_enabled("in-mv7", true);
    state.set_listen_target("in-mv7", Some("out-lg-hdmi".into()));
    state.toggle_device_pin("in-scarlett-line");

    assert!(state.is_device_in_use(state.device("out-hd660s2").unwrap()));
    assert!(state.is_device_in_use(state.device("out-arctis-chat").unwrap()));
    assert!(!state.is_device_in_use(state.device("out-scarlett").unwrap()));
    assert!(state.is_device_in_use(state.device("out-lg-hdmi").unwrap()));
    assert!(state.is_device_in_use(state.device("in-mv7").unwrap()));
    assert!(!state.is_device_in_use(state.device("in-arctis-mic").unwrap()));
    assert!(state.is_device_in_use(state.device("in-scarlett-line").unwrap()));

    state.apply_action(&AppAction::SetAutoDisableUnusedDevices(true));
    assert!(state.auto_disable_unused_devices);
    assert!(!state.device("out-hd660s2").unwrap().disabled);
    assert!(!state.device("out-arctis-chat").unwrap().disabled);
    assert!(state.device("out-scarlett").unwrap().disabled);
    assert!(!state.device("out-lg-hdmi").unwrap().disabled);
    assert!(!state.device("in-mv7").unwrap().disabled);
    assert!(state.device("in-arctis-mic").unwrap().disabled);
    assert!(!state.device("in-scarlett-line").unwrap().disabled);

    // An unused (and disabled) input's Listen target must not keep an unused output enabled.
    state.set_listen_enabled("in-arctis-mic", true);
    state.set_listen_target("in-arctis-mic", Some("out-scarlett".into()));
    assert!(state.device("in-arctis-mic").unwrap().disabled);
    assert!(state.device("out-scarlett").unwrap().disabled);
    assert!(!state.is_device_in_use(state.device("out-scarlett").unwrap()));
}

#[test]
fn set_default_and_pin_update_disabled_state_when_auto_disable_is_enabled() {
    let mut state = default_mock_state();
    // Unpin `out-hd660s2` so switching default away from it makes it unused.
    state.toggle_device_pin("out-hd660s2");
    state.set_auto_disable_unused_devices(true);
    assert!(!state.device("out-hd660s2").unwrap().disabled);
    assert!(state.device("out-scarlett").unwrap().disabled);

    state.set_default(DeviceFlow::Output, "out-scarlett");
    assert!(!state.device("out-scarlett").unwrap().disabled);
    assert!(state.device("out-hd660s2").unwrap().disabled);

    // Pinning an unused disabled device re-enables it; unpinning disables it again.
    state.toggle_device_pin("out-lg-hdmi");
    assert!(!state.device("out-lg-hdmi").unwrap().disabled);
    state.toggle_device_pin("out-lg-hdmi");
    assert!(state.device("out-lg-hdmi").unwrap().disabled);

    // Turning off listen or retargeting updates target output disabled state immediately.
    state.set_listen_enabled("in-mv7", true);
    state.set_listen_target("in-mv7", Some("out-lg-hdmi".into()));
    assert!(!state.device("out-lg-hdmi").unwrap().disabled);
    state.set_listen_enabled("in-mv7", false);
    assert!(state.device("out-lg-hdmi").unwrap().disabled);

    // Applying a profile enables the profile's target roles and disables unused devices.
    assert!(state.apply_profile("work-calls"));
    assert!(!state.device("out-arctis-chat").unwrap().disabled);
    assert!(!state.device("in-arctis-mic").unwrap().disabled);
    assert!(state.device("out-scarlett").unwrap().disabled);
}

#[test]
fn manual_disable_and_reenable_all_respect_auto_disable_mode() {
    let mut state = default_mock_state();

    state.apply_action(&AppAction::SetDeviceDisabled {
        device_id: "out-scarlett".into(),
        disabled: true,
    });
    assert!(state.device("out-scarlett").unwrap().disabled);

    state.apply_action(&AppAction::SetDeviceDisabled {
        device_id: "out-scarlett".into(),
        disabled: false,
    });
    assert!(!state.device("out-scarlett").unwrap().disabled);

    // Selecting a manually disabled device via set_default or set_comms clears disabled.
    state.set_device_disabled("out-scarlett", true);
    state.set_device_disabled("out-lg-hdmi", true);
    state.separate_comms_role = true;
    state.set_default(DeviceFlow::Output, "out-scarlett");
    state.set_comms(DeviceFlow::Output, "out-lg-hdmi");
    assert!(!state.device("out-scarlett").unwrap().disabled);
    assert!(!state.device("out-lg-hdmi").unwrap().disabled);

    // SetDeviceDisabled is ignored while auto_disable_unused_devices is active.
    state.set_auto_disable_unused_devices(true);
    assert!(state.device("out-arctis-chat").unwrap().disabled);
    state.apply_action(&AppAction::SetDeviceDisabled {
        device_id: "out-arctis-chat".into(),
        disabled: false,
    });
    assert!(state.device("out-arctis-chat").unwrap().disabled);

    // ReenableAllDevices clears disabled across all devices.
    state.apply_action(&AppAction::ReenableAllDevices);
    assert!(state.devices.iter().all(|d| !d.disabled));
}

#[test]
fn reconcile_live_devices_preserves_cached_audio_when_disabled_and_prefers_enabled_fallback() {
    let mut state = default_mock_state();
    let out1_id = "{0.0.0.00000000}.{aaaa1111-1111-1111-1111-111111111111}";
    let out2_id = "{0.0.0.00000000}.{bbbb2222-2222-2222-2222-222222222222}";
    let in1_id = "{0.0.1.00000000}.{cccc3333-3333-3333-3333-333333333333}";

    let mut initial_out1 = live_device(out1_id, "Out 1", DeviceFlow::Output);
    initial_out1.volume = 83;
    initial_out1.muted = true;
    let initial_out2 = live_device(out2_id, "Out 2", DeviceFlow::Output);
    let initial_in1 = live_device(in1_id, "In 1", DeviceFlow::Input);

    state.reconcile_live_devices(
        vec![initial_out1, initial_out2, initial_in1],
        RoleAssignments {
            default_output_id: out2_id.into(),
            comms_output_id: out2_id.into(),
            default_input_id: in1_id.into(),
            comms_input_id: in1_id.into(),
        },
    );

    // When `out1` transitions to `disabled = true`, live probe reports placeholder 0/false,
    // which must be replaced by cached volume=83 and muted=true.
    let mut disabled_out1 = live_device(out1_id, "Out 1", DeviceFlow::Output);
    disabled_out1.disabled = true;
    disabled_out1.volume = 0;
    disabled_out1.muted = false;
    let enabled_out2 = live_device(out2_id, "Out 2", DeviceFlow::Output);
    let enabled_in1 = live_device(in1_id, "In 1", DeviceFlow::Input);

    state.lock_enabled = false;
    state.reconcile_live_devices(
        vec![disabled_out1, enabled_out2, enabled_in1],
        RoleAssignments {
            default_output_id: String::new(),
            comms_output_id: String::new(),
            default_input_id: String::new(),
            comms_input_id: String::new(),
        },
    );

    let out1 = state.device(out1_id).unwrap();
    assert!(out1.disabled);
    assert_eq!(out1.volume, 83);
    assert!(out1.muted);

    // Fallback role skipped disabled `out1` and picked enabled `out2`.
    assert_eq!(state.roles.default_output_id, out2_id);
    assert_eq!(state.roles.comms_output_id, out2_id);

    // If all devices of a flow are disabled, fallback still picks the first device of that flow.
    let mut all_disabled_out1 = live_device(out1_id, "Out 1", DeviceFlow::Output);
    all_disabled_out1.disabled = true;
    let mut all_disabled_out2 = live_device(out2_id, "Out 2", DeviceFlow::Output);
    all_disabled_out2.disabled = true;
    state.reconcile_live_devices(
        vec![all_disabled_out1, all_disabled_out2],
        RoleAssignments {
            default_output_id: String::new(),
            comms_output_id: String::new(),
            default_input_id: String::new(),
            comms_input_id: String::new(),
        },
    );
    assert_eq!(state.roles.default_output_id, out1_id);
}

#[test]
fn legacy_state_json_without_disabled_fields_deserializes_with_false() {
    let state = default_mock_state();
    let mut value = serde_json::to_value(&state).unwrap();
    let obj = value.as_object_mut().unwrap();
    obj.remove("auto_disable_unused_devices");
    for dev in obj.get_mut("devices").unwrap().as_array_mut().unwrap() {
        dev.as_object_mut().unwrap().remove("disabled");
    }

    let legacy_json = serde_json::to_string(&value).unwrap();
    let loaded: AppState = serde_json::from_str(&legacy_json).unwrap();
    assert!(!loaded.auto_disable_unused_devices);
    assert!(loaded.devices.iter().all(|d| !d.disabled));
    assert_eq!(loaded, state);
}

#[test]
fn reconcile_live_devices_preserves_locked_real_id_across_unplug_and_replug() {
    let out_usb = "{0.0.0.00000000}.{aaaa1111-1111-1111-1111-111111111111}";
    let out_hdmi = "{0.0.0.00000000}.{bbbb2222-2222-2222-2222-222222222222}";
    let in_usb = "{0.0.1.00000000}.{cccc3333-3333-3333-3333-333333333333}";
    let in_aux = "{0.0.1.00000000}.{dddd4444-4444-4444-4444-444444444444}";

    let mut state = AppState::default();
    state.reconcile_live_devices(
        vec![
            live_device(out_usb, "USB DAC", DeviceFlow::Output),
            live_device(out_hdmi, "Monitor HDMI", DeviceFlow::Output),
            live_device(in_usb, "USB Mic", DeviceFlow::Input),
            live_device(in_aux, "Line In", DeviceFlow::Input),
        ],
        RoleAssignments {
            default_output_id: out_usb.into(),
            comms_output_id: out_usb.into(),
            default_input_id: in_usb.into(),
            comms_input_id: in_usb.into(),
        },
    );

    // Temporarily unplug USB DAC and USB Mic while lock_enabled is true.
    state.reconcile_live_devices(
        vec![
            live_device(out_hdmi, "Monitor HDMI", DeviceFlow::Output),
            live_device(in_aux, "Line In", DeviceFlow::Input),
        ],
        RoleAssignments {
            default_output_id: out_hdmi.into(),
            comms_output_id: out_hdmi.into(),
            default_input_id: in_aux.into(),
            comms_input_id: in_aux.into(),
        },
    );

    assert_eq!(state.roles.default_output_id, out_usb);
    assert_eq!(state.roles.comms_output_id, out_usb);
    assert_eq!(state.roles.default_input_id, in_usb);
    assert_eq!(state.roles.comms_input_id, in_usb);

    // Replugging restores the devices under the still-locked role IDs.
    state.reconcile_live_devices(
        vec![
            live_device(out_usb, "USB DAC", DeviceFlow::Output),
            live_device(out_hdmi, "Monitor HDMI", DeviceFlow::Output),
            live_device(in_usb, "USB Mic", DeviceFlow::Input),
            live_device(in_aux, "Line In", DeviceFlow::Input),
        ],
        RoleAssignments {
            default_output_id: out_hdmi.into(),
            comms_output_id: out_hdmi.into(),
            default_input_id: in_aux.into(),
            comms_input_id: in_aux.into(),
        },
    );

    assert_eq!(state.roles.default_output_id, out_usb);
    assert_eq!(state.roles.comms_output_id, out_usb);
    assert_eq!(state.roles.default_input_id, in_usb);
    assert_eq!(state.roles.comms_input_id, in_usb);
}

#[test]
fn set_separate_comms_role_false_and_apply_profile_unify_comms_roles() {
    let mut state = default_mock_state();
    state.apply_action(&AppAction::SetSeparateCommsRole(true));
    state.apply_action(&AppAction::SetDefault {
        flow: DeviceFlow::Output,
        device_id: "out-hd660s2".into(),
    });
    state.apply_action(&AppAction::SetComms {
        flow: DeviceFlow::Output,
        device_id: "out-arctis-chat".into(),
    });
    state.apply_action(&AppAction::SetDefault {
        flow: DeviceFlow::Input,
        device_id: "in-mv7".into(),
    });
    state.apply_action(&AppAction::SetComms {
        flow: DeviceFlow::Input,
        device_id: "in-arctis-mic".into(),
    });
    state.apply_action(&AppAction::SetAutoDisableUnusedDevices(true));
    assert!(!state.device("out-arctis-chat").unwrap().disabled);
    assert!(!state.device("in-arctis-mic").unwrap().disabled);

    // Turning separate_comms_role off unifies comms roles and disables no-longer-used comms devices.
    state.apply_action(&AppAction::SetSeparateCommsRole(false));
    assert!(!state.separate_comms_role);
    assert_eq!(state.roles.comms_output_id, "out-hd660s2");
    assert_eq!(state.roles.comms_input_id, "in-mv7");
    assert!(state.device("out-arctis-chat").unwrap().disabled);
    assert!(state.device("in-arctis-mic").unwrap().disabled);

    // Applying a profile with split roles while separate_comms_role is false unifies comms_*_id.
    state.profiles.push(AudioProfile {
        id: "split".into(),
        name: "Split".into(),
        roles: RoleAssignments {
            default_output_id: "out-scarlett".into(),
            comms_output_id: "out-arctis-chat".into(),
            default_input_id: "in-scarlett-line".into(),
            comms_input_id: "in-arctis-mic".into(),
        },
        device_settings: vec![],
    });
    assert!(state.apply_profile("split"));
    assert_eq!(state.roles.default_output_id, "out-scarlett");
    assert_eq!(state.roles.comms_output_id, "out-scarlett");
    assert_eq!(state.roles.default_input_id, "in-scarlett-line");
    assert_eq!(state.roles.comms_input_id, "in-scarlett-line");
    assert!(state.device("out-arctis-chat").unwrap().disabled);
    assert!(state.device("in-arctis-mic").unwrap().disabled);
}

#[test]
fn set_device_disabled_refuses_last_enabled_and_reassigns_active_roles() {
    let mut state = AppState {
        devices: vec![
            live_device("out-1", "Out 1", DeviceFlow::Output),
            live_device("out-2", "Out 2", DeviceFlow::Output),
            live_device("in-1", "In 1", DeviceFlow::Input),
        ],
        roles: RoleAssignments {
            default_output_id: "out-1".into(),
            comms_output_id: "out-1".into(),
            default_input_id: "in-1".into(),
            comms_input_id: "in-1".into(),
        },
        ..AppState::default()
    };

    // Disabling active default/comms output reassigns both roles to the remaining enabled output.
    assert!(state.set_device_disabled("out-1", true));
    assert!(state.device("out-1").unwrap().disabled);
    assert_eq!(state.roles.default_output_id, "out-2");
    assert_eq!(state.roles.comms_output_id, "out-2");

    // Refuses to disable the last enabled output and last enabled input.
    assert!(!state.set_device_disabled("out-2", true));
    assert!(!state.device("out-2").unwrap().disabled);
    assert!(!state.set_device_disabled("in-1", true));
    assert!(!state.device("in-1").unwrap().disabled);
}

#[test]
fn save_state_to_atomic_overwrite_round_trip() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("audio-lock-atomic-save-{unique}"));
    let path = dir.join("state.json");

    let mut state = default_mock_state();
    save_state_to(&path, &state).unwrap();

    for volume in [15, 55, 88] {
        state.set_volume("out-hd660s2", volume);
        save_state_to(&path, &state).unwrap();
    }

    assert!(!path.with_extension("json.tmp").exists());
    let loaded = load_state_from(&path);
    assert_eq!(loaded, state);
    assert_eq!(loaded.device("out-hd660s2").unwrap().volume, 88);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reconcile_live_devices_preserves_listen_sentinels_accurately() {
    let out_default = "{0.0.0.00000000}.{11111111-1111-1111-1111-111111111111}";
    let out_comms = "{0.0.0.00000000}.{22222222-2222-2222-2222-222222222222}";
    let out_other = "{0.0.0.00000000}.{33333333-3333-3333-3333-333333333333}";
    let in_a = "{0.0.1.00000000}.{44444444-4444-4444-4444-444444444444}";
    let in_b = "{0.0.1.00000000}.{55555555-5555-5555-5555-555555555555}";

    let mut state = AppState {
        separate_comms_role: true,
        roles: RoleAssignments {
            default_output_id: out_default.into(),
            comms_output_id: out_comms.into(),
            default_input_id: in_a.into(),
            comms_input_id: in_a.into(),
        },
        devices: vec![
            live_device(out_default, "Default Out", DeviceFlow::Output),
            live_device(out_comms, "Comms Out", DeviceFlow::Output),
            live_device(out_other, "Other Out", DeviceFlow::Output),
            AudioDevice {
                listen: Some(ListenConfig {
                    enabled: false,
                    target_device_id: Some(LISTEN_TARGET_DEFAULT_OUTPUT.into()),
                }),
                ..live_device(in_a, "Mic A", DeviceFlow::Input)
            },
            AudioDevice {
                listen: Some(ListenConfig {
                    enabled: false,
                    target_device_id: Some(LISTEN_TARGET_COMMS_OUTPUT.into()),
                }),
                ..live_device(in_b, "Mic B", DeviceFlow::Input)
            },
        ],
        ..AppState::default()
    };

    // When live_listen.enabled == false, both sentinels are preserved even if Windows reports None.
    let mut live_in_a = live_device(in_a, "Mic A", DeviceFlow::Input);
    live_in_a.listen = Some(ListenConfig {
        enabled: false,
        target_device_id: None,
    });
    let mut live_in_b = live_device(in_b, "Mic B", DeviceFlow::Input);
    live_in_b.listen = Some(ListenConfig {
        enabled: false,
        target_device_id: Some(out_other.into()),
    });

    state.reconcile_live_devices(
        vec![
            live_device(out_default, "Default Out", DeviceFlow::Output),
            live_device(out_comms, "Comms Out", DeviceFlow::Output),
            live_device(out_other, "Other Out", DeviceFlow::Output),
            live_in_a,
            live_in_b,
        ],
        state.roles.clone(),
    );

    assert_eq!(
        state.device(in_a).unwrap().listen,
        Some(ListenConfig {
            enabled: false,
            target_device_id: Some(LISTEN_TARGET_DEFAULT_OUTPUT.into()),
        })
    );
    assert_eq!(
        state.device(in_b).unwrap().listen,
        Some(ListenConfig {
            enabled: false,
            target_device_id: Some(LISTEN_TARGET_COMMS_OUTPUT.into()),
        })
    );

    // When live_listen.enabled == true, LISTEN_TARGET_COMMS_OUTPUT is ONLY substituted if
    // live target matches comms_output_id, NOT if live target was changed to another device.
    let mut active_in_b = live_device(in_b, "Mic B", DeviceFlow::Input);
    active_in_b.listen = Some(ListenConfig {
        enabled: true,
        target_device_id: Some(out_other.into()),
    });
    state.reconcile_live_devices(
        vec![
            live_device(out_default, "Default Out", DeviceFlow::Output),
            live_device(out_comms, "Comms Out", DeviceFlow::Output),
            live_device(out_other, "Other Out", DeviceFlow::Output),
            live_device(in_a, "Mic A", DeviceFlow::Input),
            active_in_b,
        ],
        state.roles.clone(),
    );
    assert_eq!(
        state.device(in_b).unwrap().listen,
        Some(ListenConfig {
            enabled: true,
            target_device_id: Some(out_other.into()),
        })
    );
}

#[test]
fn previous_spatial_format_defaults_round_trips_reconciles_and_clears_on_undo() {
    let dolby_guid = "{b53d940c-0102-4033-832d-19a81242006d}";
    let sonic_guid = "{b53d940c-0102-4033-832d-19a81242006e}";

    // Legacy EnhancementsState JSON without `previous_spatial_format` deserializes with None.
    let legacy_json =
        r#"{"sys_fx_enabled":true,"advanced_enabled":false,"spatial_sound_enabled":true}"#;
    let legacy: EnhancementsState = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(
        legacy,
        EnhancementsState {
            sys_fx_enabled: true,
            advanced_enabled: false,
            spatial_sound_enabled: true,
            previous_spatial_format: None,
        }
    );

    let out_id = "{0.0.0.00000000}.{11111111-1111-1111-1111-111111111111}";
    let mut initial = live_device(out_id, "Headphones", DeviceFlow::Output);
    initial.enhancements.spatial_sound_enabled = true;

    let mut state = AppState {
        devices: vec![initial],
        roles: RoleAssignments {
            default_output_id: out_id.into(),
            comms_output_id: out_id.into(),
            ..RoleAssignments::default()
        },
        ..AppState::default()
    };

    // Disable enhancements and record prior Spatial Sound format; repeated recording does not overwrite.
    state.set_enhancements_disabled(out_id, true);
    assert!(state.record_previous_spatial_format(out_id, dolby_guid.into()));
    assert!(!state.record_previous_spatial_format(out_id, sonic_guid.into()));
    assert_eq!(
        state
            .device(out_id)
            .unwrap()
            .enhancements
            .previous_spatial_format
            .as_deref(),
        Some(dolby_guid)
    );

    // JSON round-trip preserves `previous_spatial_format`.
    let serialized = serde_json::to_string(&state).unwrap();
    let mut round_tripped: AppState = serde_json::from_str(&serialized).unwrap();
    assert_eq!(round_tripped, state);

    // `set_device_enhancements` from live property notifications preserves `previous_spatial_format`.
    assert!(!round_tripped.set_device_enhancements(out_id, EnhancementsState::all_disabled()));
    assert_eq!(
        round_tripped
            .device(out_id)
            .unwrap()
            .enhancements
            .previous_spatial_format
            .as_deref(),
        Some(dolby_guid)
    );

    // `reconcile_live_devices` preserves `previous_spatial_format` alongside `previous_enhancements`.
    let mut live_after = live_device(out_id, "Headphones", DeviceFlow::Output);
    live_after.enhancements = EnhancementsState::all_disabled();
    round_tripped.reconcile_live_devices(vec![live_after], round_tripped.roles.clone());
    let reconciled = round_tripped.device(out_id).unwrap();
    assert!(reconciled.previous_enhancements.is_some());
    assert_eq!(
        reconciled.enhancements.previous_spatial_format.as_deref(),
        Some(dolby_guid)
    );

    // Single-device undo restores `spatial_sound_enabled` and clears `previous_spatial_format`.
    round_tripped.set_enhancements_disabled(out_id, false);
    let restored = round_tripped.device(out_id).unwrap();
    assert!(restored.enhancements.spatial_sound_enabled);
    assert_eq!(restored.enhancements.previous_spatial_format, None);
    assert_eq!(restored.previous_enhancements, None);

    // `reenable_all_enhancements` also restores `spatial_sound_enabled` and clears `previous_spatial_format`
    // even when `previous_enhancements` is None.
    round_tripped.disable_all_enhancements(out_id);
    assert!(round_tripped.record_previous_spatial_format(out_id, sonic_guid.into()));
    round_tripped.reenable_all_enhancements();
    let restored_all = round_tripped.device(out_id).unwrap();
    assert!(restored_all.enhancements.spatial_sound_enabled);
    assert_eq!(restored_all.enhancements.previous_spatial_format, None);
}
