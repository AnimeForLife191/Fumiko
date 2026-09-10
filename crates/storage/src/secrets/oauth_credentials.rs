use common::setting_keys;

use crate::{Storage, StorageError};

impl Storage {
    /// Saves custom Google OAuth credentials.
    ///
    /// Stores the client ID in SQLite settings and commits the client secret to the OS keyring.
    /// If the database write fails, the keyring secret is automatically rolled back.
    pub async fn save_user_google_credentials(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<(), StorageError> {
        self.token_store
            .set_secret(setting_keys::USER_GOOGLE_CLIENT_SECRET, client_secret)?;

        if let Err(e) = self
            .set_setting(setting_keys::USER_GOOGLE_CLIENT_ID, client_id)
            .await
        {
            let _ = self
                .token_store
                .delete_secret(setting_keys::USER_GOOGLE_CLIENT_SECRET);
            return Err(e);
        }
        Ok(())
    }

    /// Retrieves user-configured Google OAuth credentials if previously saved.
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
            .get_secret(setting_keys::USER_GOOGLE_CLIENT_SECRET)?;
        let Some(client_secret) = client_secret else {
            return Ok(None);
        };

        Ok(Some((client_id, client_secret)))
    }

    /// Deletes user-configured Google OAuth credentials from both Keyring and SQLite.
    pub async fn delete_user_google_credentials(&self) -> Result<(), StorageError> {
        self.token_store
            .delete_secret(setting_keys::USER_GOOGLE_CLIENT_SECRET)?;

        self.delete_setting(setting_keys::USER_GOOGLE_CLIENT_ID)
            .await?;

        Ok(())
    }

    /// Saves a user-configured Microsoft public Client ID in SQLite settings.
    pub async fn save_user_microsoft_client_id(&self, client_id: &str) -> Result<(), StorageError> {
        self.set_setting(setting_keys::USER_MICROSOFT_CLIENT_ID, client_id)
            .await?;
        Ok(())
    }

    /// Retrieves the user-configured Microsoft Client ID if present.
    pub async fn get_user_microsoft_client_id(&self) -> Result<Option<String>, StorageError> {
        self.get_setting(setting_keys::USER_MICROSOFT_CLIENT_ID)
            .await
    }

    /// Deletes the user-configured Microsoft Client ID from SQLite settings.
    pub async fn delete_user_microsoft_client_id(&self) -> Result<(), StorageError> {
        self.delete_setting(setting_keys::USER_MICROSOFT_CLIENT_ID)
            .await?;
        Ok(())
    }
}