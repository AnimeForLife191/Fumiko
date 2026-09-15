# Email Synchronization and Provider Notes

Email synchronization in Fumiko keeps the local inbox fast, responsive, and consistent without downloading unnecessary message data during routine sync passes. Email providers use different data models, history tracking mechanisms, and payload formats. Fumiko normalizes these differences into a unified representation for local storage and the desktop UI.

This document explains the architecture of the `email_core` crate, describes the two-phase sync model, and details the protocol invariants maintained across providers.

## The Two-Phase Sync Model: Discovery vs Hydration

To minimize bandwidth and avoid database bloat, routine synchronization does not download message bodies. It operates in two distinct phases:

### Phase 1: Discovery
* Queries the provider change-tracking endpoint (Gmail History API, Microsoft Graph Delta Query, or IMAP UID search).
* Returns a lightweight summary page: new message IDs, trashed IDs, deleted IDs, restored IDs, and the next sync cursor.
* Internal Pagination: If a sync pass contains multiple pages of changes, the provider exhausts intermediate page tokens internally before returning a consolidated `SyncPage`.

### Phase 2: Hydration
* Fetches lightweight metadata (Subject, From, Date, Snippet, and Read Status) for newly discovered message IDs.
* REST API requests (Gmail and Outlook) run in bounded concurrent batches (10 at a time via `buffer_unordered(10)`) to respect provider burst ceilings.
* IMAP requests batch multiple UIDs into a single comma-separated `UID FETCH` command to avoid duplicate socket handshakes.
* Summary headers and scrubbed preview snippets are saved to SQLite.

The full MIME payload (HTML markup, plain-text fallback, and attachment lists) is retrieved on demand when the user opens an email in the reading pane, with a quota-capped exception for Tier 2 AI evaluation on ambiguous emails.

## Cursors and Mailbox State Tracking

Incremental synchronization cursors prevent re-scanning the entire inbox on every sync pass. The `SyncCursor` type encapsulates whatever token the provider requires to resume from its last known state:

### Gmail: History IDs
Google tracks mailbox changes using a monotonically increasing numeric `historyId`:
* Initial Sync: Queries `INBOX` up to `max_results` and calls `users.getProfile` to record the current baseline `historyId`.
* Incremental Sync: Queries `/users/me/history?startHistoryId={cursor}` for changes (`messagesAdded`, `messagesDeleted`, `labelsAdded`, `labelsRemoved`). Moving a message to Trash or Spam in Gmail is reported as a label addition (`TRASH` or `SPAM`), not a deletion.
* Spam and Trash Restorations: If a user un-trashes or un-spams an email in Gmail, Google emits a `labelsRemoved` event for `TRASH` or `SPAM`. Fumiko catches both and restores the message locally.
* Cursor Expiration: Gmail prunes history records after a retention window (typically several days or weeks). When a cursor is too old, Gmail returns an HTTP 404 Not Found. Fumiko maps this to `ProviderError::CursorExpired`, instructing `SyncService` to clear the stale cursor and perform a fresh initial sync.

### Outlook: Graph Delta Links
Microsoft Graph tracks folder changes via delta queries (`/me/mailFolders/inbox/messages/delta`):
* Initial Sync: Graph returns results across multiple pages. Intermediate pages contain an `@odata.nextLink` URL, while the final page contains an `@odata.deltaLink`. Fumiko follows all `nextLink` pages to obtain that final `deltaLink`. Saving an earlier link would leave the sync engine without a valid delta cursor.
* Incremental Sync: The stored `SyncCursor` is the full `@odata.deltaLink` URL itself. Requesting that URL returns only changes that occurred since that link was issued. Graph reports folder moves and purges with an `@removed` annotation.
* Cursor Expiration: If a delta link becomes invalid or expires, Graph responds with an HTTP 410 Gone. Fumiko maps this to `ProviderError::CursorExpired` to trigger a clean re-sync baseline.

### IMAP: UIDVALIDITY and UIDs
Unlike HTTP REST APIs, IMAP (RFC 3501) identifies messages inside folders using unique identifiers (UIDs):
* UIDs vs Sequence Numbers: Fumiko strictly uses UIDs (`UID SEARCH`, `UID FETCH`). Message sequence numbers shift dynamically whenever an email is deleted or expunged by another client, which can lead to reading or corrupting the wrong message. UIDs remain stable for the lifetime of the mailbox.
* The Composite Cursor: IMAP cursors are formatted as `{uid_validity}:{last_seen_uid}`.
* UIDVALIDITY Invariant: Every IMAP mailbox provides a `UIDVALIDITY` number. If the server rebuilds its index or renumbers messages, `UIDVALIDITY` changes. When Fumiko detects a `UIDVALIDITY` mismatch during incremental sync, it returns `ProviderError::CursorExpired`, discarding previous UIDs and triggering a fresh initial sync.
* Initial Sync: Selects `INBOX`, obtains `UIDVALIDITY`, runs `UID SEARCH ALL`, sorts the returned UIDs, takes the highest `max_results` messages, and records `{uid_validity}:{highest_uid}` as the cursor.
* Incremental Sync: Evaluates `UID SEARCH UID {last_seen_uid + 1}:*`. Any UIDs greater than `last_seen_uid` are treated as new messages, and the cursor advances to the new highest UID.

