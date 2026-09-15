//! Local database engine and secure credential storage for Fumiko.
//!
//! Provides an asynchronous SQLite storage layer using `sqlx` in Write-Ahead Logging (WAL)
//! mode, monotonic time-ordered UUIDv7 primary keys, and strictly enforced foreign key cascades.
//!
//! # Architecture & Security Model
//! * Hybrid Storage: Sensitive credentials (OAuth refresh tokens, IMAP app passwords, and
//!   client secrets) live exclusively in the native operating system credential store via
//!   [`TokenStore`]. The local SQLite database stores only non-sensitive metadata, headers,
//!   previews, and sync cursors.
//! * WAL Mode & Concurrency: Configured with `PRAGMA journal_mode = WAL` and
//!   `PRAGMA synchronous = NORMAL`, allowing concurrent reader queries alongside background
//!   sync writes without lock contention or UI stutter.
//! * Crash-Resilient Write Locality: Monotonically increasing UUIDv7 timestamps maintain
//!   B-tree index locality and prevent database page fragmentation.

mod accounts;
mod criteria;
mod email;
pub mod models;
mod secrets;
mod settings;
mod sync;

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::time::{SystemTime, UNIX_EPOCH};

pub use common::StorageError;
pub use secrets::TokenStore;

/// Maximum allowed row limit for bounded list queries to prevent UI allocation spikes.
const MAX_QUERY_LIMIT: i64 = 1000;

/// Primary storage handle wrapping the SQLite connection pool and the native token vault.
#[derive(Clone, Debug)]
pub struct Storage {
    pub(crate) pool: SqlitePool,
    pub token_store: TokenStore,
}

impl Storage {
    /// Creates a new `Storage` instance using the provided SQLite pool and the production OS keyring.
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            token_store: TokenStore::with_keyring(),
        }
    }

    /// Creates an isolated `Storage` instance backed by an in-memory token store for testing.
    #[cfg(test)]
    pub fn new_for_test(pool: SqlitePool) -> Self {
        Self {
            pool,
            token_store: TokenStore::in_memory(),
        }
    }
}

/// Initializes an SQLite connection pool with WAL mode, foreign keys, and pending migrations.
///
/// SQLite defaults to `PRAGMA foreign_keys = OFF` on each opened connection.
/// Passing `.foreign_keys(true)` ensures that foreign key constraints and cascading deletes
/// are strictly enforced on every pooled connection.
///
/// Configures a connection pool capped at 5 connections, sets a 5-second busy timeout
/// for automatic lock retries, and executes embedded database migrations.
///
/// # Errors
/// Returns [`StorageError::Db`] if the database file cannot be opened or connected to,
/// or [`StorageError::Migration`] if executing embedded database migrations fails.
pub async fn init_pool(db_path: &str) -> Result<SqlitePool, StorageError> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

