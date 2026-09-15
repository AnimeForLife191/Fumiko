//! Native credential vault integration and test abstractions.

use std::{fmt::Debug, sync::Arc};

#[cfg(test)]
use std::{collections::HashMap, sync::Mutex};

use common::APP_SERVICE_NAME;
use keyring::Entry;
use uuid::Uuid;

use crate::StorageError;

/// Production secret backend backed by the native operating system credential vault.
///
/// Uses Apple Keychain on macOS, Windows Credential Manager on Windows,
/// and Secret Service (D-Bus) / libsecret on Linux.
#[derive(Clone, Debug)]
pub struct KeyringBackend;

/// Abstract interface for secure secret storage backends.
pub trait TokenBackend: Send + Sync + Debug {
    /// Writes or overwrites a secret entry.
    fn set(&self, key: &str, value: &str) -> Result<(), StorageError>;

    /// Reads a secret entry by key.
    fn get(&self, key: &str) -> Result<Option<String>, StorageError>;

    /// Deletes a secret entry. Safe no-op if the key does not exist.
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

/// Secure token vault wrapping an underlying [`TokenBackend`].
///
/// Manages unified account secret storage (OAuth refresh tokens and IMAP app passwords)
/// keyed by account UUIDs.
#[derive(Clone, Debug)]
pub struct TokenStore {
    backend: Arc<dyn TokenBackend>,
}

impl TokenStore {
    /// Creates a new `TokenStore` with a custom backend implementation.
    pub fn new(backend: Arc<dyn TokenBackend>) -> Self {
        Self { backend }
    }

    /// Creates a `TokenStore` backed by the host operating system credential vault.
    pub fn with_keyring() -> Self {
        Self::new(Arc::new(KeyringBackend))
    }

    /// Creates an in-memory `TokenStore` for headless continuous integration testing.
    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::new(Arc::new(InMemoryBackend::default()))
    }

    /// Saves the primary authentication secret (OAuth refresh token or IMAP app password) for an account.
    ///
    /// # Errors
    /// Returns [`StorageError::Keyring`] if writing to the credential vault fails.
    pub fn save_account_secret(&self, account_id: Uuid, secret: &str) -> Result<(), StorageError> {
        self.backend.set(&account_id.to_string(), secret)
    }

    /// Retrieves the primary authentication secret for an account.
    ///
    /// # Errors
    /// Returns [`StorageError::Keyring`] if reading from the credential vault fails.
    pub fn get_account_secret(&self, account_id: Uuid) -> Result<Option<String>, StorageError> {
        self.backend.get(&account_id.to_string())
    }

    /// Deletes the primary authentication secret for an account.
    ///
    /// # Errors
    /// Returns [`StorageError::Keyring`] if deleting from the credential vault fails.
    pub fn delete_account_secret(&self, account_id: Uuid) -> Result<(), StorageError> {
        self.backend.delete(&account_id.to_string())
    }

    /// Writes a generic application secret to the vault by arbitrary key string.
    ///
    /// # Errors
    /// Returns [`StorageError::Keyring`] if writing to the vault fails.
    pub fn set_secret(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.backend.set(key, value)
    }

    /// Reads a generic application secret from the vault by arbitrary key string.
    ///
    /// # Errors
    /// Returns [`StorageError::Keyring`] if reading from the vault fails.
    pub fn get_secret(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.backend.get(key)
    }

    /// Deletes a generic application secret from the vault by arbitrary key string.
    ///
    /// # Errors
    /// Returns [`StorageError::Keyring`] if deleting from the vault fails.
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