/// At-rest protection for the MQTT password using Windows DPAPI (user scope):
/// only the same Windows account on the same machine can decrypt. Encrypted
/// values are stored as `dpapi:<hex>`; a value without that prefix is treated
/// as legacy plaintext and gets encrypted transparently on the next save.
///
/// Failure policy: encryption failure falls back to plaintext-with-warning
/// (losing the broker connection over a crypto hiccup is worse than the status
/// quo ante); decryption failure yields an empty password (the value is
/// unrecoverable anyway — different user or machine).
const PREFIX: &str = "dpapi:";

pub fn conceal(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    match protect(plain.as_bytes()) {
        Some(blob) => format!("{PREFIX}{}", hex_encode(&blob)),
        None => {
            log::warn!("DPAPI encrypt failed; storing MQTT password unprotected");
            plain.to_string()
        }
    }
}

pub fn reveal(stored: &str) -> String {
    let Some(hex) = stored.strip_prefix(PREFIX) else {
        return stored.to_string(); // Legacy plaintext — re-encrypted on next save.
    };
    let Some(blob) = hex_decode(hex) else {
        log::warn!("DPAPI value malformed; treating password as empty");
        return String::new();
    };
    match unprotect(&blob) {
        Some(bytes) => String::from_utf8(bytes).unwrap_or_default(),
        None => {
            log::warn!("DPAPI decrypt failed (other user/machine?); treating password as empty");
            String::new()
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Option<Vec<u8>> {
    use windows::core::PCWSTR;
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .ok()?;
        Some(take_blob(output))
    }
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .ok()?;
        Some(take_blob(output))
    }
}

/// Copy a DPAPI output blob into owned memory and free the LocalAlloc'd original.
#[cfg(windows)]
unsafe fn take_blob(blob: windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB) -> Vec<u8> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    let bytes = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
    let _ = LocalFree(Some(HLOCAL(blob.pbData as *mut core::ffi::c_void)));
    bytes
}

#[cfg(not(windows))]
fn protect(_data: &[u8]) -> Option<Vec<u8>> {
    None
}

#[cfg(not(windows))]
fn unprotect(_data: &[u8]) -> Option<Vec<u8>> {
    None
}
