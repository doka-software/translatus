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

/// How long to wait for the OS keychain before deciding it is not going to
/// answer.
///
/// On macOS a read can raise an authorisation dialog — routinely, after a
/// rebuild changes the binary's signature. The call then blocks until somebody
/// clicks, and if the screen is asleep or the dialog is behind another window,
/// "translatus is thinking" and "translatus is waiting for a click you cannot
/// see" look identical and last forever. A missing key is a recoverable state
/// (env var, `--base-url`, Settings); an unbounded hang is not.
const KEYCHAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub fn get_key(provider: &str) -> Result<Option<String>> {
    let provider = provider.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let got = entry(&provider).and_then(|e| match e.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CoreError::Other(format!("keyring get: {e}"))),
        });
        // The receiver is gone on timeout; the thread stays parked on the
        // dialog until the user answers it, which is the OS's business.
        let _ = tx.send(got);
    });
    match rx.recv_timeout(KEYCHAIN_TIMEOUT) {
        Ok(r) => r,
        Err(_) => Err(CoreError::Other(format!(
            "keyring get: no answer in {}s — if macOS is asking permission to \
             read the keychain, that dialog may be behind another window or on \
             a sleeping display. Allow it and retry, or pass the key through \
             the provider environment variable instead.",
            KEYCHAIN_TIMEOUT.as_secs()
        ))),
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
