use std::{fmt::Debug, sync::Arc};

#[cfg(test)]
use std::{collections::HashMap, sync::Mutex};

use common::APP_SERVICE_NAME;
use keyring::Entry;
use uuid::Uuid;

use crate::StorageError;

/// Production backend backed by the operating system credential vault.
#[derive(Clone, Debug)]
pub struct KeyringBackend;

/// Abstract interface for secure secret storage backends.
pub trait TokenBackend: Send + Sync + Debug {
    fn set(&self, key: &str, value: &str) -> Result<(), StorageError>;

    fn get(&self, key: &str) -> Result<Option<String>, StorageError>;

    fn delete(&self, key: &str) -> Result<(), StorageError>;
}

impl TokenBackend for KeyringBackend {
    fn set(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let entry = Entry::new(APP_SERVICE_NAME, key)?;
        entry.set_password(value)?;

        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        let entry = Entry::new(APP_SERVICE_NAME, key)?;

        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        let entry = Entry::new(APP_SERVICE_NAME, key)?;

        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
#[derive(Default, Debug)]
pub struct InMemoryBackend {
    entries: Mutex<HashMap<String, String>>,
}

/// Secure token vault wrapping an underlying `TokenBackend`.
#[derive(Clone, Debug)]
pub struct TokenStore {
    backend: Arc<dyn TokenBackend>,
}

impl TokenStore {
    /// Creates a new `TokenStore` with a custom backend implementation.
    pub fn new(backend: Arc<dyn TokenBackend>) -> Self {
        Self { backend }
    }

    /// Creates a `TokenStore` backed by the native OS keyring.
    pub fn with_keyring() -> Self {
        Self::new(Arc::new(KeyringBackend))
    }

    /// Creates an in-memory `TokenStore` for headless continuous integration tests.
    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::new(Arc::new(InMemoryBackend::default()))
    }

    /// Stores an account's refresh token keyed by account UUID.
    pub fn save_refresh_token(
        &self,
        account_id: Uuid,
        refresh_token: &str,
    ) -> Result<(), StorageError> {
        self.backend.set(&account_id.to_string(), refresh_token)
    }

    /// Retrieves an account's refresh token from the secure vault.
    pub fn get_refresh_token(&self, account_id: Uuid) -> Result<Option<String>, StorageError> {
        self.backend.get(&account_id.to_string())
    }

    /// Removes an account's refresh token from the secure vault.
    pub fn delete_refresh_token(&self, account_id: Uuid) -> Result<(), StorageError> {
        self.backend.delete(&account_id.to_string())
    }

    /// Stores an arbitrary secret string by key.
    pub fn set_secret(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.backend.set(key, value)
    }

    /// Retrieves an arbitrary secret string by key.
    pub fn get_secret(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.backend.get(key)
    }

    /// Deletes an arbitrary secret string by key.
    pub fn delete_secret(&self, key: &str) -> Result<(), StorageError> {
        self.backend.delete(key)
    }
}

#[cfg(test)]
impl TokenBackend for InMemoryBackend {
    fn set(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.entries
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());

        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        Ok(self.entries.lock().unwrap().get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.entries.lock().unwrap().remove(key);

        Ok(())
    }
}