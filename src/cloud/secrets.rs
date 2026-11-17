//! Account secrets (passwords, secret keys) in the system keyring: the
//! Secret Service on Linux (KWallet, GNOME Keyring, KeePassXC), the
//! Credential Manager on Windows. One entry per account, service
//! "PaperView", user = the account id.
//!
//! Keyring calls can block (an unlock prompt, a slow D-Bus), so the app only
//! makes them from worker threads. A copy is remembered in memory for the
//! session. When there is no keyring, the app keeps the secret in its
//! settings file instead ([`Account::plain_secret`]).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use super::Account;

const SERVICE: &str = "PaperView";

static CACHE: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

/// Keeps a secret for this session (call before the keyring has it).
pub fn remember(account: &str, secret: &str) {
    if let Ok(mut c) = CACHE.lock() {
        c.insert(account.to_string(), secret.to_string());
    }
}

/// The account's secret: settings-file copy, this session's copy, or the
/// keyring — in that order.
pub fn get(acct: &Account) -> Result<String, String> {
    if let Some(s) = acct.plain_secret.as_ref().filter(|s| !s.is_empty()) {
        return Ok(s.clone());
    }
    if let Some(s) = CACHE.lock().ok().and_then(|c| c.get(&acct.id).cloned()) {
        return Ok(s);
    }
    #[cfg(debug_assertions)]
    if std::env::var_os("PAPERVIEW_NO_KEYRING").is_some() {
        return Err("No saved password (keyring disabled for tests)".into());
    }
    let entry = keyring::Entry::new(SERVICE, &acct.id).map_err(|e| unavailable(&e))?;
    match entry.get_password() {
        Ok(s) => {
            remember(&acct.id, &s);
            Ok(s)
        }
        Err(keyring::Error::NoEntry) => Err(format!(
            "No saved password for “{}” — enter it again in Settings",
            acct.name
        )),
        Err(e) => Err(unavailable(&e)),
    }
}

/// Saves a secret in the keyring. Blocking: call from a worker thread.
pub fn store(account: &str, secret: &str) -> Result<(), String> {
    #[cfg(debug_assertions)]
    if std::env::var_os("PAPERVIEW_NO_KEYRING").is_some() {
        return Err("keyring disabled for tests".into());
    }
    keyring::Entry::new(SERVICE, account)
        .and_then(|e| e.set_password(secret))
        .map_err(|e| unavailable(&e))
}

/// Forgets an account's secret everywhere. Blocking: call from a worker.
pub fn forget(account: &str) {
    if let Ok(mut c) = CACHE.lock() {
        c.remove(account);
    }
    #[cfg(debug_assertions)]
    if std::env::var_os("PAPERVIEW_NO_KEYRING").is_some() {
        return;
    }
    if let Ok(e) = keyring::Entry::new(SERVICE, account) {
        let _ = e.delete_credential();
    }
}

fn unavailable(e: &keyring::Error) -> String {
    format!("The system keyring isn't available ({e})")
}
