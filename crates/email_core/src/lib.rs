//! Core email synchronization engine, provider abstractions, and account linking.
//!
//! Translates provider-specific protocols (Google History API and Microsoft Graph Delta queries)
//! into a unified two-phase sync model (discovery followed by bounded metadata hydration).

mod email_providers;
mod linking;
mod sync_page;

pub use email_providers::{GmailProvider, OutlookProvider};
pub use linking::{link_gmail_account, link_outlook_account};
pub use sync_page::{SyncCursor, SyncOptions, SyncPage, SyncService};

use common::ProviderError;

/// Basic profile information for an authenticated mailbox user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub email_address: String,
    pub display_name: Option<String>,
}

/// Lightweight message summary fetched during phase 2 hydration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MessageMetadata {
    pub id: String,
    pub subject: String,
    pub from: String,
    pub received_at: i64,
    pub is_read: bool,
    pub snippet: Option<String>,
}

/// Metadata describing a message attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentMeta {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
}

/// Complete message payload retrieved on demand when viewing an email or running Tier 2 AI classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullMessage {
    pub id: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub attachments: Vec<AttachmentMeta>,
    pub unique_body_html: Option<String>,
}

/// Common interface implemented by email service backends (Gmail, Outlook).
#[async_trait::async_trait]
pub trait EmailProvider: Send + Sync {
    /// Retrieves the mailbox owner's email address and profile name.
    async fn get_profile(&self, access_token: &str) -> Result<Profile, ProviderError>;

    /// Executes the initial discovery pass, retrieving the latest message IDs and establishing a baseline cursor.
    async fn initial_sync(
        &self,
        access_token: &str,
        options: SyncOptions,
    ) -> Result<SyncPage, ProviderError>;

    /// Queries incremental mailbox changes that occurred since the provided sync cursor.
    async fn incremental_sync(
        &self,
        access_token: &str,
        cursor: &SyncCursor,
    ) -> Result<SyncPage, ProviderError>;

    /// Hydrates lightweight headers and preview snippets for a batch of message IDs.
    async fn fetch_message_metadata(
        &self,
        access_token: &str,
        message_ids: &[String],
    ) -> Result<Vec<Result<MessageMetadata, ProviderError>>, ProviderError>;

    /// Downloads and parses the full MIME body, HTML markup, and attachment metadata for a single email.
    async fn fetch_full_message(
        &self,
        access_token: &str,
        message_id: &str,
    ) -> Result<FullMessage, ProviderError>;
}