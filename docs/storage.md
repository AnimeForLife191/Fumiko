# Local Database and Secret Storage Notes

Storage in Fumiko is designed around three goals: keeping local mailbox operations instant, keeping them reliable across crashes, and keeping all email content private. A desktop email client should let you browse previously synced messages and findings even when you are offline, load inboxes without lag, and survive sudden power loss without corrupting state. It must also do this without turning database backups, diagnostics, or logs into credential leaks.

This document explains the architecture of the `storage` crate, details the hybrid storage model using SQLite alongside the operating system keyring, and highlights the invariants maintained across the app.

## What We Store (and What We Deliberately Do Not Store)

A common mistake in email clients is dumping everything (tokens, headers, complete MIME bodies, and attachments) into a single database file. That causes rapid database bloat, slows down queries, and creates serious security risks.

Fumiko enforces a strict boundary between three distinct categories of data:

1. **Authentication Secrets**: Stored exclusively in the operating system credential vault (Apple Keychain, Windows Credential Manager, or Linux Secret Service). These include account refresh tokens, IMAP app passwords, and custom Google OAuth client secrets. If someone copies, inspects, or backs up the SQLite database file, no usable passwords or secrets are exposed.
2. **Synchronized Metadata and Settings**: Stored in a local, indexed SQLite database. This includes message IDs, subjects, senders, timestamps, preview snippets, AI classification results, IMAP connection parameters, and custom public Client IDs. This is just enough information to render the inbox and findings dashboard instantly.
3. **Full Content and Bodies**: Kept on the provider servers and retrieved on demand. Full MIME payloads, raw HTML markup, and non-inline attachments are never stored in local SQLite tables.

## The Hybrid Storage Model: TokenStore and Keyring

To keep database queries fast while keeping credentials safe, `Storage` wraps both an SQLite connection pool and a dedicated `TokenStore` backend.

### Primary Account Secret Unification
Each account has one primary secret stored in the OS vault keyed by its account UUID string:
* OAuth accounts store their refresh token.
* IMAP accounts store their App Password.

`TokenStore` exposes unified methods:
* `save_account_secret(account_id, secret)`
* `get_account_secret(account_id)`
* `delete_account_secret(account_id)`

This ensures that deleting an account or performing a full application wipe cleanly removes both OAuth tokens and IMAP passwords without leaving orphaned entries in the system keychain.

### Abstraction and Test Isolation
Interacting directly with the operating system keychain inside unit tests breaks automated continuous integration on headless environments (such as Linux CI runners without an active D-Bus Secret Service session).

The `TokenBackend` trait abstracts secret storage across two implementations:
* `KeyringBackend`: Production backend backed by the `keyring` crate, communicating with native OS credential vaults.
* `InMemoryBackend`: Thread-safe, mutex-guarded hash map used by `Storage::new_for_test()`.

## Database Schema and Entity Relationships

The schema is normalized to allow accounts, emails, and AI classifications to evolve independently while maintaining referential integrity.

### Tables Overview
* `linked_accounts`: Stores account configuration, provider types, email addresses, display names, sync cursors, error messages, and optional IMAP settings (`imap_host TEXT`, `imap_port INTEGER`).
* `emails`: Stores message metadata, read flags, local view state, and trash status.
* `watch_criteria`: Stores user-defined rules and descriptions used for background AI classification.
* `classifications`: Connects emails to matching criteria along with confidence scores.
* `settings`: Simple key-value storage for application-level options and public Client IDs.

### Port Typing and Validation
In SQLite, integer columns are signed 64-bit (`INTEGER`). In `LinkedAccount`, the port is represented as `pub imap_port: Option<i64>`. To avoid casting errors, `LinkedAccount` provides the `imap_port_u16(&self) -> Option<u16>` helper, safely validating that the port resides in the valid `1..=65535` range.

### UUIDv7 Primary Keys
All primary keys use UUIDv7 (`Uuid::now_v7()`).

Unlike random UUIDv4, UUIDv7 embeds a millisecond-precision Unix timestamp in its most significant bits. This makes primary keys monotonically increasing and time-ordered, eliminating B-Tree index fragmentation and maintaining write locality on disk.

### Cascading Rules
Foreign keys are strictly enforced using `PRAGMA foreign_keys = ON`:
* Deleting an account from `linked_accounts` automatically cascades to remove all of its cached emails and classifications.
* Deleting a watch criterion from `watch_criteria` automatically cascades to purge its matching records from `classifications`, clearing the dashboard findings immediately without leaving dangling records behind.

## SQLite Configuration and WAL Mode

Fumiko configures SQLite specifically for concurrent desktop usage:

