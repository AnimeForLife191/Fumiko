//! Relational database models and projection entities for Fumiko.

use common::Provider;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents a linked email account in local storage.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq, Eq)]
pub struct LinkedAccount {
    /// Monotonically increasing unique identifier (UUIDv7).
    #[sqlx(try_from = "String")]
    pub id: Uuid,
    /// Upstream mail provider (`gmail`, `outlook`, or `imap`).
    #[sqlx(try_from = "String")]
    pub provider: Provider,
    /// Primary email address associated with the account.
    pub email_address: String,
    /// User-configurable custom display name or alias.
    pub display_name: Option<String>,
    /// Provider-specific incremental synchronization resume token.
    pub sync_cursor: Option<String>,
    /// Unix timestamp (seconds) of the last successful synchronization.
    pub last_synced_at: Option<i64>,
    /// Diagnostic error message from the most recent failed sync pass, if any.
    pub last_sync_error: Option<String>,
    /// Unix timestamp (seconds) when the account was linked.
    pub created_at: i64,
    /// Hostname of the IMAP server (e.g. `imap.mail.me.com`). None for OAuth accounts.
    #[sqlx(default)]
    pub imap_host: Option<String>,
    /// Port number of the IMAP server stored as a signed 64-bit integer.
    #[sqlx(default)]
    pub imap_port: Option<i64>,
}

impl LinkedAccount {
    /// Safely validates and converts the database `imap_port` into a valid `u16` network port.
    pub fn imap_port_u16(&self) -> Option<u16> {
        self.imap_port.and_then(|p| u16::try_from(p).ok())
    }
}

/// Lightweight message metadata cached in SQLite.
///
/// Full MIME bodies and raw attachment streams are deliberately omitted to prevent
/// local database bloat and preserve system disk space.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq, Eq)]
pub struct EmailRow {
    /// Monotonically increasing unique identifier (UUIDv7).
    #[sqlx(try_from = "String")]
    pub id: Uuid,
    /// Foreign key referencing the parent [`LinkedAccount`].
    #[sqlx(try_from = "String")]
    pub account_id: Uuid,
    /// Upstream provider identifier (Gmail message ID, Graph ID, or IMAP UID).
    pub provider_message_id: String,
    /// Parsed email subject line.
    pub subject: Option<String>,
    /// Formatted sender header string (e.g. `"Jane Doe <jane@example.com>"`).
    pub sender: Option<String>,
    /// Server-reported delivery timestamp in Unix seconds.
    pub received_at: i64,
    /// Remote server read flag (`true` if read on the server).
    pub is_read: bool,
    /// Local UI view flag (`true` if opened in Fumiko's reading pane).
    pub app_has_viewed: bool,
    /// Soft-delete status flag indicating presence in the trash view.
    pub is_trashed: bool,
    /// Unix timestamp (seconds) when the message was marked as trashed.
    pub trashed_at: Option<i64>,
    /// Scrubbed plain-text preview snippet.
    pub snippet: Option<String>,
}

/// Represents an email that matched an active watch criterion with AI confidence scoring.
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
    /// Natural-language label of the matched watch criterion.
    pub criterion_label: String,
    /// Classification confidence score assigned by the local AI engine (0.0 to 1.0).
    pub confidence: Option<f32>,
}

/// User-defined classification rule evaluated by the local AI engine.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq, Eq)]
pub struct WatchCriterion {
    /// Monotonically increasing unique identifier (UUIDv7).
    #[sqlx(try_from = "String")]
    pub id: Uuid,
    /// Short human-readable category label (e.g. `"Careers & Interviews"`).
    pub label: String,
    /// Detailed natural-language prompt instructions describing what to match.
    pub description: String,
    /// Flag indicating whether the local AI engine evaluates incoming mail against this rule.
    pub is_active: bool,
    /// Unix timestamp (seconds) when the criterion was created.
    pub created_at: i64,
}

/// Mailbox summary statistics used to render real-time dashboard counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxStats {
    /// Number of active, unread, and unviewed messages.
    pub unread_count: i64,
    /// Number of unique unread messages matching active watch criteria.
    pub findings_count: i64,
    /// Total count of non-trashed messages across monitored mailboxes.
    pub total_count: i64,
}