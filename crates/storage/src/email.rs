use crate::validate_limit;
use uuid::Uuid;

use super::models::{EmailRow, MailboxStats};
use super::unix_timestamp;
use super::{Storage, StorageError};

const SECONDS_PER_DAY: i64 = 86_400;
const MAX_RETENTION_DAYS: i64 = 3650;

impl Storage {
    /// Inserts or updates synchronized email metadata.
    ///
    /// Uses `ON CONFLICT DO UPDATE` to safely update read flags and fill missing snippets
    /// without resetting existing subject headers or local view status.
    pub async fn save_email(
        &self,
        account_id: Uuid,
        provider_message_id: &str,
        subject: Option<&str>,
        sender: Option<&str>,
        received_at: i64,
        is_read: bool,
        snippet: Option<&str>,
    ) -> Result<Uuid, StorageError> {
        let id = Uuid::now_v7();
        let row: (String,) = sqlx::query_as(
            "INSERT INTO emails
                (id, account_id, provider_message_id, subject, sender, received_at, is_read, snippet, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(account_id, provider_message_id)
            DO UPDATE SET
                is_read = excluded.is_read,
                subject = COALESCE(excluded.subject, emails.subject),
                snippet = COALESCE(excluded.snippet, emails.snippet)
            RETURNING id",
        )
        .bind(id.to_string())
        .bind(account_id.to_string())
        .bind(provider_message_id)
        .bind(subject)
        .bind(sender)
        .bind(received_at)
        .bind(is_read)
        .bind(snippet)
        .bind(unix_timestamp())
        .fetch_one(&self.pool)
        .await?;

        let email_id = Uuid::parse_str(&row.0)?;
        Ok(email_id)
    }

    /// Fetches a single email record by its internal UUID.
    pub async fn get_email(&self, email_id: Uuid) -> Result<Option<EmailRow>, StorageError> {
        let row = sqlx::query_as::<_, EmailRow>(
            "SELECT id, account_id, provider_message_id, subject, sender, received_at, is_read, app_has_viewed, is_trashed, trashed_at, snippet
            FROM emails WHERE id = ?",
        )
        .bind(email_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Lists active inbox messages for an account, sorted from newest to oldest.
    pub async fn list_inbox(
        &self,
        account_id: Uuid,
        limit: i64,
    ) -> Result<Vec<EmailRow>, StorageError> {
        validate_limit(limit)?;
        let rows = sqlx::query_as::<_, EmailRow>(
            "SELECT id, account_id, provider_message_id, subject, sender, received_at, is_read, app_has_viewed, is_trashed, trashed_at, snippet
            FROM emails
            WHERE account_id = ? AND is_trashed = 0
            ORDER BY received_at DESC
            LIMIT ?",
        )
        .bind(account_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Lists active inbox messages across all accounts in a unified chronological view.
    pub async fn list_inbox_all(&self, limit: i64) -> Result<Vec<EmailRow>, StorageError> {
        validate_limit(limit)?;
        let rows = sqlx::query_as::<_, EmailRow>(
            "SELECT id, account_id, provider_message_id, subject, sender, received_at, is_read, app_has_viewed, is_trashed, trashed_at, snippet
            FROM emails WHERE is_trashed = 0 ORDER BY received_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Permanently deletes an email record by internal ID.
    pub async fn delete_email(&self, email_id: Uuid) -> Result<(), StorageError> {
        let result = sqlx::query("DELETE FROM emails WHERE id = ?")
            .bind(email_id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }

    /// Permanently deletes an email identified by its provider message ID (e.g., during purge syncs).
    pub async fn delete_by_provider_message_id(
        &self,
        account_id: Uuid,
        provider_message_id: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM emails WHERE account_id = ? AND provider_message_id = ?")
            .bind(account_id.to_string())
            .bind(provider_message_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Prunes old emails exceeding the maximum retention capacity while preserving AI-classified findings.
    pub async fn prune_old_emails(
        &self,
        account_id: Uuid,
        max_recent_emails: i64,
    ) -> Result<(), StorageError> {
        validate_limit(max_recent_emails)?;
        sqlx::query(
            "DELETE FROM emails
             WHERE account_id = ?
             AND id NOT IN (
                 SELECT id FROM emails
                 WHERE account_id = ?
                 ORDER BY received_at DESC
                 LIMIT ?
             )
             AND id NOT IN (
                 SELECT email_id FROM classifications
             )",
        )
        .bind(account_id.to_string())
        .bind(account_id.to_string())
        .bind(max_recent_emails)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Marks an email as trashed during synchronization. Safe no-op if message does not exist locally.
    pub async fn mark_trashed(
        &self,
        account_id: Uuid,
        provider_message_id: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE emails SET is_trashed = 1, trashed_at = ? WHERE account_id = ? AND provider_message_id = ?",
        )
        .bind(unix_timestamp())
        .bind(account_id.to_string())
        .bind(provider_message_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Restores a trashed email during synchronization.
    pub async fn mark_untrashed(
        &self,
        account_id: Uuid,
        provider_message_id: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE emails SET is_trashed = 0, trashed_at = NULL WHERE account_id = ? AND provider_message_id = ?")
            .bind(account_id.to_string())
            .bind(provider_message_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Updates read status for an email.
    pub async fn set_email_read(
        &self,
        account_id: Uuid,
        provider_message_id: &str,
        is_read: bool,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE emails SET is_read = ? WHERE account_id = ? AND provider_message_id = ?",
        )
        .bind(is_read)
        .bind(account_id.to_string())
        .bind(provider_message_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Moves an email to trash from the local user interface.
    pub async fn trash_email(&self, email_id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE emails SET is_trashed = 1, trashed_at = ? WHERE id = ?")
            .bind(unix_timestamp())
            .bind(email_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Restores an email from trash.
    pub async fn restore_email(&self, email_id: Uuid) -> Result<(), StorageError> {
        let result =
            sqlx::query("UPDATE emails SET is_trashed = 0, trashed_at = NULL WHERE id = ?")
                .bind(email_id.to_string())
                .execute(&self.pool)
                .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }

    /// Sets the local `app_has_viewed` flag when the user opens an email.
    pub async fn mark_email_viewed(&self, email_id: Uuid) -> Result<(), StorageError> {
        let result = sqlx::query("UPDATE emails SET app_has_viewed = 1 WHERE id = ?")
            .bind(email_id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }

    /// Lists trashed messages sorted by trash timestamp.
    pub async fn list_trash(&self, limit: i64) -> Result<Vec<EmailRow>, StorageError> {
        validate_limit(limit)?;
        let rows = sqlx::query_as::<_, EmailRow>(
            "SELECT id, account_id, provider_message_id, subject, sender, received_at, is_read, app_has_viewed, is_trashed, trashed_at, snippet
            FROM emails
            WHERE is_trashed = 1
            ORDER BY trashed_at DESC
            LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Permanently deletes trashed emails older than the configured retention threshold.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidInput`] if `retention_days` is negative or exceeds 3650 days.
    pub async fn purge_expired_trash(&self, retention_days: i64) -> Result<u64, StorageError> {
        if !(0..=MAX_RETENTION_DAYS).contains(&retention_days) {
            return Err(StorageError::InvalidInput(
                "retention_days must be between 0 and 3650".into(),
            ));
        }

        let retention_seconds = retention_days
            .checked_mul(SECONDS_PER_DAY)
            .ok_or_else(|| StorageError::InvalidInput("retention period is too large".into()))?;

        let cutoff = unix_timestamp()
            .checked_sub(retention_seconds)
            .ok_or_else(|| StorageError::InvalidInput("retention period is too large".into()))?;

        let result = sqlx::query(
            "DELETE FROM emails
            WHERE is_trashed = 1
            AND trashed_at <= ?",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Returns accurate total, unread, and findings counts for an account (or across all accounts).
    pub async fn get_mailbox_stats(
        &self,
        account_id: Option<Uuid>,
    ) -> Result<MailboxStats, StorageError> {
        let account_str = account_id.map(|id| id.to_string());

        let row: (i64, i64, i64) = sqlx::query_as(
            "SELECT
                (SELECT COUNT(*) FROM emails 
                 WHERE is_trashed = 0 AND is_read = 0 AND app_has_viewed = 0 
                 AND (? IS NULL OR account_id = ?)) AS unread_count,
                 
                (SELECT COUNT(DISTINCT c.email_id) FROM classifications c 
                 JOIN emails m ON m.id = c.email_id 
                 WHERE m.is_trashed = 0 
                 AND (? IS NULL OR m.account_id = ?)) AS findings_count,
                 
                (SELECT COUNT(*) FROM emails 
                 WHERE is_trashed = 0 
                 AND (? IS NULL OR account_id = ?)) AS total_count"
        )
        .bind(&account_str)
        .bind(&account_str)
        .bind(&account_str)
        .bind(&account_str)
        .bind(&account_str)
        .bind(&account_str)
        .fetch_one(&self.pool)
        .await?;

        Ok(MailboxStats {
            unread_count: row.0,
            findings_count: row.1,
            total_count: row.2,
        })
    }
}