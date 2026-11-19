use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ,
    REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW,
    RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::PCWSTR;

use crate::win32;

const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "DefaultAudioLock";

struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn read_run_value() -> Option<String> {
    let subkey = win32::to_wide(RUN_SUBKEY);
    let value_name = win32::to_wide(VALUE_NAME);
    let mut hkey = HKEY::default();

    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            Some(0),
            KEY_QUERY_VALUE,
            &mut hkey,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let key = RegKey(hkey);

    let mut value_type = REG_VALUE_TYPE::default();
    let mut byte_len = 0u32;
    let query = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut value_type),
            None,
            Some(&mut byte_len),
        )
    };
    if query != ERROR_SUCCESS
        || (value_type != REG_SZ && value_type != REG_EXPAND_SZ)
        || byte_len < 2
    {
        return None;
    }

    let mut wide = vec![0u16; (byte_len as usize).div_ceil(2)];
    let mut read_len = (wide.len() * 2) as u32;
    let read_status = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut value_type),
            Some(wide.as_mut_ptr().cast::<u8>()),
            Some(&mut read_len),
        )
    };
    if read_status != ERROR_SUCCESS || (value_type != REG_SZ && value_type != REG_EXPAND_SZ) {
        return None;
    }

    let unit_len = (read_len as usize / 2).min(wide.len());
    let end = wide[..unit_len]
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(unit_len);
    let value = String::from_utf16_lossy(&wide[..end]);
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn current_exe_command() -> Result<String, String> {
    let exe = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve executable path: {err}"))?;
    Ok(format!("\"{}\"", exe.display()))
}

pub fn is_enabled() -> bool {
    read_run_value().is_some()
}

pub fn sync_if_enabled() -> bool {
    let Some(existing) = read_run_value() else {
        return false;
    };
    if let Ok(expected) = current_exe_command()
        && !existing.eq_ignore_ascii_case(&expected)
    {
        let _ = set_enabled(true);
    }
    true
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let subkey = win32::to_wide(RUN_SUBKEY);
    let value_name = win32::to_wide(VALUE_NAME);

    if enabled {
        let command = current_exe_command()?;
        let data_wide = win32::to_wide(&command);
        let data_bytes = unsafe {
            std::slice::from_raw_parts(data_wide.as_ptr().cast::<u8>(), data_wide.len() * 2)
        };

        let mut hkey = HKEY::default();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut hkey,
                None,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!("RegCreateKeyExW failed: {}", status.0));
        }
        let key = RegKey(hkey);

        let set_status = unsafe {
            RegSetValueExW(
                key.0,
                PCWSTR(value_name.as_ptr()),
                Some(0),
                REG_SZ,
                Some(data_bytes),
            )
        };
        if set_status != ERROR_SUCCESS {
            return Err(format!("RegSetValueExW failed: {}", set_status.0));
        }
        Ok(())
    } else {
        let mut hkey = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                KEY_SET_VALUE,
                &mut hkey,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if status != ERROR_SUCCESS {
            return Err(format!("RegOpenKeyExW failed: {}", status.0));
        }
        let key = RegKey(hkey);

        let delete_status = unsafe { RegDeleteValueW(key.0, PCWSTR(value_name.as_ptr())) };
        if delete_status == ERROR_SUCCESS || delete_status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("RegDeleteValueW failed: {}", delete_status.0))
        }
    }
}
