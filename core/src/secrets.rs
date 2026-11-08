//! API keys live only in the OS keychain (CLAUDE.md rule 7). Never in SQLite, config, logs, or the UI.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{CoreError, Result};

pub const SERVICE: &str = "dev.kovirlabs.tandem";

pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn set(&self, account: &str, value: &str) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
}

pub struct KeyringStore;

fn entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, account).map_err(|e| CoreError::Secret(e.to_string()))
}

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        match entry(account)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CoreError::Secret(e.to_string())),
        }
    }
    fn set(&self, account: &str, value: &str) -> Result<()> {
        entry(account)?
            .set_password(value)
            .map_err(|e| CoreError::Secret(e.to_string()))
    }
    fn delete(&self, account: &str) -> Result<()> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CoreError::Secret(e.to_string())),
        }
    }
}

/// In-memory store for tests and headless tools.
#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        Ok(self.0.lock().ok().and_then(|m| m.get(account).cloned()))
    }
    fn set(&self, account: &str, value: &str) -> Result<()> {
        if let Ok(mut m) = self.0.lock() {
            m.insert(account.to_string(), value.to_string());
        }
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<()> {
        if let Ok(mut m) = self.0.lock() {
            m.remove(account);
        }
        Ok(())
    }
}