/// Returns the current Unix timestamp in seconds.
pub(crate) fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Validates that a requested query limit is non-negative and does not exceed [`MAX_QUERY_LIMIT`].
///
/// # Errors
/// Returns [`StorageError::InvalidInput`] if `limit < 0` or `limit > MAX_QUERY_LIMIT`.
pub(crate) fn validate_limit(limit: i64) -> Result<(), StorageError> {
    if limit < 0 {
        return Err(StorageError::InvalidInput(
            "limit must be non-negative".into(),
        ));
    }

    if limit > MAX_QUERY_LIMIT {
        return Err(StorageError::InvalidInput(
            "limit exceeds maximum allowed value".into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
pub async fn test_storage() -> Storage {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("failed to open in-memory sqlite");

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::migrate!()
        .run(&pool)
        .await
        .expect("failed to run migrations");

    Storage::new_for_test(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::Provider;

    #[tokio::test]
    async fn migration_and_foreign_keys() {
        let storage = test_storage().await;
        let (fk_enabled,): (i64,) = sqlx::query_as("PRAGMA foreign_keys")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        assert_eq!(fk_enabled, 1);
    }

    #[tokio::test]
    async fn add_and_get_oauth_account() {
        let storage = test_storage().await;
        let id = storage
            .add_account(Provider::Gmail, "alice@example.com", Some("Alice"))
            .await
            .unwrap();

        assert!(storage.account_exists("alice@example.com").await.unwrap());
        assert!(!storage.account_exists("bob@example.com").await.unwrap());

        let account = storage.get_account(id).await.unwrap().unwrap();
        assert_eq!(account.email_address, "alice@example.com");
        assert_eq!(account.provider, Provider::Gmail);
        assert_eq!(account.display_name.as_deref(), Some("Alice"));
        assert!(account.imap_host.is_none());
        assert_eq!(account.imap_port_u16(), None);
    }

    #[tokio::test]
    async fn add_and_get_imap_account() {
        let storage = test_storage().await;
        let id = storage
            .add_imap_account(
                Provider::Imap,
                "user@custom.mail",
                None,
                "imap.custom.mail",
                993,
            )
            .await
            .unwrap();

        let account = storage.get_account(id).await.unwrap().unwrap();
        assert_eq!(account.provider, Provider::Imap);
        assert_eq!(account.imap_host.as_deref(), Some("imap.custom.mail"));
        assert_eq!(account.imap_port, Some(993));
        assert_eq!(account.imap_port_u16(), Some(993));
    }

    #[tokio::test]
    async fn duplicate_email_returns_conflict() {
        let storage = test_storage().await;
        storage
            .add_account(Provider::Outlook, "dup@example.com", None)
            .await
            .unwrap();

        let err = storage
            .add_account(Provider::Outlook, "dup@example.com", None)
            .await
            .unwrap_err();

        match err {
            StorageError::Conflict => (),
            other => panic!("expected Conflict error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn display_name_mutations() {
        let storage = test_storage().await;
        let id = storage
            .add_account(Provider::Gmail, "name@example.com", None)
            .await
            .unwrap();

        storage.update_display_name(id, "Renamed").await.unwrap();
        let acct = storage.get_account(id).await.unwrap().unwrap();
        assert_eq!(acct.display_name.as_deref(), Some("Renamed"));

        storage.clear_display_name(id).await.unwrap();
        let acct = storage.get_account(id).await.unwrap().unwrap();
        assert_eq!(acct.display_name, None);
    }

    #[tokio::test]
    async fn delete_account_cascades_and_removes_secret() {
        let storage = test_storage().await;
        let account_id = storage
            .add_account(Provider::Gmail, "delete-me@example.com", None)
            .await
            .unwrap();

        storage
            .token_store
            .save_account_secret(account_id, "secret_refresh_token")
            .unwrap();
        assert_eq!(
            storage
                .token_store
                .get_account_secret(account_id)
                .unwrap()
                .as_deref(),
            Some("secret_refresh_token")
        );

        let email_id = storage
            .save_email(
                account_id,
                "msg-1",
                Some("Subject"),
                Some("Sender"),
                1000,
                false,
                Some("Snippet"),
            )
            .await
            .unwrap();

        storage.delete_account(account_id).await.unwrap();

        assert_eq!(storage.get_account(account_id).await.unwrap(), None);
        assert_eq!(storage.get_email(email_id).await.unwrap(), None);
        assert_eq!(
            storage.token_store.get_account_secret(account_id).unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn email_save_and_conflict_upsert() {
        let storage = test_storage().await;
        let acct_id = storage
            .add_account(Provider::Gmail, "sync@example.com", None)
            .await
            .unwrap();

        let id1 = storage
            .save_email(
                acct_id,
                "provider-id-42",
                Some("Original Subject"),
                Some("Alice"),
                100,
                false,
                Some("Original Snippet"),
            )
            .await
            .unwrap();

        let id2 = storage
            .save_email(
                acct_id,
                "provider-id-42",
                None,
                Some("Alice"),
                100,
                true,
                Some("Updated Snippet"),
            )
            .await
            .unwrap();

        assert_eq!(id1, id2);

        let row = storage.get_email(id1).await.unwrap().unwrap();
        assert_eq!(row.subject.as_deref(), Some("Original Subject"));
        assert_eq!(row.snippet.as_deref(), Some("Updated Snippet"));
        assert!(row.is_read);
        assert!(!row.app_has_viewed);
    }

    #[tokio::test]
    async fn mailbox_stats_calculation() {
        let storage = test_storage().await;
        let acct1 = storage
            .add_account(Provider::Gmail, "m1@example.com", None)
            .await
            .unwrap();
        let acct2 = storage
            .add_account(Provider::Outlook, "m2@example.com", None)
            .await
            .unwrap();

        let e1 = storage
            .save_email(acct1, "msg1", Some("Unread"), None, 200, false, None)
            .await
            .unwrap();
        let e2 = storage
            .save_email(acct1, "msg2", Some("Read"), None, 100, true, None)
            .await
            .unwrap();
        let _e3 = storage
            .save_email(acct2, "msg3", Some("Other Acct"), None, 50, false, None)
            .await
            .unwrap();

        let crit_id = storage
            .add_criterion("Urgent", "Needs action")
            .await
            .unwrap();
        storage
            .save_classification(e1, crit_id, Some(0.95))
            .await
            .unwrap();

        let stats_acct1 = storage.get_mailbox_stats(Some(acct1)).await.unwrap();
        assert_eq!(stats_acct1.total_count, 2);
        assert_eq!(stats_acct1.unread_count, 1);
        assert_eq!(stats_acct1.findings_count, 1);

        let stats_all = storage.get_mailbox_stats(None).await.unwrap();
        assert_eq!(stats_all.total_count, 3);
        assert_eq!(stats_all.unread_count, 2);
        assert_eq!(stats_all.findings_count, 1);

        storage.mark_email_viewed(e1).await.unwrap();
        let stats_after_view = storage.get_mailbox_stats(Some(acct1)).await.unwrap();
        assert_eq!(stats_after_view.unread_count, 0);

        storage.trash_email(e2).await.unwrap();
        let stats_after_trash = storage.get_mailbox_stats(Some(acct1)).await.unwrap();
        assert_eq!(stats_after_trash.total_count, 1);
    }

    #[tokio::test]
    async fn trash_retention_purge() {
        let storage = test_storage().await;
        let acct = storage
            .add_account(Provider::Gmail, "trash@example.com", None)
            .await
            .unwrap();

        let email_id = storage
            .save_email(acct, "old-msg", Some("Trash Me"), None, 50, true, None)
            .await
            .unwrap();

        storage.trash_email(email_id).await.unwrap();

        let old_time = unix_timestamp() - (100 * 86_400);
        sqlx::query("UPDATE emails SET trashed_at = ? WHERE id = ?")
            .bind(old_time)
            .bind(email_id.to_string())
            .execute(&storage.pool)
            .await
            .unwrap();

        let purged = storage.purge_expired_trash(30).await.unwrap();
        assert_eq!(purged, 1);
        assert_eq!(storage.get_email(email_id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn criteria_management_and_classification() {
        let storage = test_storage().await;
        storage.seed_default_criteria().await.unwrap();
        let active = storage.list_active_criteria().await.unwrap();
        assert_eq!(active.len(), 3);

        let acct = storage
            .add_account(Provider::Gmail, "crit@example.com", None)
            .await
            .unwrap();
        let email_id = storage
            .save_email(acct, "m1", Some("Invoice"), None, 100, false, None)
            .await
            .unwrap();

        let crit = &active[0];
        storage
            .save_classification(email_id, crit.id, Some(0.88))
            .await
            .unwrap();

        let matches = storage.list_criteria_for_all(10).await.unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].email_id, email_id);
        assert_eq!(matches[0].criterion_id, crit.id);
        assert_eq!(matches[0].confidence, Some(0.88));

        storage.delete_criterion(crit.id).await.unwrap();
        let matches_after = storage.list_criteria_for_all(10).await.unwrap();
        assert_eq!(matches_after.len(), 0);
    }

    #[tokio::test]
    async fn app_settings_and_credentials_rollback() {
        let storage = test_storage().await;

        storage.set_setting("theme", "dark").await.unwrap();
        assert_eq!(
            storage.get_setting("theme").await.unwrap().as_deref(),
            Some("dark")
        );
        storage.delete_setting("theme").await.unwrap();
        assert_eq!(storage.get_setting("theme").await.unwrap(), None);

        storage
            .save_user_google_credentials("google_client_id_123", "google_client_secret_xyz")
            .await
            .unwrap();

        let (cid, csec) = storage
            .get_user_google_credentials()
            .await
            .unwrap()
            .expect("credentials present");
        assert_eq!(cid, "google_client_id_123");
        assert_eq!(csec, "google_client_secret_xyz");

        storage.delete_user_google_credentials().await.unwrap();
        assert_eq!(storage.get_user_google_credentials().await.unwrap(), None);
    }

    #[tokio::test]
    async fn sync_cursor_and_error_handling() {
        let storage = test_storage().await;
        let id = storage
            .add_account(Provider::Gmail, "cursor@example.com", None)
            .await
            .unwrap();

        storage
            .set_sync_error(id, "Connection timed out")
            .await
            .unwrap();
        let acct = storage.get_account(id).await.unwrap().unwrap();
        assert_eq!(
            acct.last_sync_error.as_deref(),
            Some("Connection timed out")
        );

        storage.update_sync_cursor(id, "1234567").await.unwrap();
        let acct = storage.get_account(id).await.unwrap().unwrap();
        assert_eq!(acct.sync_cursor.as_deref(), Some("1234567"));
        assert_eq!(acct.last_sync_error, None);
        assert!(acct.last_synced_at.is_some());
    }

    #[tokio::test]
    async fn wipe_all_data_clears_database_and_keyring() {
        let storage = test_storage().await;
        let acct = storage
            .add_account(Provider::Gmail, "wipe@example.com", None)
            .await
            .unwrap();
        storage
            .token_store
            .save_account_secret(acct, "refresh_token")
            .unwrap();
        storage
            .set_setting("sample_setting", "value")
            .await
            .unwrap();

        storage.wipe_all_data().await.unwrap();

        assert_eq!(storage.list_accounts().await.unwrap().len(), 0);
        assert_eq!(storage.get_setting("sample_setting").await.unwrap(), None);
        assert_eq!(storage.token_store.get_account_secret(acct).unwrap(), None);
    }

    #[tokio::test]
    async fn capacity_pruning_protects_findings() {
        let storage = test_storage().await;
        let acct = storage
            .add_account(Provider::Gmail, "capacity@example.com", None)
            .await
            .unwrap();

        let old_finding = storage
            .save_email(acct, "finding-1", Some("Interview"), None, 100, false, None)
            .await
            .unwrap();
        let crit = storage.add_criterion("Jobs", "Interviews").await.unwrap();
        storage.save_classification(old_finding, crit, Some(0.9)).await.unwrap();

        let _e1 = storage.save_email(acct, "msg-1", Some("News 1"), None, 200, true, None).await.unwrap();
        let _e2 = storage.save_email(acct, "msg-2", Some("News 2"), None, 300, true, None).await.unwrap();
        let _e3 = storage.save_email(acct, "msg-3", Some("News 3"), None, 400, true, None).await.unwrap();

        let (inbox_purged, findings_purged) = storage.purge_old_emails(acct, 2, 10).await.unwrap();

        assert_eq!(inbox_purged, 1);
        assert_eq!(findings_purged, 0);
        assert!(storage.get_email(old_finding).await.unwrap().is_some());
    }
}