use std::fmt::Write;
use std::fs::File;
use std::io::Read;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use windows::Win32::Foundation::{HWND, INVALID_HANDLE_VALUE};
use windows::Win32::Security::Cryptography::{
    BCRYPT_ALG_HANDLE, BCRYPT_HANDLE, BCRYPT_HASH_HANDLE, BCRYPT_HASH_LENGTH, BCRYPT_OBJECT_LENGTH,
    BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS, BCRYPT_SHA256_ALGORITHM, BCryptCloseAlgorithmProvider,
    BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptGetProperty, BCryptHashData,
    BCryptOpenAlgorithmProvider, CERT_CONTEXT, CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW,
};
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_NONE, WTD_REVOKE_NONE, WTD_SAFER_FLAG,
    WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvCertFromChain,
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows::core::{GUID, PCWSTR};

const CHUNK_SIZE: usize = 16 * 1024;
const SHA256_LEN: usize = 32;

const REQUIRE_AUTHENTICODE: bool = match option_env!("AUDIO_LOCK_REQUIRE_AUTHENTICODE") {
    Some(value) => matches!(value.as_bytes(), b"1" | b"true" | b"TRUE"),
    None => false,
};
const EXPECTED_PUBLISHER: Option<&str> = option_env!("AUDIO_LOCK_EXPECTED_PUBLISHER");

struct AlgHandle(BCRYPT_ALG_HANDLE);

impl Drop for AlgHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = BCryptCloseAlgorithmProvider(self.0, 0);
        }
    }
}

struct HashHandle(BCRYPT_HASH_HANDLE);

impl Drop for HashHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = BCryptDestroyHash(self.0);
        }
    }
}

fn query_u32_property(alg: BCRYPT_ALG_HANDLE, property: PCWSTR) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    let mut copied = 0u32;
    let status = unsafe {
        BCryptGetProperty(
            BCRYPT_HANDLE(alg.0),
            property,
            Some(&mut bytes),
            &mut copied,
            0,
        )
    };
    status
        .ok()
        .map_err(|err| format!("BCryptGetProperty failed: {err}"))?;
    if copied != 4 {
        return Err(format!("Unexpected BCrypt property size: {copied}"));
    }
    Ok(u32::from_ne_bytes(bytes))
}

pub fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let mut raw_alg = BCRYPT_ALG_HANDLE::default();
    unsafe {
        BCryptOpenAlgorithmProvider(
            &mut raw_alg,
            BCRYPT_SHA256_ALGORITHM,
            PCWSTR::null(),
            BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0),
        )
    }
    .ok()
    .map_err(|err| format!("BCryptOpenAlgorithmProvider failed: {err}"))?;
    let alg = AlgHandle(raw_alg);

    let obj_len = query_u32_property(alg.0, BCRYPT_OBJECT_LENGTH)? as usize;
    let hash_len = query_u32_property(alg.0, BCRYPT_HASH_LENGTH)? as usize;
    if hash_len != SHA256_LEN {
        return Err(format!("Unexpected SHA-256 length: {hash_len}"));
    }

    let mut hash_obj = vec![0u8; obj_len];
    let mut raw_hash = BCRYPT_HASH_HANDLE::default();
    unsafe { BCryptCreateHash(alg.0, &mut raw_hash, Some(&mut hash_obj), None, 0) }
        .ok()
        .map_err(|err| format!("BCryptCreateHash failed: {err}"))?;
    let hash = HashHandle(raw_hash);

    let mut file = File::open(path).map_err(|err| format!("Failed to open file for hashing: {err}"))?;
    let mut buf = [0u8; CHUNK_SIZE];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|err| format!("Failed to read file for hashing: {err}"))?;
        if read == 0 {
            break;
        }
        unsafe { BCryptHashData(hash.0, &buf[..read], 0) }
            .ok()
            .map_err(|err| format!("BCryptHashData failed: {err}"))?;
    }

    let mut digest = [0u8; SHA256_LEN];
    unsafe { BCryptFinishHash(hash.0, &mut digest, 0) }
        .ok()
        .map_err(|err| format!("BCryptFinishHash failed: {err}"))?;

    let mut hex = String::with_capacity(SHA256_LEN * 2);
    for byte in digest {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    Ok(hex)
}

