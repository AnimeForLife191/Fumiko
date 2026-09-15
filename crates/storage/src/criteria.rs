//! Database operations for user-defined watch criteria and AI classification matches.

use crate::validate_limit;

use super::models::{MatchedEmailWithReason, WatchCriterion};
use super::{Storage, StorageError, unix_timestamp};
use uuid::Uuid;

impl Storage {
    /// Seeds default starter criteria on a fresh installation if not previously initialized.
    ///
    /// Uses the `has_seeded_default_criteria` setting as a persistent tombstone flag.
    /// If the user deliberately deleted all criteria, this flag prevents the defaults
    /// from being re-seeded on subsequent application launches.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if checking or writing default criteria records fails.
    pub async fn seed_default_criteria(&self) -> Result<(), StorageError> {
        let already_seeded: Option<(String,)> =
            sqlx::query_as("SELECT value FROM settings WHERE key = 'has_seeded_default_criteria'")
                .fetch_optional(&self.pool)
                .await?;

        if already_seeded.is_some() {
            return Ok(());
        }

        let (existing_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM watch_criteria")
            .fetch_one(&self.pool)
            .await?;

        if existing_count > 0 {
            sqlx::query(
                "INSERT INTO settings (key, value) VALUES ('has_seeded_default_criteria', 'true')
                 ON CONFLICT(key) DO UPDATE SET value = 'true'",
            )
            .execute(&self.pool)
            .await?;
            return Ok(());
        }

        let defaults = [
            (
                "Interview Invitations",
                "Direct requests to schedule or confirm a job interview, phone screening, hiring call, or recruiter conversation. Does not include automated job posting alerts.",
            ),
            (
                "Job Alerts & Openings",
                "Automated job recommendations, daily job match digests, and new role alerts from platforms such as Indeed, LinkedIn, ZipRecruiter, and Glassdoor.",
            ),
            (
                "Account Security & Logins",
                "Security verification codes, multi-factor authentication (2FA/MFA) prompts, password reset links, and new sign-in or device alerts.",
            ),
            (
                "Receipts & Orders",
                "Purchase confirmations, payment receipts, subscription renewal invoices, order summaries, and package shipping or tracking updates.",
            ),
            (
                "Promotions & Deals",
                "Marketing discounts, promotional coupon codes, sales events, limited-time offers, and store deal announcements.",
            ),
        ];

        let created_at = unix_timestamp();

        for (label, description) in defaults {
            let id = Uuid::now_v7().to_string();
            sqlx::query(
                "INSERT INTO watch_criteria (id, label, description, is_active, created_at)
                 VALUES (?, ?, ?, 1, ?)",
            )
            .bind(id)
            .bind(label)
            .bind(description)
            .bind(created_at)
            .execute(&self.pool)
            .await?;
        }

        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ('has_seeded_default_criteria', 'true')
             ON CONFLICT(key) DO UPDATE SET value = 'true'",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Inserts a new user-defined watch criterion.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the database insert fails.
    pub async fn add_criterion(
        &self,
        label: &str,
        description: &str,
    ) -> Result<Uuid, StorageError> {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO watch_criteria (id, label, description, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(label)
        .bind(description)
        .bind(unix_timestamp())
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Deletes a watch criterion.
    ///
    /// Foreign key cascades on `classifications.criterion_id` automatically remove
    /// all corresponding findings from the dashboard.
    ///
    /// # Errors
    /// Returns [`StorageError::NotFound`] if the criterion does not exist,
    /// or [`StorageError::Db`] if the deletion query fails.
    pub async fn delete_criterion(&self, criterion_id: Uuid) -> Result<(), StorageError> {
        let result = sqlx::query("DELETE FROM watch_criteria WHERE id = ?")
            .bind(criterion_id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }

    /// Lists all watch criteria ordered chronologically by creation date.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the query fails.
    pub async fn list_criteria(&self) -> Result<Vec<WatchCriterion>, StorageError> {
        let rows = sqlx::query_as::<_, WatchCriterion>(
            "SELECT id, label, description, is_active, created_at
             FROM watch_criteria
             ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Lists only active watch criteria evaluated by the local AI engine.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the query fails.
    pub async fn list_active_criteria(&self) -> Result<Vec<WatchCriterion>, StorageError> {
        let rows = sqlx::query_as::<_, WatchCriterion>(
            "SELECT id, label, description, is_active, created_at
             FROM watch_criteria WHERE is_active = 1 ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Enables or disables an individual watch criterion.
    ///
    /// # Errors
    /// Returns [`StorageError::NotFound`] if the criterion ID does not exist,
    /// or [`StorageError::Db`] if the update fails.
    pub async fn set_criterion_active(
        &self,
        criterion_id: Uuid,
        is_active: bool,
    ) -> Result<(), StorageError> {
        let result = sqlx::query("UPDATE watch_criteria SET is_active = ? WHERE id = ?")
            .bind(is_active)
            .bind(criterion_id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }

    /// Counts non-trashed messages that have matched at least one active criterion.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the count query fails.
    pub async fn count_classified_emails(&self) -> Result<i64, StorageError> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(DISTINCT c.email_id)
             FROM classifications c
             JOIN emails m ON m.id = c.email_id
             WHERE m.is_trashed = 0",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    /// Lists emails matched against watch criteria across all monitored mailboxes.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidInput`] if `limit` is invalid,
    /// or [`StorageError::Db`] if the query fails.
    pub async fn list_criteria_for_all(
        &self,
        limit: i64,
    ) -> Result<Vec<MatchedEmailWithReason>, StorageError> {
        validate_limit(limit)?;
        let rows = sqlx::query_as::<_, MatchedEmailWithReason>(
            "SELECT
                m.id AS email_id,
                m.account_id,
                c.criterion_id AS criterion_id,
                m.subject,
                m.sender,
                m.received_at,
                m.snippet,
                m.is_read,
                m.app_has_viewed,
                w.label AS criterion_label,
                c.confidence
            FROM emails m
            JOIN classifications c ON c.email_id = m.id
            JOIN watch_criteria w ON w.id = c.criterion_id
            WHERE m.is_trashed = 0
            ORDER BY m.received_at DESC
            LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Lists emails matched against watch criteria for an individual account.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidInput`] if `limit` is invalid,
    /// or [`StorageError::Db`] if the query fails.
    pub async fn list_criteria_for_account(
        &self,
        account_id: Uuid,
        limit: i64,
    ) -> Result<Vec<MatchedEmailWithReason>, StorageError> {
        validate_limit(limit)?;
        let rows = sqlx::query_as::<_, MatchedEmailWithReason>(
            "SELECT
                m.id AS email_id,
                m.account_id,
                c.criterion_id AS criterion_id,
                m.subject,
                m.sender,
                m.received_at,
                m.snippet,
                m.is_read,
                m.app_has_viewed,
                w.label AS criterion_label,
                c.confidence
            FROM emails m
            JOIN classifications c ON c.email_id = m.id
            JOIN watch_criteria w ON w.id = c.criterion_id
            WHERE m.account_id = ? AND m.is_trashed = 0
            ORDER BY m.received_at DESC
            LIMIT ?",
        )
        .bind(account_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Removes an individual email classification link from the findings board.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the deletion query fails.
    pub async fn delete_classification(
        &self,
        email_id: Uuid,
        criterion_id: Uuid,
    ) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM classifications WHERE email_id = ? AND criterion_id = ?")
            .bind(email_id.to_string())
            .bind(criterion_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Saves or updates an AI classification match with its confidence score.
    ///
    /// Uses composite `ON CONFLICT(email_id, criterion_id)` to update scores idempotently
    /// if a message is re-evaluated across sync passes.
    ///
    /// # Errors
    /// Returns [`StorageError::Db`] if the upsert query fails.
    pub async fn save_classification(
        &self,
        email_id: Uuid,
        criterion_id: Uuid,
        confidence: Option<f32>,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO classifications
                (id, email_id, criterion_id, confidence, created_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(email_id, criterion_id)
            DO UPDATE SET confidence = excluded.confidence",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(email_id.to_string())
        .bind(criterion_id.to_string())
        .bind(confidence)
        .bind(unix_timestamp())
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}