## Invariant: Safe Cursor Advancement

A critical failure mode in custom email clients is the silent loss of messages caused by premature cursor updates.

If discovery finds 50 new message IDs, but the network drops while hydrating metadata for message 30, the sync cursor in SQLite must not advance. Advancing the cursor past this batch means the provider will never report those 50 messages as new again, permanently hiding messages 30 through 50 from the user.

In `SyncService`:
1. Metadata is fetched for all newly discovered IDs.
2. If any message metadata fails to load due to network or server errors, a warning is logged and `fetch_errors_occurred` is set to true.
3. If an individual message returns 404 (indicating it was deleted on the server between discovery and hydration), it is safely skipped without raising `fetch_errors_occurred`.
4. The sync cursor in SQLite is updated only if all items in the batch were successfully processed.
5. If a partial failure occurs, the cursor remains at its previous position, allowing the next sync pass to re-discover and fetch the missing messages cleanly.

## Account Linking and Token Safety

Linking an account coordinates authentication verification, profile discovery, and initial storage registration:

* OAuth Accounts: Verifies that a valid `refresh_token` exists in the provider response before writing the account row to SQLite.
* IMAP Accounts: Establishes a live TLS connection and executes an IMAP `LOGIN` to verify credentials before committing records to storage.
* Keyring Rollback: The account secret (OAuth refresh token or IMAP app password) is saved into the operating system keyring. If keyring storage fails, the newly created account row in SQLite is rolled back and deleted.
* Proactive Expiry Buffering: Active credentials are held in memory with a 50-minute TTL to avoid keeping plain secrets permanently pinned in RAM.
* Automatic Retry on 401: If an API request fails with `ProviderError::Unauthorized` (HTTP 401 or IMAP auth failure), the sync engine invalidates the memory cache, refreshes credentials, and retries the sync pass once before failing.

## Connection Lifecycles and Rate Limiting

Providers enforce different traffic limits:

* Microsoft Graph 429 Handling: Graph throttles bursts with HTTP 429 Too Many Requests. `OutlookProvider` wraps requests in `send_request_with_retry`. It inspects the `Retry-After` HTTP header and falls back to capped exponential backoff (`2_u64.pow(attempts)`) before retrying up to 3 times.
* Bounded REST Concurrency: Batch metadata operations for Gmail and Graph limit concurrent requests to 10 via `buffer_unordered(10)` to stay below provider rate ceilings.
* IMAP Socket Timeouts and Batching: IMAP servers strictly limit concurrent sockets per account. Rather than opening parallel sockets, `ImapProvider` connects a single TLS socket per sync pass, batches requested UIDs into a single comma-separated command (`UID FETCH id1,id2,...`), and enforces explicit 10-second connect and 15-second read/write timeouts.

## Message Parsing, Encodings, and Inline Images

### MIME Trees vs Derived Bodies
* Gmail: Delivers messages as recursive MIME trees (`MessagePartFull`). We recursively search the tree for `text/plain` and `text/html` parts. If an email is HTML-only (no `text/plain` part), we pass the HTML to `html2text` to derive a clean plain-text fallback. Large bodies offloaded to attachment storage (`attachmentId`) are fetched transparently.
* Outlook: Returns a single body representation (HTML by default). We accept the HTML payload and derive the plain-text fallback locally using `html2text`.
* IMAP: Delivers raw RFC 822 MIME byte streams parsed with `mail_parser`. If the email contains a plain-text part, it is loaded directly. If the email is HTML-only, it is converted via `html2text`.

### The IMAP BODY.PEEK Invariant
In IMAP, issuing a standard `FETCH ... RFC822` or `FETCH ... BODY[]` command causes the server to automatically mark the message as `\Seen` (read).

Because Fumiko runs Tier 2 AI classification in the background during inbox sync, fetching an ambiguous email with standard `BODY[]` would cause the background AI to mark unread messages as read before the user ever opened them.

To prevent this, `ImapProvider` strictly uses `BODY.PEEK[]` when fetching full message payloads, and `BODY.PEEK[TEXT]<0.500>` when sampling headers, preserving the unread status of user mail.

