//! Core email synchronization engine, provider abstractions, and account linking.
//!
//! Translates provider-specific protocols (Gmail History API, Microsoft Graph Delta queries,
//! and RFC 3501 IMAP) into a unified two-phase sync model.
//!
//! # Synchronization Invariants
//! - **Two-Phase Architecture**: Discovery queries change-tracking endpoints for message IDs,
//!   followed by bounded concurrent metadata hydration (headers and preview snippets only).
//!   Full MIME payloads are retrieved on demand when viewing emails or running Tier 2 AI scans.
//! - **Safe Cursor Advancement**: Sync cursors update in SQLite only after an entire batch
//!   has successfully hydrated. Dropped network packets leave the cursor at its previous
//!   position to prevent permanently skipping unread mail.
//! - **The `BODY.PEEK` Invariant**: IMAP snippet and body fetches strictly use `BODY.PEEK`
//!   instead of `BODY[]`, preventing background classification from marking unread emails as read.
//! - **Cursor Expiration Resilience**: Upstream retention expiries (Gmail HTTP 404, Graph HTTP 410,
//!   or IMAP `UIDVALIDITY` mismatches) map to [`ProviderError::CursorExpired`], triggering an
//!   automatic baseline resync.

mod email_providers;
mod linking;
mod sync_page;

pub use email_providers::{GmailProvider, ImapProvider, OutlookProvider};
pub use linking::{link_gmail_account, link_imap_account, link_outlook_account};
pub use sync_page::{SyncCursor, SyncOptions, SyncPage, SyncPhase, SyncProgress, SyncService};

use common::ProviderError;

/// Basic profile information for an authenticated mailbox user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Mailbox email address.
    pub email_address: String,
    /// User display name reported by the provider, if available.
    pub display_name: Option<String>,
}

/// Lightweight message summary fetched during phase 2 hydration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MessageMetadata {
    /// Upstream provider message identifier (Gmail ID, Graph ID, or IMAP UID).
    pub id: String,
    /// Parsed subject line.
    pub subject: String,
    /// Formatted sender string (e.g. `"Alice <alice@example.com>"`).
    pub from: String,
    /// Delivery timestamp in Unix seconds.
    pub received_at: i64,
    /// Server-side read flag status.
    pub is_read: bool,
    /// Plain-text preview snippet scrubbed of MIME boundaries.
    pub snippet: Option<String>,
}

/// Metadata describing a non-inline message attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentMeta {
    /// Provider attachment identifier.
    pub id: String,
    /// Original file name.
    pub filename: String,
    /// Detected MIME content type.
    pub mime_type: String,
    /// File size in bytes.
    pub size: u64,
}

/// Complete message payload retrieved on demand when viewing an email or running Tier 2 AI classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullMessage {
    /// Upstream provider message identifier.
    pub id: String,
    /// Plain-text body representation.
    pub body_text: Option<String>,
    /// Sanitized HTML markup with inline CID images converted to Base64 data URIs.
    pub body_html: Option<String>,
    /// Non-inline attachments.
    pub attachments: Vec<AttachmentMeta>,
    /// Thread-unique body content (supported by Microsoft Graph).
    pub unique_body_html: Option<String>,
}

/// Common interface implemented by email service backends (Gmail, Outlook, and IMAP).
#[async_trait::async_trait]
pub trait EmailProvider: Send + Sync {
    /// Retrieves the mailbox owner's email address and profile name.
    ///
    /// # Errors
    /// Returns [`ProviderError::Unauthorized`] if credentials are invalid,
    /// or [`ProviderError::Other`] on network failure.
    async fn get_profile(&self, access_token: &str) -> Result<Profile, ProviderError>;

    /// Executes the initial discovery pass, retrieving the latest message IDs and establishing a baseline cursor.
    ///
    /// # Errors
    /// Returns [`ProviderError`] on rate-limiting, authentication failure, or invalid payload structure.
    async fn initial_sync(
        &self,
        access_token: &str,
        options: SyncOptions,
    ) -> Result<SyncPage, ProviderError>;

    /// Queries incremental mailbox changes that occurred since the provided sync cursor.
    ///
    /// # Errors
    /// Returns [`ProviderError::CursorExpired`] if the cursor is stale,
    /// [`ProviderError::RateLimited`] if throttled, or [`ProviderError::Unauthorized`] on token expiry.
    async fn incremental_sync(
        &self,
        access_token: &str,
        cursor: &SyncCursor,
    ) -> Result<SyncPage, ProviderError>;

    /// Hydrates lightweight headers and preview snippets for a batch of message IDs.
    ///
    /// Runs bounded concurrent requests (maximum 10) to respect provider burst ceilings.
    ///
    /// # Errors
    /// Returns [`ProviderError`] if the batch request fails.
    async fn fetch_message_metadata(
        &self,
        access_token: &str,
        message_ids: &[String],
    ) -> Result<Vec<Result<MessageMetadata, ProviderError>>, ProviderError>;

    /// Downloads and parses the full MIME body, HTML markup, and attachment metadata for a single email.
    ///
    /// # Errors
    /// Returns [`ProviderError`] if the message cannot be found or downloaded.
    async fn fetch_full_message(
        &self,
        access_token: &str,
        message_id: &str,
    ) -> Result<FullMessage, ProviderError>;
}