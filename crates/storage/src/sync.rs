use super::unix_timestamp;
use super::{Storage, StorageError};
use uuid::Uuid;

impl Storage {
    /// Records a synchronization failure message on the account row.
    pub async fn set_sync_error(&self, account_id: Uuid, error: &str) -> Result<(), StorageError> {
        sqlx::query("UPDATE linked_accounts SET last_sync_error = ? WHERE id = ?")
            .bind(error)
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Atomically advances the sync cursor, records the sync timestamp, and clears any previous error.
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
}