### Snippet MIME Boundary Scrubbing
When sampling metadata over IMAP (`BODY.PEEK[TEXT]<0.500>`), multipart emails return raw MIME boundaries (such as `--boundary_123` and `Content-Type: text/plain`). `ImapProvider` scrubs boundary prefixes and transfer encoding headers from the preview snippet before saving it to SQLite.

### Charsets and Base64 Quirks
Gmail message parts are encoded in URL-safe Base64 and can use legacy charsets:
* We decode Base64 data with a fallback for unpadded strings (`URL_SAFE_NO_PAD` falling back to `URL_SAFE`).
* We extract the charset from the `Content-Type` header and decode raw bytes through `encoding_rs` so invalid byte sequences are safely replaced rather than panicking.

### Case-Insensitive Inlining of CID Images
HTML emails frequently reference embedded images using `src="cid:image_identifier"`:
* For Gmail, we collect all parts containing a `Content-ID` header.
* For Outlook, we query `/attachments?$filter=isInline eq true`.
* For IMAP, we iterate over MIME attachment parts containing a `Content-ID`.
* Image bytes are converted into Base64 data URIs.
* Substitution uses a case-insensitive regex pattern: `(?i)cid:{escaped_id}`, ensuring uppercase references like `CID:image001.png` render properly.

## Background AI Classification Gating

Fumiko includes an automated classification engine running against on-device AI models (using either the zero-setup built-in sidecar engine or an external Ollama daemon).

To maximize accuracy without exhausting provider quotas or starving local compute, classification uses a two-tier evaluation model:

### 1. Tier 1: Fast Metadata and Snippet Evaluation
Every incoming email is first evaluated using its subject, sender address, and server-provided snippet preview. Because snippets are downloaded during Phase 2 metadata hydration, Tier 1 incurs zero additional network requests.

### 2. Ambiguity Gating for Tier 2 Promotion
* High Confidence (>= 0.50): The email is cleanly classified in Tier 1, saved to the findings board, and triggers a desktop notification.
* Clear Non-Match (< 0.25): The email is discarded without fetching the body.
* Ambiguous Match (0.25 to 0.49): The email qualifies for Tier 2 deep body inspection.

### 3. API Quota and Memory Protections
* Initial Sync Exclusion: Tier 2 is disabled during the first initial sync pass (`is_initial_sync == true`). Initial batches run strictly on Tier 1 to prevent large initial body downloads.
* Bounded Fetch Budget: A thread-safe atomic counter limits full-body downloads to at most 3 messages per sync pass (`MAX_TIER2_FETCHES_PER_SYNC`).
* Bounded Inferences: Classification dispatches up to 4 concurrent HTTP requests in parallel (`for_each_concurrent(4, ...)`), pipelining into the single-slot inference queue.
* Hallucination Guards: Model outputs are matched against active database criteria, and unknown or hallucinated labels are discarded.

## Provider and Synchronization Checklist

When adding new provider features, modifying sync logic, or touching message parsing, ensure these rules remain intact:

* Never advance the cursor on partial hydration failure: all messages in a discovered batch must be successfully saved before updating `sync_cursor` in SQLite.
* Tolerate missing server messages during hydration: skip 404 messages without holding back cursor advancement.
* Follow delta pages to completion: never assume a single page contains `@odata.deltaLink` during Outlook initial sync; always exhaust intermediate `@odata.nextLink` pages.
* Validate UIDVALIDITY on IMAP: if `UIDVALIDITY` changes between sync cycles, map it to `ProviderError::CursorExpired` and trigger a fresh baseline sync.
* Never use RFC822 or BODY[] during IMAP sync: always use `BODY.PEEK` so background AI evaluation does not mark unread emails as read.
* Validate refresh tokens or passwords before account persistence: verify credentials against the provider before committing records to storage.
* Respect 429 backoff headers: parse `Retry-After` on Graph API responses before retrying requests.
* Maintain plain-text parity: ensure HTML-only emails in Gmail, Outlook, and IMAP derive a plain-text fallback via `html2text` for local AI prompts.
* Set explicit socket timeouts on IMAP: configure connect and read/write timeouts on TCP streams before wrapping with TLS.
* Substitute CID images case-insensitively: use `(?i)cid:` to catch uppercase content identifier references.
* Handle cursor expiration gracefully: map provider history purges (HTTP 404, HTTP 410, and IMAP validity changes) to `ProviderError::CursorExpired` and trigger a fresh initial sync.
* Isolate token refresh retries: catch `Unauthorized` errors, invalidate the memory cache, refresh credentials, and retry once.
* Use non-panicking text decoders: run all external message bytes through `encoding_rs` and safe Base64 decoders.
* Bound classification concurrency: never dispatch unthrottled concurrent requests against the local AI model.