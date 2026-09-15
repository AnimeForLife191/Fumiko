//! Database operations for tracking mailbox synchronization state and cursors.

use super::unix_timestamp;
use super::{Storage, StorageError};
use uuid::Uuid;

impl Storage {
    /// Records a diagnostic synchronization failure message on the account record.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the update fails.
    pub async fn set_sync_error(&self, account_id: Uuid, error: &str) -> Result<(), StorageError> {
        sqlx::query("UPDATE linked_accounts SET last_sync_error = ? WHERE id = ?")
            .bind(error)
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Advances the synchronization cursor, records the sync timestamp, and clears any previous error.
    ///
    /// Invariant: Called only after an entire discovered batch of messages has been successfully
    /// hydrated and persisted. If network issues cause a partial fetch failure, the cursor is
    /// not advanced, allowing the next sync cycle to re-discover missing messages.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the update fails.
    pub async fn update_sync_cursor(
        &self,
        account_id: Uuid,
        sync_cursor: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE linked_accounts 
             SET sync_cursor = ?, last_synced_at = ?, last_sync_error = NULL 
             WHERE id = ?",
        )
        .bind(sync_cursor)
        .bind(unix_timestamp())
        .bind(account_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Clears the synchronization cursor, forcing the next sync cycle to establish a fresh baseline.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the update fails.
    pub async fn clear_sync_cursor(&self, account_id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE linked_accounts SET sync_cursor = NULL WHERE id = ?")
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}