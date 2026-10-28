//! Windows current-user protection for persisted credentials. Other platforms keep plaintext.

#[derive(Debug, thiserror::Error)]
#[error("Saved credentials cannot be unlocked for this Windows user or are damaged. Keep the database intact and sign in with the original Windows account, or explicitly forget the affected account.")]
pub struct Error;

pub const PREFIX: &str = "limusic-dpapi:v1:";

pub fn protect(value: &str) -> Result<String, Error> {
    if value.is_empty() {
        return Ok(String::new());
    }
    #[cfg(windows)]
    {
        use base64::Engine;
        Ok(format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(crypt(value.as_bytes(), true)?)
        ))
    }
    #[cfg(not(windows))]
    Ok(value.to_owned())
}

pub fn unprotect(value: &str) -> Result<String, Error> {
    let Some(encoded) = value.strip_prefix(PREFIX) else {
        return if value.starts_with("limusic-dpapi:") { Err(Error) } else { Ok(value.to_owned()) };
    };
    #[cfg(windows)]
    {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| Error)?;
        String::from_utf8(crypt(&bytes, false)?).map_err(|_| Error)
    }
    #[cfg(not(windows))]
    {
        let _ = encoded;
        Err(Error)
    }
}

#[cfg(windows)]
fn crypt(bytes: &[u8], protect: bool) -> Result<Vec<u8>, Error> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| Error)?,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // No LOCAL_MACHINE flag: only this Windows user's DPAPI identity can unlock it.
    unsafe {
        if protect {
            CryptProtectData(
                &input,
                windows::core::PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .map_err(|_| Error)?;
        if output.pbData.is_null() {
            return Err(Error);
        }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn windows_dpapi_roundtrip_never_stores_plaintext() {
        let protected = protect("fixture-credential-no-real-user-data").unwrap();
        assert!(protected.starts_with("limusic-dpapi:v1:"));
        assert!(!protected.contains("fixture-credential-no-real-user-data"));
        assert_eq!(unprotect(&protected).unwrap(), "fixture-credential-no-real-user-data");
    }

    #[test]
    fn invalid_or_unknown_ciphertext_is_never_treated_as_plaintext() {
        for value in
            ["limusic-dpapi:v1:not-base64!", "limusic-dpapi:v2:AAAA", "limusic-dpapi:v1:AAAA"]
        {
            assert!(unprotect(value).is_err());
        }
    }
}