1. **Write-Ahead Logging (`journal_mode = WAL`)**: Allows background sync tasks to write incoming messages without blocking UI read queries. Readers and writers operate simultaneously without lock contention.
2. **Crash-Safe Performance (`synchronous = NORMAL`)**: In traditional rollback journal mode, `NORMAL` risks corruption during operating system crashes. In WAL mode, however, `NORMAL` guarantees full crash durability while eliminating redundant disk syncs (`fsync`) on write transactions.
3. **Automatic Lock Retries (`busy_timeout = 5s`)**: If a write transaction is in progress, competing queries sleep and retry automatically for up to 5 seconds before returning an `SQLITE_BUSY` error, preventing UI lockup panics.
4. **Connection Pooling**: Uses an active pool capped at 5 connections, giving background sync and foreground UI readers dedicated channels.

## Invariants and Sync Edge Cases

### 1. Idempotent Message Upserts with RETURNING
When background sync downloads new emails, it calls `save_email`. We use SQLite's `ON CONFLICT(account_id, provider_message_id) DO UPDATE` syntax with a `RETURNING id` clause:
* If an email is re-synced, read flags are updated.
* Missing snippets or subjects are filled using `COALESCE` without resetting local flags like `app_has_viewed`.
* The `RETURNING id` expression returns the stable UUID in a single round-trip without executing a secondary `SELECT` query.

### 2. Safe Trash Transitions
During incremental sync, providers notify us of messages moved to Trash on the server. If an email was received and trashed on another device before Fumiko ever synced it, the message ID does not exist in our database. `mark_trashed` runs an `UPDATE` query without requiring rows to be affected. If the message is missing locally, the query affects zero rows and returns `Ok(())` safely.

### 3. Mailbox Capacity Pruning and Finding Protection
To prevent unbounded database growth, `purge_old_emails` enforces user-configured inbox and findings quotas.
* General Inbox Purging: Regular emails beyond the newest `max_inbox` cutoff are deleted. Critically, this query contains `AND id NOT IN (SELECT email_id FROM classifications)`. This ensures that an influx of newsletters or transactional noise never deletes high-priority findings.
* Findings Purging: If flagged findings exceed `max_findings`, older findings are pruned according to their own separate limit. Deleting from `emails` cascades to `classifications`.
* Sync Safety: Pruning old records locally does not invalidate incremental sync cursors (Gmail history IDs, Graph delta links, and IMAP UIDs remain valid).

### 4. Scoped Findings Deletion
`clear_classifications(account_id, criterion_id)` respects active filter contexts:
* If `criterion_id` is provided, only classifications for that specific watch rule are removed.
* If `account_id` is provided, only classifications for that mailbox are removed.
* If both are `None`, all findings across all mailboxes are cleared.

### 5. Keyring-First Account Deletion
In `delete_account`, `self.token_store.delete_account_secret(account_id)` is called before deleting the SQLite row. If the keyring call fails, the database row is left intact so the user can retry unlinking. This prevents orphaned secrets from lingering in the OS vault.

### 6. Automatic Credential Rollback
When saving custom Google credentials, `save_user_google_credentials` writes the client secret to the keyring first, then attempts to save the client ID in SQLite settings. If the SQLite write fails, `delete_secret` is called immediately to remove the secret from the keyring.

## Complete Data Erasure (wipe_all_data)

When a user clicks "Wipe All Local Data" in settings, data is removed thoroughly across both SQLite and the OS keyring:

1. **Keyring Wipe**: Enumerates all linked accounts and calls `delete_account_secret` for each record, removing all OAuth refresh tokens and IMAP app passwords.
2. **Custom Developer Key Cleanup**: Deletes user-configured Google OAuth client secrets from the keyring and Microsoft client IDs from settings.
3. **Atomic Database Truncation**: Wraps `DELETE FROM linked_accounts`, `DELETE FROM watch_criteria`, and `DELETE FROM settings` in an explicit transaction. Foreign key cascades purge all cached emails and classifications.
4. **Physical Disk Reclaim**: Runs `PRAGMA wal_checkpoint(TRUNCATE)` followed by `VACUUM` to physically zero out and shrink the database file on disk.

## Storage Checklist

When altering queries, adding tables, or modifying storage methods, verify these rules:

* Never add columns or settings that store raw refresh tokens, passwords, or client secrets in plain text SQLite tables.
* Always delete credentials from the OS keyring before removing corresponding database account rows.
* Use `delete_account_secret` so both OAuth tokens and IMAP passwords are wiped cleanly on account removal.
* Validate IMAP ports with `imap_port_u16` before passing them into network sockets.
* Protect findings during inbox capacity pruning by checking `id NOT IN (SELECT email_id FROM classifications)`.
* Enforce bounded query limits with `validate_limit` to prevent UI memory allocation spikes.
* Generate all primary keys using `Uuid::now_v7()` to preserve index locality.
* Ensure `PRAGMA foreign_keys = ON` remains enabled across all connection pool options.
* Keep sync operations idempotent so duplicate or out-of-order provider events never crash a sync cycle.
* Maintain `SqliteJournalMode::Wal` and `SqliteSynchronous::Normal` across connection pools.