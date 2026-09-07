# Local Database and Secret Storage Notes

Storage in Fumiko is mostly about keeping local mailbox operations instant, resilient, and completely private. A desktop email client needs to work offline, render inboxes with zero latency, and survive sudden power losses or application crashes without corrupting mailbox state. Crucially, it must do all of this without turning ordinary database backups, logs, or diagnostics into credential leaks.

This document explains the architecture of the `storage` crate, details our hybrid storage model using SQLite and the operating system keyring, and outlines the database invariants and performance rules we maintain across the application.

## What we are storing (and what we deliberately do not store)

A common mistake in desktop email clients is dumping everything (tokens, headers, complete MIME bodies, and large attachments) into a single database file. That leads to heavy database bloat, slow index traversals, and unnecessary security risks.

Fumiko enforces a strict boundary between three distinct categories of data:

1. Authentication Secrets: Stored exclusively in the operating system credential vault (Apple Keychain, Windows Credential Manager, or Linux Secret Service). These include long-lived refresh tokens and custom user OAuth client secrets. If a user copies, backs up, or inspects the SQLite database file, no usable credentials or refresh tokens are exposed.
2. Synchronized Metadata: Stored in a local, indexed SQLite database. This includes message IDs, subjects, senders, timestamps, snippet previews, and AI classification results. This is just enough information to render the inbox and dashboard instantly.
3. Full Content and Bodies: Kept on the provider servers and retrieved on demand. Full bodies, raw MIME trees, and non-inline attachments are never stored in local database tables.

## The Hybrid Storage Model: TokenStore and Keyring

To keep database queries fast and secure, `Storage` wraps both an SQLite connection pool and a dedicated `TokenStore` backend.

### Abstraction and Test Isolation

Interacting directly with the operating system keychain inside database methods breaks automated continuous integration testing on headless environments, such as Linux test runners without an active D-Bus secret service.

The `TokenBackend` trait abstracts secret storage across two implementations:

* `KeyringBackend`: Production backend backed by the `keyring` crate.
* `InMemoryBackend`: Thread-safe, mutex-guarded hash map used by `Storage::new_for_test()`.

All sensitive operations, including account refresh tokens and custom user OAuth client secrets, route through this abstraction.

## Database Schema and Entity Relationships

The schema is normalized to allow accounts, emails, and AI classifications to evolve independently while enforcing referential integrity.

### Tables Overview

* `linked_accounts`: Stores account configuration, email addresses, display names, and opaque synchronization tokens.
* `emails`: Stores message metadata, read flags, local view state, and trash status.
* `watch_criteria`: Stores user-defined rules and descriptions used for categorization.
* `classifications`: Connects emails to matching criteria with optional confidence scores.
* `settings`: Simple key-value storage for application-level options.

### UUID v7 Primary Keys

All primary keys use UUID v7 (`Uuid::now_v7()`).

Unlike random UUID v4, UUID v7 embeds a millisecond-precision Unix timestamp in its most significant bits. This makes primary keys monotonically increasing and time-ordered, eliminating B-Tree index fragmentation and making timestamp-based sorting efficient directly on primary keys.

### Cascading Rules

Foreign keys are strictly enforced using `PRAGMA foreign_keys = ON`:

* Deleting an account from `linked_accounts` automatically cascades and removes all of its cached emails and classifications.
* Deleting a watch criterion from `watch_criteria` automatically purges its matching records from `classifications`, ensuring the dashboard findings clear immediately without leaving dangling foreign references.

## SQLite Engine Configuration and WAL Mode

Fumiko configures SQLite specifically for concurrent desktop usage:

1. Write-Ahead Logging (`journal_mode = WAL`): Allows background sync tasks to write incoming messages without blocking user interface read queries. Readers and writers operate concurrently.
2. `synchronous = NORMAL`: In traditional rollback journal mode, `NORMAL` risks database corruption during operating system crashes. In WAL mode, however, `NORMAL` guarantees full crash durability while eliminating redundant disk flushes on every write transaction. This significantly speeds up multi-email sync passes.
3. `busy_timeout = 5s`: If a write transaction is in progress, competing queries sleep and retry for up to 5 seconds before throwing an `SQLITE_BUSY` error, preventing lock contention panics.

