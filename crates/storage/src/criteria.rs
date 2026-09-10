use crate::validate_limit;

use super::models::{MatchedEmailWithReason, WatchCriterion};
use super::{Storage, StorageError, unix_timestamp};
use uuid::Uuid;

impl Storage {
    /// Inserts a new watch criterion for automated AI classification.
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

    /// Deletes a watch criterion and cascades deletion to all matching classification rows.
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

    /// Lists all watch criteria, including inactive rules.
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

    /// Lists only active watch criteria used for background classification.
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

    /// Counts distinct non-trashed emails that have matched at least one active criterion.
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

    /// Lists matched emails and their classification reasons across all accounts.
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

    /// Lists matched emails and classification reasons for a specific linked account.
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

    /// Removes a specific classification link between an email and a criterion.
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

    /// Records or updates an AI classification match with its confidence score.
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