//! Database queries and lifecycle operations for cached email metadata.

use crate::validate_limit;
use uuid::Uuid;

use super::models::{EmailRow, MailboxStats};
use super::unix_timestamp;
use super::{Storage, StorageError};

const SECONDS_PER_DAY: i64 = 86_400;
const MAX_RETENTION_DAYS: i64 = 3650;

impl Storage {
    /// Inserts or updates cached email metadata.
    ///
    /// Idempotency & Partial Updates:
    /// - Uses `ON CONFLICT(account_id, provider_message_id) DO UPDATE` so repeated sync passes
    ///   update server read flags cleanly without duplicate row errors.
    /// - Uses `COALESCE` to preserve previously hydrated subjects or snippets if incoming fields are null.
    /// - Deliberately omits `app_has_viewed` from the update list so background provider sync passes
    ///   never overwrite whether the user opened the message in the local UI.
    /// - Uses SQLite 3.35+ `RETURNING id` to retrieve the stable UUID in a single database round-trip.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the upsert query fails.
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
                sender = COALESCE(excluded.sender, emails.sender),
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

    /// Fetches an individual email metadata row by its internal UUID.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the query fails.
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

    /// Lists active, non-trashed messages for an individual linked account.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidInput`] if `limit` is invalid,
    /// or [`StorageError::Db`] if the query fails.
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

    /// Lists active, non-trashed messages across all linked accounts.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidInput`] if `limit` is invalid,
    /// or [`StorageError::Db`] if the query fails.
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

    /// Permanently deletes an email record by its internal UUID.
    ///
    /// # Errors
    /// Returns [`StorageError::NotFound`] if the email does not exist,
    /// or [`StorageError::Db`] if the deletion query fails.
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

    /// Permanently deletes an email record by its upstream provider message identifier.
    ///
    /// Safe no-op if the message does not exist locally (handles out-of-order deletion events).
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the deletion query fails.
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

    /// Marks an email as trashed during incremental synchronization.
    ///
    /// Safe no-op if the message was trashed on another client before ever being synced locally.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the query fails.
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

    /// Restores an email from trash during incremental sync (e.g. un-trashed on the provider).
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the query fails.
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

    /// Updates the remote server read flag for an email.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the query fails.
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