## Indexing Strategy

To keep user interface views fast regardless of how many thousands of emails are stored, we maintain composite indexes tailored to our exact query patterns:

* `idx_emails_inbox` on `(account_id, is_trashed, received_at DESC)`: Covers the primary inbox query, filtering out trashed items and sorting by newest first without needing in-memory table sorts.
* `idx_emails_inbox_all` on `(is_trashed, received_at DESC)`: Accelerates the unified global inbox view across all linked accounts.
* `idx_emails_trash` on `(is_trashed, trashed_at DESC)`: Accelerates the trash view and allows background retention purging to quickly locate and delete expired rows without scanning active inbox messages.
* `idx_classifications_email` and `idx_classifications_criterion`: Accelerate dashboard finding joins and cascade deletions.

## Invariants and Sync Edge Cases

### 1. Idempotent Message Upserts

When background synchronization discovers messages, it calls `save_email`. If a message was already saved, we must update its state without throwing a unique constraint conflict.

If a message was previously saved with a missing snippet or subject, re-syncing updates those fields via `COALESCE` without resetting local flags like `app_has_viewed`.

### 2. Safe Trash State Transitions

During incremental sync, providers notify us of messages moved to Trash on the server. If an email was received and trashed on another device before Fumiko ever synced it, the message ID does not exist in our database.

`mark_trashed` executes an `UPDATE` query without requiring rows to be affected. If the message is not present locally, the update affects zero rows and returns `Ok(())` safely. Treating this as an error would cause the entire sync pass to abort on routine mailbox activity.

### 3. Atomic Cursor Updates and Error Clearing

When a sync pass completes successfully, updating the cursor, recording the timestamp, and clearing any prior error message happens in a single atomic SQL statement. If the sync succeeded, any previous error banner in the user interface is dismissed automatically.

### 4. Bounded Query Limits and Trash Retention

To prevent out-of-memory errors from unexpected inputs:

* All listing queries validate limits through `validate_limit(limit)`, enforcing that the limit stays between 0 and 1000.
* Trash purging enforces that retention days stay between 0 and 3650 (10 years), preventing integer overflow when computing Unix millisecond cutoffs.

## Complete Data Erasure (wipe_all_data)

When a user requests a complete reset of the application, data is removed thoroughly across both SQLite and the operating system keyring:

1. Keyring Wipe: Enumerates all linked accounts and explicitly deletes their refresh tokens from the operating system credential store.
2. Credential Cleanup: Deletes user-provided OAuth client secrets from the keyring.
3. Atomic Database Truncation: Wraps `DELETE FROM linked_accounts`, `DELETE FROM watch_criteria`, and `DELETE FROM settings` inside an explicit transaction. Foreign key cascades purge all emails and classifications.
4. Physical Disk Reclaim: Executes `PRAGMA wal_checkpoint(TRUNCATE)` followed by `VACUUM` to physically zero out and shrink the database file on disk.

## Storage and Database Checklist

When altering queries, adding tables, or modifying storage logic, verify these invariants:

* No Secrets in SQLite: Never add columns or settings that store raw refresh tokens, passwords, or client secrets in plain text.
* Enforce Keyring-First Deletion: Always remove secrets from the operating system keyring before deleting the corresponding database row.
* Maintain UUID v7: Generate all new primary keys with `Uuid::now_v7()` to preserve index locality.
* Keep Foreign Keys Enabled: Always ensure `PRAGMA foreign_keys = ON` is configured on all connections.
* Validate Query Bounds: Route all user-supplied query limits through `validate_limit`.
* Make Sync Operations Idempotent: Ensure `save_email`, `mark_trashed`, and `delete_by_provider_message_id` never fail when processing out-of-order or duplicate provider events.
* Use Non-Blocking WAL Defaults: Maintain `SqliteJournalMode::Wal` and `SqliteSynchronous::Normal` across all connection pools.