pub fn is_valid_sha256_hex(hex: &str) -> bool {
    hex.len() == SHA256_LEN * 2 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn verify_sha256(path: &Path, expected_hex: &str) -> Result<(), String> {
    let expected = expected_hex.trim();
    if !is_valid_sha256_hex(expected) {
        return Err("Invalid expected SHA-256 hex string".to_string());
    }

    let actual = sha256_file_hex(path)?;
    let mut diff = 0u8;
    for (a, b) in actual.bytes().zip(expected.bytes()) {
        diff |= a ^ b.to_ascii_lowercase();
    }

    if diff != 0 {
        return Err(format!(
            "SHA-256 mismatch (expected {}, got {actual})",
            expected.to_ascii_lowercase()
        ));
    }
    Ok(())
}

struct WinTrustStateGuard<'a> {
    action_id: GUID,
    data: &'a mut WINTRUST_DATA,
}

impl Drop for WinTrustStateGuard<'_> {
    fn drop(&mut self) {
        self.data.dwStateAction = WTD_STATEACTION_CLOSE;
        unsafe {
            let _ = WinVerifyTrust(
                HWND(INVALID_HANDLE_VALUE.0),
                ptr::from_mut(&mut self.action_id),
                ptr::from_mut::<WINTRUST_DATA>(self.data).cast(),
            );
        }
    }
}

unsafe fn extract_cert_simple_name(cert_ctx: *const CERT_CONTEXT) -> Result<String, String> {
    let required =
        unsafe { CertGetNameStringW(cert_ctx, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, None) };
    if required <= 1 {
        return Err("Signer certificate has no display name".to_string());
    }

    let mut buf = vec![0u16; required as usize];
    let written = unsafe {
        CertGetNameStringW(
            cert_ctx,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            None,
            Some(&mut buf),
        )
    };
    if written <= 1 {
        return Err("Failed to read signer certificate display name".to_string());
    }

    let used = (written as usize).min(buf.len());
    let slice = &buf[..used];
    let end = slice.iter().position(|&ch| ch == 0).unwrap_or(slice.len());
    let name = String::from_utf16(&slice[..end])
        .map_err(|_| "Signer certificate display name is not valid UTF-16".to_string())?;
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        return Err("Signer certificate display name is empty".to_string());
    }
    Ok(trimmed)
}

fn verify_and_extract_signer(exe_path: &Path) -> Result<String, String> {
    let wide_path: Vec<u16> = exe_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();

    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wide_path.as_ptr()),
        ..Default::default()
    };

    let mut trust_data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: ptr::from_mut(&mut file_info),
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_SAFER_FLAG | WTD_REVOCATION_CHECK_NONE,
        ..Default::default()
    };

    let mut action_id = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let status = unsafe {
        WinVerifyTrust(
            HWND(INVALID_HANDLE_VALUE.0),
            ptr::from_mut(&mut action_id),
            ptr::from_mut::<WINTRUST_DATA>(&mut trust_data).cast(),
        )
    };
    let state_guard = WinTrustStateGuard {
        action_id,
        data: &mut trust_data,
    };

    if status != 0 {
        return Err(format!(
            "Authenticode verification failed for {} (WinVerifyTrust status: 0x{:08x})",
            exe_path.display(),
            status as u32
        ));
    }

    let state_data = state_guard.data.hWVTStateData;
    if state_data.is_invalid() {
        return Err(format!(
            "Authenticode verification returned no state data for {}",
            exe_path.display()
        ));
    }

    let prov_data = unsafe { WTHelperProvDataFromStateData(state_data) };
    if prov_data.is_null() {
        return Err(format!(
            "WTHelperProvDataFromStateData returned null for {}",
            exe_path.display()
        ));
    }

    let signer = unsafe { WTHelperGetProvSignerFromChain(prov_data, 0, false, 0) };
    if signer.is_null() {
        return Err(format!(
            "WTHelperGetProvSignerFromChain returned null for {}",
            exe_path.display()
        ));
    }

    let prov_cert = unsafe { WTHelperGetProvCertFromChain(signer, 0) };
    if prov_cert.is_null() {
        return Err(format!(
            "WTHelperGetProvCertFromChain returned null for {}",
            exe_path.display()
        ));
    }

    let cert_ctx = unsafe { (*prov_cert).pCert };
    if cert_ctx.is_null() {
        return Err(format!(
            "Signer certificate context is null for {}",
            exe_path.display()
        ));
    }

    unsafe { extract_cert_simple_name(cert_ctx) }
}

