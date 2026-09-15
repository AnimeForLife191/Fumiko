//! Global constants, configuration limits, and storage key identifiers.

/// Application identifier used as the service namespace in the host OS credential vault
/// (Windows Credential Manager, Apple Keychain, or Linux Secret Service).
pub const APP_SERVICE_NAME: &str = "fumiko";

/// Key names used for reading and writing rows in the SQLite `settings` table.
///
/// Plain metadata and public client identifiers are stored here, while sensitive
/// secrets (client secrets, refresh tokens, passwords) live in the OS Keyring.
pub mod setting_keys {
    /// Public Google OAuth client ID configured by the user.
    pub const USER_GOOGLE_CLIENT_ID: &str = "user_google_client_id";
    /// Public Microsoft Azure / Entra client ID configured by the user.
    pub const USER_MICROSOFT_CLIENT_ID: &str = "user_microsoft_client_id";
    /// Identifier or filename of the active local AI model.
    pub const ACTIVE_AI_MODEL_ID: &str = "active_ai_model_id";
    /// Days before soft-deleted emails in the trash table are purged.
    pub const TRASH_RETENTION_DAYS: &str = "trash_retention_days";
    /// Active local AI backend ("builtin" or "ollama").
    pub const AI_BACKEND: &str = "ai_backend";
    /// Number of recent emails to fetch during the first mailbox synchronization.
    pub const INITIAL_SYNC_LIMIT: &str = "initial_sync_limit";
    /// Maximum number of general inbox emails retained and displayed per account.
    pub const INBOX_CAPACITY: &str = "inbox_capacity";
    /// Maximum number of flagged findings retained and displayed per account.
    pub const FINDINGS_CAPACITY: &str = "findings_capacity";
}

/// Identifiers for standalone secrets stored in the OS Keyring.
///
/// Account-specific secrets (OAuth refresh tokens and IMAP app passwords) are
/// keyed directly by the account's UUID string rather than constants in this module.
pub mod keyring_keys {
    /// Confidential client secret for user-configured Google OAuth credentials.
    pub const USER_GOOGLE_CLIENT_SECRET: &str = "user_google_client_secret";
}

/// Supported local AI inference backend engines.
pub mod ai_backends {
    /// Bundled zero-setup llama-server sidecar process running on port 11435.
    pub const BUILTIN: &str = "builtin";
    /// External user-managed Ollama daemon running on port 11434.
    pub const OLLAMA: &str = "ollama";
}

/// Global operational limits, timeouts, and resource caps.
pub mod config {
    use std::time::Duration;

    /// Background polling interval for active account inbox watchers.
    pub const POLL_INTERVAL: Duration = Duration::from_secs(60);
    pub const POLL_INTERVAL_SECS: u64 = 60;

    /// Default duration before soft-deleted emails are permanently expunged.
    pub const DEFAULT_TRASH_RETENTION_DAYS: i64 = 30;
    /// Enforced minimum retention window to prevent accidental permanent deletion.
    pub const MIN_TRASH_RETENTION_DAYS: i64 = 7;
    /// Enforced maximum retention window to prevent database bloat.
    pub const MAX_TRASH_RETENTION_DAYS: i64 = 365;

    /// Default email fetch limit during initial mailbox synchronization.
    pub const DEFAULT_INITIAL_SYNC_LIMIT: u32 = 100;
    /// Upper bound on initial sync fetch limit to prevent provider rate limits.
    pub const MAX_INITIAL_SYNC_LIMIT: u32 = 200;

    /// Maximum number of full email bodies downloaded per sync cycle for Tier 2 AI classification.
    /// This bound prevents network saturation and controls on-device compute usage.
    pub const MAX_TIER2_FETCHES_PER_SYNC: usize = 3;

    /// Context window token limit enforced during local LLM inference (-c 2048 / num_ctx: 2048).
    /// Prevents excessive KV cache RAM allocation on systems with limited memory.
    pub const AI_MAX_CONTEXT_TOKENS: u32 = 2048;

    /// Maximum output generation tokens for structured classification responses.
    /// Because the classification schema requires only 30 to 50 tokens, capping generation
    /// prevents runaway inference loops.
    pub const AI_MAX_OUTPUT_TOKENS: u32 = 128;

    /// Default inbox email capacity per account.
    pub const DEFAULT_INBOX_CAPACITY: u32 = 200;
    /// Enforced minimum inbox capacity.
    pub const MIN_INBOX_CAPACITY: u32 = 50;
    /// Enforced maximum inbox capacity to prevent memory spikes.
    pub const MAX_INBOX_CAPACITY: u32 = 1000;

    /// Default findings capacity per account.
    pub const DEFAULT_FINDINGS_CAPACITY: u32 = 200;
    /// Enforced minimum findings capacity.
    pub const MIN_FINDINGS_CAPACITY: u32 = 50;
    /// Enforced maximum findings capacity.
    pub const MAX_FINDINGS_CAPACITY: u32 = 1000;
}