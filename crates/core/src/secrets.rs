//! API key storage in the OS keychain (macOS Keychain / Windows Credential
//! Manager / Linux Secret Service). The key never lands in a plaintext config
//! file or the frontend — it is written here and read back only by the engine
//! when it makes the LLM call.

use crate::error::{CoreError, Result};
use keyring::Entry;

// Stable keychain namespace. Changing this string orphans every already-saved
// API key, so treat it as frozen from the first public release onward.
const SERVICE: &str = "translatus";

fn entry(provider: &str) -> Result<Entry> {
    Entry::new(SERVICE, provider).map_err(|e| CoreError::Other(format!("keyring: {e}")))
}

pub fn set_key(provider: &str, key: &str) -> Result<()> {
    entry(provider)?
        .set_password(key)
        .map_err(|e| CoreError::Other(format!("keyring set: {e}")))
}

pub fn get_key(provider: &str) -> Result<Option<String>> {
    match entry(provider)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(CoreError::Other(format!("keyring get: {e}"))),
    }
}

pub fn delete_key(provider: &str) -> Result<()> {
    match entry(provider)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(CoreError::Other(format!("keyring delete: {e}"))),
    }
}

/// A masked hint for the UI: last 4 chars, never the full key.
pub fn key_hint(provider: &str) -> Result<Option<String>> {
    Ok(get_key(provider)?.map(|k| {
        let tail: String = k
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("••••{tail}")
    }))
}