fn expected_publisher() -> Option<&'static str> {
    EXPECTED_PUBLISHER.map(str::trim).filter(|s| !s.is_empty())
}

fn publishers_match(actual: &str, expected: &str) -> bool {
    actual.trim().to_lowercase() == expected.trim().to_lowercase()
}

/// Verifies a PE file's Windows Authenticode signature via `WinVerifyTrust` and confirms
/// its signer certificate display name matches the expected publisher (or the running binary's
/// publisher when `AUDIO_LOCK_EXPECTED_PUBLISHER` is not set at compile time).
pub fn verify_authenticode(exe_path: &Path) -> Result<(), String> {
    let staged_signer = verify_and_extract_signer(exe_path)?;

    let expected = match expected_publisher() {
        Some(publisher) => publisher.to_string(),
        None => {
            let current_exe = std::env::current_exe().map_err(|err| {
                format!("Failed to resolve current executable for publisher verification: {err}")
            })?;
            verify_and_extract_signer(&current_exe).map_err(|err| {
                format!(
                    "Failed to verify current executable signature for publisher comparison: {err}"
                )
            })?
        }
    };

    if !publishers_match(&staged_signer, &expected) {
        return Err(format!(
            "Authenticode signer mismatch (expected '{expected}', got '{staged_signer}')"
        ));
    }

    Ok(())
}

/// Enforces Windows Authenticode verification when compiled with `AUDIO_LOCK_REQUIRE_AUTHENTICODE`.
pub fn verify_authenticode_if_required(exe_path: &Path) -> Result<(), String> {
    if !REQUIRE_AUTHENTICODE {
        return Ok(());
    }
    verify_authenticode(exe_path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn computes_and_verifies_sha256_via_bcrypt() {
        let path = std::env::temp_dir().join(format!(
            "audio-lock-sha256-test-{}.bin",
            std::process::id()
        ));
        fs::write(&path, b"hello world").expect("write temp file");

        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let actual = sha256_file_hex(&path).expect("hash file");
        assert_eq!(actual, expected);
        assert!(verify_sha256(&path, &expected.to_ascii_uppercase()).is_ok());
        assert!(verify_sha256(&path, &"0".repeat(64)).is_err());
        assert!(verify_sha256(&path, "short").is_err());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn verify_authenticode_rejects_unsigned_file() {
        let path = std::env::temp_dir().join(format!(
            "audio-lock-authenticode-unsigned-test-{}.exe",
            std::process::id()
        ));
        fs::write(&path, b"unsigned dummy executable").expect("write temp exe");

        let err = verify_authenticode(&path)
            .expect_err("unsigned binary must fail Authenticode verification");
        assert!(
            err.contains("WinVerifyTrust"),
            "unexpected Authenticode error: {err}"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn verify_authenticode_if_required_follows_compile_time_policy() {
        let path = std::env::temp_dir().join(format!(
            "audio-lock-authenticode-policy-test-{}.exe",
            std::process::id()
        ));
        fs::write(&path, b"unsigned dummy executable").expect("write temp exe");

        let result = verify_authenticode_if_required(&path);
        if REQUIRE_AUTHENTICODE {
            assert!(result.is_err());
        } else {
            assert!(result.is_ok());
        }

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn matches_publishers_case_insensitively() {
        assert!(publishers_match("Tulantro LLC", "tulantro llc"));
        assert!(publishers_match("  Tulantro LLC ", "TULANTRO LLC"));
        assert!(!publishers_match("Other Publisher Inc.", "Tulantro LLC"));
    }
}
