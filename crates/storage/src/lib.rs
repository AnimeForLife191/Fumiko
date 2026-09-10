//! Local database engine and secure credential storage for Fumiko.
//!
//! Provides an SQLite storage layer configured for WAL-mode concurrency alongside
//! a `TokenStore` that isolates OAuth secrets and refresh tokens into the operating
//! system keyring.

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

const MAX_QUERY_LIMIT: i64 = 1000;

/// Primary storage handle wrapping the SQLite connection pool and the OS token store.
#[derive(Clone, Debug)]
pub struct Storage {
    pub(crate) pool: SqlitePool,
    /// Vault handle for managing secrets in the operating system keyring.
    pub token_store: TokenStore,
}

impl Storage {
    /// Creates a new `Storage` instance using the native OS keyring for secret storage.
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            token_store: TokenStore::with_keyring(),
        }
    }

    /// Creates a test storage instance using an in-memory hash map for credentials.
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
/// # SQLite Configuration
/// - `journal_mode = WAL`: Concurrent background writes without blocking UI reads.
/// - `synchronous = NORMAL`: Crash-safe without redundant disk syncs in WAL mode.
/// - `busy_timeout = 5s`: Automatic retry on lock contention.
pub async fn init_pool(db_path: &str) -> Result<SqlitePool, StorageError> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        // In WAL mode, Synchronous::Normal eliminates redundant fsyncs while preserving crash resilience.
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

pub(crate) fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

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
mod database_tests {
    use super::*;

    #[tokio::test]
    async fn initial_migration_succeeds_on_empty_database() {
        let _storage = test_storage().await;
    }

    #[tokio::test]
    async fn foreign_keys_are_enabled() {
        let storage = test_storage().await;
        let (fk_enabled,): (i64,) = sqlx::query_as("PRAGMA foreign_keys")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        assert_eq!(fk_enabled, 1);
    }

    #[tokio::test]
    async fn reopening_does_not_reapply_or_corrupt_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let path_str = db_path.to_str().unwrap();

        let pool1 = init_pool(path_str).await.unwrap();
        pool1.close().await;

        let pool2 = init_pool(path_str).await.unwrap();
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM linked_accounts")
            .fetch_one(&pool2)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}