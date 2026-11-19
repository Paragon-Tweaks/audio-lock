use std::fmt::Write;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use windows::Win32::Security::Cryptography::{
    BCRYPT_ALG_HANDLE, BCRYPT_HANDLE, BCRYPT_HASH_HANDLE, BCRYPT_HASH_LENGTH, BCRYPT_OBJECT_LENGTH,
    BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS, BCRYPT_SHA256_ALGORITHM, BCryptCloseAlgorithmProvider,
    BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptGetProperty, BCryptHashData,
    BCryptOpenAlgorithmProvider,
};
use windows::core::PCWSTR;

const CHUNK_SIZE: usize = 16 * 1024;
const SHA256_LEN: usize = 32;

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

/// Verification hook for Authenticode (`WinVerifyTrust`) once release binaries are code-signed.
pub fn verify_authenticode_if_required(_exe_path: &Path) -> Result<(), String> {
    Ok(())
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
}
