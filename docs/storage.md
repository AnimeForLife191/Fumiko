# Local Database and Secret Storage Notes

Storage in Fumiko is designed around three goals: keeping local mailbox operations instant, keeping them reliable across crashes, and keeping all email content private. A desktop email client should let you browse previously synced messages and findings even when you are offline, load inboxes without lag, and survive sudden power loss without corrupting state. It must also do this without turning database backups, diagnostics, or logs into credential leaks.

This document explains the architecture of the `storage` crate, details the hybrid storage model using SQLite alongside the operating system keyring, and highlights the invariants we maintain across the app.

---

## What We Store (and What We Deliberately Do Not Store)

A common mistake in email clients is dumping everything (tokens, headers, complete MIME bodies, and attachments) into a single database file. That causes rapid database bloat, slows down queries, and creates serious security risks.

Fumiko enforces a strict boundary between three distinct categories of data:

1. **Authentication Secrets**: Stored exclusively in the operating system credential vault (Apple Keychain, Windows Credential Manager, or Linux Secret Service). These include account refresh tokens and custom Google OAuth client secrets. If someone copies, inspects, or backs up the SQLite database file, no usable passwords or secrets are exposed.
2. **Synchronized Metadata and Settings**: Stored in a local, indexed SQLite database. This includes message IDs, subjects, senders, timestamps, preview snippets, AI classification results, and custom public Client IDs. This is just enough information to render the inbox and findings dashboard instantly.
3. **Full Content and Bodies**: Kept on the provider servers and retrieved on demand. Full MIME payloads, raw HTML markup, and non-inline attachments are never stored in local SQLite tables.

---

## The Hybrid Storage Model: TokenStore and Keyring

To keep database queries fast while keeping credentials safe, `Storage` wraps both an SQLite connection pool and a dedicated `TokenStore` backend.

### Abstraction and Test Isolation
Interacting directly with the operating system keychain inside unit tests breaks automated continuous integration on headless environments, such as Linux CI runners without an active D-Bus Secret Service session.

The `TokenBackend` trait abstracts secret storage across two implementations:

* `KeyringBackend`: Production backend backed by the `keyring` crate, communicating with native OS credential vaults.
* `InMemoryBackend`: Thread-safe, mutex-guarded hash map used by `Storage::new_for_test()`.

All sensitive operations route through this abstraction, allowing comprehensive automated testing without platform credential dependencies.

---

## Database Schema and Entity Relationships

The schema is normalized to allow accounts, emails, and AI classifications to evolve independently while maintaining referential integrity.

### Tables Overview
* `linked_accounts`: Stores account configuration, email addresses, display names, sync cursors, and error messages.
* `emails`: Stores message metadata, read flags, local view state, and trash status.
* `watch_criteria`: Stores user-defined rules and descriptions used for background AI classification.
* `classifications`: Connects emails to matching criteria along with confidence scores.
* `settings`: Simple key-value storage for application-level options and public Client IDs.

### UUIDv7 Primary Keys
All primary keys use UUIDv7 (`Uuid::now_v7()`).

Unlike random UUIDv4, UUIDv7 embeds a millisecond-precision Unix timestamp in its most significant bits. This makes primary keys monotonically increasing and time-ordered. This eliminates B-Tree index fragmentation, maintains write locality on disk, and makes timestamp-based sorting efficient directly on the primary key index.

### Cascading Rules
Foreign keys are strictly enforced using `PRAGMA foreign_keys = ON`:

* Deleting an account from `linked_accounts` automatically cascades to remove all of its cached emails and classifications.
* Deleting a watch criterion from `watch_criteria` automatically cascades to purge its matching records from `classifications`, clearing the dashboard findings immediately without leaving dangling records behind.

---

## SQLite Configuration and WAL Mode

Fumiko configures SQLite specifically for concurrent desktop usage:

1. **Write-Ahead Logging (`journal_mode = WAL`)**: Allows background sync tasks to write incoming messages without blocking UI read queries. Readers and writers operate simultaneously without lock contention.
2. **Crash-Safe Performance (`synchronous = NORMAL`)**: In traditional rollback journal mode, `NORMAL` risks corruption during operating system crashes. In WAL mode, however, `NORMAL` guarantees full crash durability while eliminating redundant disk syncs (`fsync`) on write transactions. This keeps multi-email sync passes fast on consumer drives.
3. **Automatic Lock Retries (`busy_timeout = 5s`)**: If a write transaction is in progress, competing queries sleep and retry automatically for up to 5 seconds before returning an `SQLITE_BUSY` error, preventing UI lockup panics.
4. **Connection Pooling**: Uses an active pool capped at 5 connections, giving background sync and foreground UI readers dedicated channels.

