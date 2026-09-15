//! Storage operations for custom Google and Microsoft developer credentials.

use common::{keyring_keys, setting_keys};

use crate::{Storage, StorageError};

impl Storage {
    /// Saves custom Google OAuth client credentials.
    ///
    /// Heterogeneous Rollback Pattern:
    /// SQLite and the OS Keyring cannot participate in a single unified ACID transaction.
    /// To ensure consistency, the confidential client secret is committed to the OS Keyring first.
    /// If writing the public client ID to SQLite settings fails, the client secret is purged
    /// immediately from the Keyring to prevent orphaned credentials in the host vault.
    ///
    /// # Errors
    /// Returns [`StorageError::Keyring`] if writing to the OS keyring fails,
    /// or [`StorageError::Db`] if writing the client ID to SQLite fails.
    pub async fn save_user_google_credentials(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<(), StorageError> {
        self.token_store
            .set_secret(keyring_keys::USER_GOOGLE_CLIENT_SECRET, client_secret)?;

        if let Err(e) = self
            .set_setting(setting_keys::USER_GOOGLE_CLIENT_ID, client_id)
            .await
        {
            let _ = self
                .token_store
                .delete_secret(keyring_keys::USER_GOOGLE_CLIENT_SECRET);
            return Err(e);
        }
        Ok(())
    }

    /// Retrieves user-configured Google OAuth credentials if previously saved.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if reading SQLite settings fails,
    /// or [`StorageError::Keyring`] if reading from the OS keyring fails.
    pub async fn get_user_google_credentials(
        &self,
    ) -> Result<Option<(String, String)>, StorageError> {
        let client_id = self
            .get_setting(setting_keys::USER_GOOGLE_CLIENT_ID)
            .await?;
        let Some(client_id) = client_id else {
            return Ok(None);
        };

        let client_secret = self
            .token_store
            .get_secret(keyring_keys::USER_GOOGLE_CLIENT_SECRET)?;
        let Some(client_secret) = client_secret else {
            return Ok(None);
        };

        Ok(Some((client_id, client_secret)))
    }

    /// Deletes user-configured Google OAuth credentials from both the OS Keyring and SQLite settings.
    ///
    /// # Errors
    /// Returns [`StorageError::Keyring`] if deleting from the OS keyring fails,
    /// or [`StorageError::Db`] if deleting from SQLite fails.
    pub async fn delete_user_google_credentials(&self) -> Result<(), StorageError> {
        self.token_store
            .delete_secret(keyring_keys::USER_GOOGLE_CLIENT_SECRET)?;

        self.delete_setting(setting_keys::USER_GOOGLE_CLIENT_ID)
            .await?;

        Ok(())
    }

    /// Saves a user-configured Microsoft public Client ID in SQLite settings.
    ///
    /// Microsoft public desktop client flows use PKCE and do not require or use a client secret.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the database write fails.
    pub async fn save_user_microsoft_client_id(&self, client_id: &str) -> Result<(), StorageError> {
        self.set_setting(setting_keys::USER_MICROSOFT_CLIENT_ID, client_id)
            .await?;
        Ok(())
    }

    /// Retrieves the user-configured Microsoft Client ID if present.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the query fails.
    pub async fn get_user_microsoft_client_id(&self) -> Result<Option<String>, StorageError> {
        self.get_setting(setting_keys::USER_MICROSOFT_CLIENT_ID)
            .await
    }

    /// Deletes the user-configured Microsoft Client ID from SQLite settings.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the database deletion fails.
    pub async fn delete_user_microsoft_client_id(&self) -> Result<(), StorageError> {
        self.delete_setting(setting_keys::USER_MICROSOFT_CLIENT_ID)
            .await?;
        Ok(())
    }
}