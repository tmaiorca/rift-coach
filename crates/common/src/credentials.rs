//! Secrets at rest live in Windows Credential Manager (generic credentials
//! named `RiftCoach:<name>`), never in the database, settings, or logs.
//! Values go in and come out only as [`Secret`].

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use keyring_core::api::CredentialStore;
use keyring_core::{Entry, Error};

use crate::Secret;

const SERVICE: &str = "RiftCoach";

/// Credential name for the Riot API key.
pub const RIOT_API_KEY: &str = "riot_api_key";
/// Credential name for the MCP server's access token.
pub const MCP_TOKEN: &str = "mcp_token";

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("Windows Credential Manager is unavailable: {0}")]
    Unavailable(String),
    #[error("credential store error: {0}")]
    Store(String),
}

static STORE: OnceLock<Arc<CredentialStore>> = OnceLock::new();

/// Use a different store (tests). Must run before the first credential call.
pub fn init_store(store: Arc<CredentialStore>) {
    let _ = STORE.set(store);
}

fn store() -> Result<&'static Arc<CredentialStore>, CredentialError> {
    if let Some(s) = STORE.get() {
        return Ok(s);
    }
    let s = platform_store()?;
    Ok(STORE.get_or_init(|| s))
}

#[cfg(windows)]
fn platform_store() -> Result<Arc<CredentialStore>, CredentialError> {
    let s: Arc<CredentialStore> =
        windows_native_keyring_store::Store::new().map_err(|e| CredentialError::Unavailable(e.to_string()))?;
    Ok(s)
}

#[cfg(not(windows))]
fn platform_store() -> Result<Arc<CredentialStore>, CredentialError> {
    Err(CredentialError::Unavailable("only Windows Credential Manager is supported".into()))
}

fn entry(name: &str) -> Result<Entry, CredentialError> {
    // Local persistence: the secret stays on this PC instead of roaming
    // with a domain profile.
    let mods = HashMap::from([("persistence", "Local")]);
    let store = store()?;
    match store.build(SERVICE, name, Some(&mods)) {
        // Stores without modifier support (the test mock) get a plain entry.
        Err(Error::NotSupportedByStore(_)) => store.build(SERVICE, name, None),
        other => other,
    }
    .map_err(|e| CredentialError::Store(describe(&e)))
}

/// Error text without any attached secret bytes.
fn describe(e: &Error) -> String {
    match e {
        Error::BadEncoding(_) => "stored value is not valid UTF-8".into(),
        Error::BadDataFormat(_, _) => "stored value has an unexpected format".into(),
        other => other.to_string(),
    }
}

pub fn store_secret(name: &str, secret: &Secret) -> Result<(), CredentialError> {
    entry(name)?.set_password(secret.expose()).map_err(|e| CredentialError::Store(describe(&e)))
}

pub fn load_secret(name: &str) -> Result<Option<Secret>, CredentialError> {
    match entry(name)?.get_password() {
        Ok(v) if v.trim().is_empty() => Ok(None),
        Ok(v) => Ok(Some(Secret::new(v.trim()))),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(CredentialError::Store(describe(&e))),
    }
}

pub fn has_secret(name: &str) -> bool {
    matches!(load_secret(name), Ok(Some(_)))
}

pub fn delete_secret(name: &str) -> Result<(), CredentialError> {
    match entry(name)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(CredentialError::Store(describe(&e))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_store() {
        init_store(keyring_core::mock::Store::new().expect("mock store"));
        assert!(load_secret("t1").expect("load").is_none());
        store_secret("t1", &Secret::new("sk-test-123")).expect("store");
        assert_eq!(load_secret("t1").expect("load").map(|s| s.expose().to_owned()).as_deref(), Some("sk-test-123"));
        assert!(has_secret("t1"));
        delete_secret("t1").expect("delete");
        delete_secret("t1").expect("deleting twice is fine");
        assert!(!has_secret("t1"));
    }
}