---

## Indexing Strategy

To keep inbox lists responsive even with thousands of stored emails, we maintain composite indexes tailored to specific query patterns:

* `idx_emails_inbox` on `(account_id, is_trashed, received_at DESC)`: Covers the filtered account inbox view, excluding trashed items and sorting by newest first without in-memory sorting.
* `idx_emails_inbox_all` on `(is_trashed, received_at DESC)`: Accelerates the unified global inbox view across all connected accounts.
* `idx_emails_trash` on `(is_trashed, trashed_at DESC)`: Accelerates the trash view and allows background retention purging to quickly find expired rows without scanning active inbox mail.
* `idx_classifications_email` and `idx_classifications_criterion`: Accelerate findings dashboard queries and cascade deletions.

---

## Invariants and Sync Edge Cases

### 1. Idempotent Message Upserts
When background sync downloads new emails, it calls `save_email`. If an email was already saved during a previous pass, we update its state without throwing a unique constraint conflict.

We use SQLite's `ON CONFLICT(account_id, provider_message_id) DO UPDATE` syntax. If an email is re-synced, read flags are updated and missing snippets or subjects are filled using `COALESCE` without resetting local flags like `app_has_viewed`.

### 2. Safe Trash Transitions
During incremental sync, providers notify us of messages moved to Trash on the server. If an email was received and trashed on another device before Fumiko ever synced it, the message ID does not exist in our database.

`mark_trashed` runs an `UPDATE` query without requiring rows to be affected. If the message is missing locally, the query affects zero rows and returns `Ok(())` safely. Treating this as an error would cause the entire sync pass to abort on routine mailbox activity.

### 3. Atomic Cursor Updates and Error Dismissal
When a sync pass completes successfully, updating the cursor, recording the timestamp, and clearing any prior error message happens in a single atomic SQL statement. If the sync succeeded, any previous error banner in the user interface is dismissed automatically.

### 4. Keyring-First Account Deletion
In `delete_account`, the account's refresh token is deleted from the OS keyring before deleting the SQLite row. If the keyring call fails, the database row is left intact so the user can retry unlinking. This prevents orphaned secrets from lingering in the OS vault.

### 5. Automatic Credential Rollback
When saving custom Google credentials, `save_user_google_credentials` writes the client secret to the keyring first, then attempts to save the client ID in SQLite settings. If the SQLite write fails, the secret is automatically deleted from the keyring to keep storage states synchronized.

### 6. Bounded Limits and Retention Bounds
To protect against integer overflow or excessive memory allocations:
* All listing queries validate limits through `validate_limit`, bounding them between 0 and 1000.
* Automatic trash purging enforces retention days between 0 and 3650 (10 years) before converting days into seconds.

---

## Complete Data Erasure (wipe_all_data)

When a user clicks 'Wipe All Local Data' in settings, data is removed thoroughly across both SQLite and the OS keyring:

1. **Keyring Wipe**: Enumerates all linked accounts and deletes their refresh tokens from the OS vault.
2. **Custom Key Cleanup**: Deletes any user-configured Google OAuth client secrets from the keyring.
3. **Atomic Database Truncation**: Wraps `DELETE FROM linked_accounts`, `DELETE FROM watch_criteria`, and `DELETE FROM settings` in an explicit transaction. Foreign key cascades purge all cached emails and classifications.
4. **Physical Disk Reclaim**: Runs `PRAGMA wal_checkpoint(TRUNCATE)` followed by `VACUUM` to physically zero out and shrink the database file on disk.

---

## Storage Checklist

When altering queries, adding tables, or modifying storage methods, verify these rules:

* Never add columns or settings that store raw refresh tokens, passwords, or client secrets in plain text SQLite tables.
* Always delete credentials from the OS keyring before removing corresponding database account rows.
* Generate all primary keys using `Uuid::now_v7()` to preserve index locality.
* Ensure `PRAGMA foreign_keys = ON` remains enabled across all connection pool options.
* Route user-supplied query limits through `validate_limit`.
* Keep sync operations idempotent so duplicate or out-of-order provider events never crash a sync cycle.
* Maintain `SqliteJournalMode::Wal` and `SqliteSynchronous::Normal` across connection pools.