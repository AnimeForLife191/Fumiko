//! Database mutations and queries for managing linked email accounts.

use super::models::LinkedAccount;
use super::{Storage, StorageError, unix_timestamp};
use common::Provider;
use uuid::Uuid;

impl Storage {
    /// Inserts a new linked OAuth email account record.
    ///
    /// Primary keys use time-ordered UUIDv7 to ensure sequential write locality
    /// and prevent B-tree index fragmentation in SQLite.
    ///
    /// # Errors
    /// Returns [`StorageError::Conflict`] if the email address is already linked,
    /// or [`StorageError::Db`] if the database query fails.
    pub async fn add_account(
        &self,
        provider: Provider,
        email_address: &str,
        display_name: Option<&str>,
    ) -> Result<Uuid, StorageError> {
        let account_id = Uuid::now_v7();
        let created_at = unix_timestamp();

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

    /// Inserts a new linked IMAP email account record with connection parameters.
    ///
    /// The port is cast to `i64` because SQLite integer columns are signed 64-bit values.
    ///
    /// # Errors
    /// Returns [`StorageError::Conflict`] if the email address is already linked,
    /// or [`StorageError::Db`] if the database query fails.
    pub async fn add_imap_account(
        &self,
        provider: Provider,
        email_address: &str,
        display_name: Option<&str>,
        imap_host: &str,
        imap_port: u16,
    ) -> Result<Uuid, StorageError> {
        let account_id = Uuid::now_v7();
        let created_at = unix_timestamp();

        let result = sqlx::query(
            "INSERT INTO linked_accounts (
                id, provider, email_address, display_name, created_at, imap_host, imap_port
            ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(account_id.to_string())
        .bind(provider.as_str())
        .bind(email_address)
        .bind(display_name)
        .bind(created_at)
        .bind(imap_host)
        .bind(imap_port as i64)
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

    /// Checks whether an email address is already registered in local storage.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the query fails.
    pub async fn account_exists(&self, email_address: &str) -> Result<bool, StorageError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT id FROM linked_accounts WHERE email_address = ?")
                .bind(email_address)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    /// Fetches an account record by its unique account ID.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the query fails.
    pub async fn get_account(
        &self,
        account_id: Uuid,
    ) -> Result<Option<LinkedAccount>, StorageError> {
        let row = sqlx::query_as::<_, LinkedAccount>(
            "SELECT id, provider, email_address, display_name, sync_cursor, 
                    last_synced_at, last_sync_error, created_at, imap_host, imap_port
             FROM linked_accounts 
             WHERE id = ?",
        )
        .bind(account_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Deletes an account and cascades deletion across all associated messages and classifications.
    ///
    /// Invariant: Removes the account secret from the OS Keyring before deleting the SQLite row.
    /// If Keyring deletion fails, the database row is left intact so the user can retry unlinking,
    /// preventing orphaned credentials in the host vault.
    ///
    /// # Errors
    /// Returns [`StorageError::Keyring`] if secret deletion fails,
    /// [`StorageError::NotFound`] if the account does not exist,
    /// or [`StorageError::Db`] if the database deletion fails.
    pub async fn delete_account(&self, account_id: Uuid) -> Result<(), StorageError> {
        self.token_store.delete_account_secret(account_id)?;

        let result = sqlx::query("DELETE FROM linked_accounts WHERE id = ?")
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }

    /// Returns all linked accounts ordered chronologically by creation date.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the query fails.
    pub async fn list_accounts(&self) -> Result<Vec<LinkedAccount>, StorageError> {
        let rows = sqlx::query_as::<_, LinkedAccount>(
            "SELECT id, provider, email_address, display_name, sync_cursor, 
                    last_synced_at, last_sync_error, created_at, imap_host, imap_port
             FROM linked_accounts 
             ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Updates the custom display name for an account.
    ///
    /// # Errors
    /// Returns [`StorageError::NotFound`] if the account ID does not exist,
    /// or [`StorageError::Db`] if the update fails.
    pub async fn update_display_name(
        &self,
        account_id: Uuid,
        display_name: &str,
    ) -> Result<(), StorageError> {
        let result = sqlx::query("UPDATE linked_accounts SET display_name = ? WHERE id = ?")
            .bind(display_name)
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }

    /// Clears the custom display name for an account, reverting to its primary email address in the UI.
    ///
    /// # Errors
    /// Returns [`StorageError::NotFound`] if the account ID does not exist,
    /// or [`StorageError::Db`] if the update fails.
    pub async fn clear_display_name(&self, account_id: Uuid) -> Result<(), StorageError> {
        let result = sqlx::query("UPDATE linked_accounts SET display_name = NULL WHERE id = ?")
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }

    /// Permanently wipes all accounts, credentials, criteria, emails, and application settings.
    ///
    /// Purges all OS keyring secrets across all linked accounts, removes custom developer keys,
    /// truncates database tables inside an explicit transaction, checkpoints the WAL journal,
    /// and runs an SQLite VACUUM to return reclaimed disk space to the host operating system.
    ///
    /// # Errors
    /// Returns [`StorageError::Keyring`] if credential removal fails,
    /// or [`StorageError::Db`] if the database truncation transaction fails.
    pub async fn wipe_all_data(&self) -> Result<(), StorageError> {
        // 1. Purge all account secrets (OAuth refresh tokens and IMAP passwords) from OS Keyring
        let accounts = self.list_accounts().await?;
        for account in accounts {
            self.token_store.delete_account_secret(account.id)?;
        }

        // 2. Remove user-configured Google and Microsoft developer credentials
        self.delete_user_google_credentials().await?;
        self.delete_user_microsoft_client_id().await?;

        // 3. Atomically truncate relational tables (foreign keys cascade to emails and classifications)
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

        // 4. In WAL mode, VACUUM only defragments the main database file. Executing
        // PRAGMA wal_checkpoint(TRUNCATE) first flushes and truncates the -wal log file
        // back to zero bytes so physical disk space is fully reclaimed.
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("VACUUM").execute(&self.pool).await;

        Ok(())
    }
}