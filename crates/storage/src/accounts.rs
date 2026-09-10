use super::models::LinkedAccount;
use super::{Storage, StorageError};
use common::Provider;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

impl Storage {
    /// Inserts a new linked email account record into SQLite.
    ///
    /// Generates a time-ordered UUIDv7 primary key for index locality.
    ///
    /// # Errors
    /// Returns [`StorageError::Conflict`] if the email address is already linked.
    pub async fn add_account(
        &self,
        provider: Provider,
        email_address: &str,
        display_name: Option<&str>,
    ) -> Result<Uuid, StorageError> {
        let account_id = Uuid::now_v7();
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let result = sqlx::query(
            "INSERT INTO linked_accounts (id, provider, email_address, display_name, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(account_id.to_string())
        .bind(provider.as_str())
        .bind(email_address)
        .bind(display_name)
        .bind(created_at)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(account_id),
            Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
                Err(StorageError::Conflict)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Checks whether an email address is already registered.
    pub async fn account_exists(&self, email_address: &str) -> Result<bool, StorageError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT id FROM linked_accounts WHERE email_address = ?")
                .bind(email_address)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    /// Fetches an account record by its unique ID.
    pub async fn get_account(
        &self,
        account_id: Uuid,
    ) -> Result<Option<LinkedAccount>, StorageError> {
        let row = sqlx::query_as::<_, LinkedAccount>(
            "SELECT id, provider, email_address, display_name, sync_cursor, last_synced_at, last_sync_error, created_at
             FROM linked_accounts WHERE id = ?",
        )
        .bind(account_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Deletes an account and cascades deletion to all associated emails and classifications.
    ///
    /// # Invariant
    /// Deletes the refresh token from the OS keyring before deleting the SQLite row.
    pub async fn delete_account(&self, account_id: Uuid) -> Result<(), StorageError> {
        // Invariant: Remove the refresh token from the OS keyring before deleting the DB row.
        // If keyring deletion fails, the account row remains so the user can retry unlinking.
        self.token_store.delete_refresh_token(account_id)?;

        // Foreign keys with ON DELETE CASCADE will automatically remove associated emails and classifications.
        sqlx::query("DELETE FROM linked_accounts WHERE id = ?")
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Returns all linked accounts ordered by creation date.
    pub async fn list_accounts(&self) -> Result<Vec<LinkedAccount>, StorageError> {
        let rows = sqlx::query_as::<_, LinkedAccount>(
            "SELECT id, provider, email_address, display_name, sync_cursor, last_synced_at, last_sync_error, created_at
             FROM linked_accounts ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Updates the custom display name for an account.
    pub async fn update_display_name(
        &self,
        account_id: Uuid,
        display_name: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE linked_accounts SET display_name = ? WHERE id = ?")
            .bind(display_name)
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Clears the custom display name for an account, reverting to default formatting.
    pub async fn clear_display_name(&self, account_id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE linked_accounts SET display_name = NULL WHERE id = ?")
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Performs a complete application data wipe.
    ///
    /// Clears all secrets from the OS Keyring, truncates all SQLite tables inside an atomic
    /// transaction, checkpoints the WAL log, and executes a database `VACUUM` to reclaim disk space.
    pub async fn wipe_all_data(&self) -> Result<(), StorageError> {
        let accounts = self.list_accounts().await?;
        for account in accounts {
            self.token_store.delete_refresh_token(account.id)?;
        }

        self.delete_user_google_credentials().await?;
        self.delete_user_microsoft_client_id().await?;

        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM linked_accounts")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM watch_criteria")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM settings")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        // Checkpoint WAL and reclaim freed disk space.
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("VACUUM").execute(&self.pool).await;

        Ok(())
    }
}