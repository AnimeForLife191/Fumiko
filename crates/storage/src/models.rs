use common::Provider;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents a linked email account.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct LinkedAccount {
    #[sqlx(try_from = "String")]
    pub id: Uuid,
    #[sqlx(try_from = "String")]
    pub provider: Provider,
    pub email_address: String,
    pub display_name: Option<String>,
    /// Opaque synchronization cursor: Gmail `historyId` or Microsoft Graph `deltaLink`.
    pub sync_cursor: Option<String>,
    pub last_synced_at: Option<i64>,
    pub last_sync_error: Option<String>,
    pub created_at: i64,
}

/// Lightweight message metadata cached in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq, Eq)]
pub struct EmailRow {
    #[sqlx(try_from = "String")]
    pub id: Uuid,
    #[sqlx(try_from = "String")]
    pub account_id: Uuid,
    pub provider_message_id: String,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub received_at: i64,
    pub is_read: bool,
    /// Local-only viewed flag; not synchronized back to the provider.
    pub app_has_viewed: bool,
    pub is_trashed: bool,
    pub trashed_at: Option<i64>,
    pub snippet: Option<String>,
}

/// Represents an email that matched an active watch criterion.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct MatchedEmailWithReason {
    #[sqlx(try_from = "String")]
    pub email_id: Uuid,
    #[sqlx(try_from = "String")]
    pub account_id: Uuid,
    #[sqlx(try_from = "String")]
    pub criterion_id: Uuid,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub received_at: i64,
    pub snippet: Option<String>,
    pub is_read: bool,
    pub app_has_viewed: bool,
    pub criterion_label: String,
    pub confidence: Option<f32>,
}

/// User-defined watch criterion evaluated by the local AI engine.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq, Eq)]
pub struct WatchCriterion {
    #[sqlx(try_from = "String")]
    pub id: Uuid,
    pub label: String,
    pub description: String,
    pub is_active: bool,
    pub created_at: i64,
}

// Represents the full count of each stat for UI
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MailboxStats {
    pub unread_count: i64,
    pub findings_count: i64,
    pub total_count: i64,
}