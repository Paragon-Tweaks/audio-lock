use windows::Media::Audio::SpatialAudioDeviceConfiguration;
use windows_core::HSTRING;

const NONE_SPATIAL_FORMAT: &str = "{00000000-0000-0000-0000-000000000000}";

fn swd_device_path(mmdevice_id: &str) -> HSTRING {
    HSTRING::from(format!(
        r"\\?\SWD#MMDEVAPI#{}#{{e6327cad-dcec-4949-ae8a-991e976a79d2}}",
        mmdevice_id
    ))
}

fn is_active_format(format: &HSTRING) -> bool {
    !format.is_empty() && !format.to_string_lossy().eq_ignore_ascii_case(NONE_SPATIAL_FORMAT)
}

pub fn is_spatial_enabled(mmdevice_id: &str) -> bool {
    let Ok(config) = SpatialAudioDeviceConfiguration::GetForDeviceId(&swd_device_path(mmdevice_id))
    else {
        return false;
    };
    if !config.IsSpatialAudioSupported().unwrap_or(false) {
        return false;
    }

    if let Ok(active) = config.ActiveSpatialAudioFormat()
        && is_active_format(&active)
    {
        return true;
    }

    config
        .DefaultSpatialAudioFormat()
        .is_ok_and(|default_fmt| is_active_format(&default_fmt))
}

pub fn disable_spatial(mmdevice_id: &str) {
    let Ok(config) = SpatialAudioDeviceConfiguration::GetForDeviceId(&swd_device_path(mmdevice_id))
    else {
        return;
    };
    if !config.IsSpatialAudioSupported().unwrap_or(false) {
        return;
    }

    let none = HSTRING::from(NONE_SPATIAL_FORMAT);
    let _ = config
        .SetDefaultSpatialAudioFormatAsync(&none)
        .and_then(|op| op.get());
}