    /// Moves an email to the local trash view and records the current timestamp.
    ///
    /// # Errors
    /// Returns [`StorageError::NotFound`] if the email ID does not exist,
    /// or [`StorageError::Db`] if the query fails.
    pub async fn trash_email(&self, email_id: Uuid) -> Result<(), StorageError> {
        let result = sqlx::query("UPDATE emails SET is_trashed = 1, trashed_at = ? WHERE id = ?")
            .bind(unix_timestamp())
            .bind(email_id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }

    /// Restores a soft-deleted email back to the inbox view.
    ///
    /// # Errors
    /// Returns [`StorageError::NotFound`] if the email ID does not exist,
    /// or [`StorageError::Db`] if the query fails.
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

    /// Marks an email as viewed in the desktop application reading pane.
    ///
    /// # Errors
    /// Returns [`StorageError::NotFound`] if the email ID does not exist,
    /// or [`StorageError::Db`] if the query fails.
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

    /// Lists soft-deleted messages ordered by deletion date.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidInput`] if `limit` is invalid,
    /// or [`StorageError::Db`] if the query fails.
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

    /// Permanently removes trashed messages older than the specified retention period.
    ///
    /// Passing `0` sets the cutoff to the current Unix timestamp, permanently purging
    /// all currently trashed emails immediately.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidInput`] if `retention_days` is outside `0..=3650`,
    /// or [`StorageError::Db`] if the deletion query fails.
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
            AND (trashed_at IS NULL OR trashed_at <= ?)",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Purges older emails for an account exceeding the configured capacity limits.
    ///
    /// Invariant: General inbox purging explicitly excludes emails that matched watch
    /// criteria (`id NOT IN (SELECT email_id FROM classifications)`). This guarantees that
    /// receiving high volumes of regular mail never deletes priority findings prematurely.
    ///
    /// # Returns
    /// Returns `(inbox_purged, findings_purged)` representing the number of deleted rows.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if either deletion query fails.
    pub async fn purge_old_emails(
        &self,
        account_id: Uuid,
        max_inbox: i64,
        max_findings: i64
    ) -> Result<(u64, u64), StorageError> {
        let account_str = account_id.to_string();

        let inbox_result = sqlx::query(
            "DELETE FROM emails
             WHERE account_id = ?1
               AND is_trashed = 0
               AND id NOT IN (SELECT email_id FROM classifications)
               AND id NOT IN (
                   SELECT id FROM emails
                   WHERE account_id = ?1
                     AND is_trashed = 0
                     AND id NOT IN (SELECT email_id FROM classifications)
                   ORDER BY received_at DESC
                   LIMIT ?2
               )"
        )
        .bind(&account_str)
        .bind(max_inbox)
        .execute(&self.pool)
        .await?;

        let findings_result = sqlx::query(
            "DELETE FROM emails
             WHERE account_id = ?1
               AND is_trashed = 0
               AND id IN (SELECT email_id FROM classifications)
               AND id NOT IN (
                   SELECT m.id FROM emails m
                   JOIN classifications c ON c.email_id = m.id
                   WHERE m.account_id = ?1
                     AND m.is_trashed = 0
                   ORDER BY m.received_at DESC
                   LIMIT ?2
               )"
        )
        .bind(&account_str)
        .bind(max_findings)
        .execute(&self.pool)
        .await?;

        Ok((inbox_result.rows_affected(), findings_result.rows_affected()))
    }

    /// Iterates across all registered accounts and enforces inbox and findings capacities.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if querying accounts or executing purge queries fails.
    pub async fn purge_old_emails_all(
        &self,
        max_inbox: i64,
        max_findings: i64
    ) -> Result<(u64, u64), StorageError> {
        let accounts = self.list_accounts().await?;
        let mut total_inbox_purged = 0;
        let mut total_findings_purged = 0;

        for account in accounts {
            let (inbox_count, findings_count) = self
                .purge_old_emails(account.id, max_inbox, max_findings)
                .await?;
            total_inbox_purged += inbox_count;
            total_findings_purged += findings_count;
        }

        Ok((total_inbox_purged, total_findings_purged))
    }

    /// Marks all active messages as read and viewed.
    ///
    /// Targets an individual account if `account_id` is provided, or all mailboxes if `None`.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the update fails.
    pub async fn mark_all_read(&self, account_id: Option<Uuid>) -> Result<(), StorageError> {
        let account_str = account_id.map(|id| id.to_string());
        sqlx::query(
            "UPDATE emails
             SET is_read = 1, app_has_viewed = 1
             WHERE is_trashed = 0 AND (?1 IS NULL OR account_id = ?1)",
        )
        .bind(&account_str)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Clears AI classification findings from the findings board.
    ///
    /// Respects active filter contexts:
    /// - If `account_id` is provided, only classifications for that mailbox are cleared.
    /// - If `criterion_id` is provided, only classifications matching that specific rule are cleared.
    /// - If both are `None`, all findings across all mailboxes are cleared.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the deletion query fails.
    pub async fn clear_classifications(
        &self,
        account_id: Option<Uuid>,
        criterion_id: Option<Uuid>
    ) -> Result<u64, StorageError> {
        let account_str = account_id.map(|id| id.to_string());
        let criterion_str = criterion_id.map(|id| id.to_string());

        let result = sqlx::query(
            "DELETE FROM classifications
             WHERE (?1 IS NULL OR email_id IN (
                 SELECT id FROM emails WHERE account_id = ?1
             ))
             AND (?2 IS NULL OR criterion_id = ?2)",
        )
        .bind(&account_str)
        .bind(&criterion_str)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Legacy alias delegating to `clear_classifications` with no criterion filter.
    pub async fn clear_all_classifications(
        &self,
        account_id: Option<Uuid>,
    ) -> Result<u64, StorageError> {
        self.clear_classifications(account_id, None).await
    }

    /// Computes unread, findings, and total message counts for the dashboard.
    ///
    /// Evaluates counts within a single round-trip query using isolated subqueries
    /// rather than dispatching multiple separate database operations.
    /// Targets an individual account if `account_id` is provided, or all mailboxes if `None`.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the calculation query fails.
    pub async fn get_mailbox_stats(
        &self,
        account_id: Option<Uuid>,
    ) -> Result<MailboxStats, StorageError> {
        let account_str = account_id.map(|id| id.to_string());

        let row: (i64, i64, i64) = sqlx::query_as(
            "SELECT
                (SELECT COUNT(*) FROM emails 
                 WHERE is_trashed = 0 AND is_read = 0 AND app_has_viewed = 0 
                 AND (?1 IS NULL OR account_id = ?1)) AS unread_count,
                 
                (SELECT COUNT(DISTINCT c.email_id) FROM classifications c 
                 JOIN emails m ON m.id = c.email_id 
                 WHERE m.is_trashed = 0 AND m.is_read = 0 AND m.app_has_viewed = 0
                 AND (?1 IS NULL OR m.account_id = ?1)) AS findings_count,
                 
                (SELECT COUNT(*) FROM emails 
                 WHERE is_trashed = 0 
                 AND (?1 IS NULL OR account_id = ?1)) AS total_count",
        )
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