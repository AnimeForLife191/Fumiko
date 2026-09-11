# Email Synchronization and Provider Notes

Email synchronization in Fumiko is about keeping the local inbox fast, responsive, and consistent without downloading the entire internet every time we check for mail. Email providers have completely different data models, history mechanisms, and payload structures. Fumiko translates those differences into a predictable representation that our local storage and UI can easily consume.

This document explains the architecture of the `email_core` crate, describes our synchronization lifecycle, and highlights the rules and protocol invariants we maintain across providers.

---

## What We Are Solving

A naive email client downloads entire messages (headers, plain text, HTML bodies, and multi-megabyte attachments) all at once during synchronization. That approach breaks down quickly:

* Inbox sync becomes sluggish and bandwidth-heavy on large mailboxes.
* Local databases balloon with cached message bodies the user may never open.
* Network hiccups during large sync passes cause partial writes and corrupted mailbox state.
* Differences between Google's History API and Microsoft Graph's Delta Query turn the synchronization engine into a tangled web of provider-specific conditional checks.

Our design separates synchronization into distinct stages: discovery, metadata hydration, lazy full-message retrieval, and background AI classification.

---

## The Two-Phase Sync Model: Discovery vs Hydration

Synchronization does not download message bodies by default. Instead, it operates in two distinct phases:

### Phase 1: Discovery
* Queries the provider's change-tracking endpoint (Gmail History API or Microsoft Graph Delta Query).
* Returns a lightweight summary page: new message IDs, trashed IDs, deleted IDs, restored IDs, and the next sync cursor.
* Internal Pagination: If a sync pass contains multiple pages of changes, the provider exhausts the intermediate page tokens internally before returning a consolidated `SyncPage`.

### Phase 2: Hydration
* Fetches lightweight metadata (Subject, From, Date, Snippet, and Read Status) for newly discovered message IDs.
* Requests run in bounded concurrent batches (10 at a time) to respect provider burst rate limits.
* Headers and preview snippets are saved to SQLite.

By requesting only summary headers along with the preview snippet during inbox sync, we can synchronize hundreds of new emails in seconds with minimal network overhead. 

The full MIME payload (the HTML markup, plain-text fallback, attachment lists, and inline images) is deferred until the user clicks on a message to view it, with a small exception for quota-capped Tier 2 AI classification on ambiguous emails.

---

## Cursors and State Tracking

To avoid re-scanning the entire inbox on every sync cycle, Fumiko uses incremental synchronization cursors. Because providers track mailbox state differently, the `SyncCursor` type encapsulates whatever token the provider needs to resume from its last known state.

### Gmail: History IDs
Google tracks mailbox changes using a monotonically increasing numeric `historyId`.
* **Initial Sync**: We list the latest message IDs in the user's `INBOX` up to `max_results` and query `users.getProfile` to record the current baseline `historyId`.
* **Incremental Sync**: We query `/users/me/history?startHistoryId={cursor}` for changes (`messagesAdded`, `messagesDeleted`, `labelsAdded`, `labelsRemoved`). Moving a message to Trash or Spam in Gmail is reported as a label addition (`TRASH` or `SPAM`), not a deletion.
* **Spam and Trash Restorations**: If a user un-trashes or un-spams an email in Gmail, Google emits a `labelsRemoved` event for `TRASH` or `SPAM`. Fumiko catches both and restores the message locally.
* **Cursor Expiration**: Gmail prunes history records after a retention window (often a few days or weeks). When a cursor is too old, Gmail returns an HTTP 404 Not Found. We map this to `ProviderError::CursorExpired`, which tells `SyncService` to discard the stale cursor and run a fresh initial sync.

### Outlook: Graph Delta Links
Microsoft Graph tracks folder changes via delta queries (`/me/mailFolders/inbox/messages/delta`).
* **Initial Sync**: Graph returns results across multiple pages. Intermediate pages contain an `@odata.nextLink` URL, while the final page contains an `@odata.deltaLink`. We follow all `nextLink` pages during the initial pass to obtain that `deltaLink`. Saving anything earlier would leave the sync engine without a valid delta cursor.
* **Incremental Sync**: The stored `SyncCursor` is the full `@odata.deltaLink` URL itself. Requesting that URL returns only changes that occurred since that link was minted. Graph reports folder moves and purges with an `@removed` annotation.
* **Cursor Expiration**: If a delta link becomes invalid or expires, Graph responds with an HTTP 410 Gone. We map this to `ProviderError::CursorExpired` to trigger a clean re-sync baseline.

---

## Invariant: Safe Cursor Advancement

A critical bug in many custom email clients is the silent loss of messages caused by premature cursor updates. 

If we discover 50 new message IDs, but the network drops while fetching metadata for message 30, we must not advance the sync cursor in SQLite. Advancing the cursor past this batch means the provider will never report those 50 messages as new again, permanently hiding messages 30 through 50 from the user.

In `SyncService`:
1. Metadata is fetched for all newly discovered IDs.
2. If any message metadata fails to load, a warning is logged and an internal error flag (`fetch_errors_occurred`) is raised.
3. The sync cursor in SQLite is updated only if all items in the batch were successfully hydrated.
4. If a partial failure occurs, the cursor remains at its previous position, allowing the next sync cycle to re-discover and fetch the missing messages cleanly.

---

## Account Linking and Token Safety

Linking an account coordinates OAuth token exchange, profile discovery, and initial storage registration:

* **Refresh Token Requirement**: We verify that a valid `refresh_token` exists in the provider response before writing the account row to SQLite.
* **Keyring Rollback**: The refresh token is saved immediately into the operating system keyring. If keyring storage fails, the newly created account row is rolled back and deleted.
* **Proactive Expiry Buffering**: Access tokens are cached in memory with a safety margin (50 minutes instead of the standard 60-minute expiry) to prevent requests from racing with token expiration.
* **Automatic Retry on 401**: If an API request fails with `ProviderError::Unauthorized` (HTTP 401), the sync engine automatically invalidates the memory cache, triggers an OAuth refresh flow with the provider, and retries the sync pass once before giving up.

---

## Rate Limiting and Backoff

Cloud email APIs enforce strict rate limits:

* **Microsoft Graph 429 Handling**: Graph API frequently throttles bursts with HTTP 429 Too Many Requests. `OutlookProvider` wraps requests in `send_request_with_retry`. It inspects the standard `Retry-After` HTTP header and falls back to capped exponential backoff (`2_u64.pow(attempts)`) before retrying.
* **Bounded Concurrency**: All batch metadata operations limit concurrent requests to 10 via `buffer_unordered(10)` to stay well below Google and Microsoft per-second request ceilings.

---

## Message Parsing, Encodings, and Inline Images

Email formatting is historically messy. Providers deliver bodies and attachments in completely different structures:

### MIME Trees vs Derived Bodies
* **Gmail**: Delivers messages as recursive MIME trees (`MessagePartFull`). We recursively walk the tree to find `text/plain` and `text/html` parts. If an email is HTML-only (no `text/plain` part), we run the HTML through `html2text` to derive a clean plain-text fallback. This guarantees that both views are populated and prevents raw HTML markup from flooding local AI prompts. Large bodies offloaded to attachment storage (`attachmentId`) are fetched transparently.
* **Outlook**: Returns a single body representation (HTML by default). We accept the HTML payload and derive the plain-text fallback locally using `html2text` without needing a second API call.

### Charsets and Base64 Quirks
Gmail message parts are encoded in URL-safe Base64 and can use arbitrary legacy charsets (like ISO-8859-1 or Windows-1252):
* We decode the Base64 data with a fallback for unpadded strings (`URL_SAFE_NO_PAD` falling back to `URL_SAFE`).
* We extract the charset from the `Content-Type` header and decode raw bytes through `encoding_rs` so invalid byte sequences are safely replaced rather than panicking.

### Case-Insensitive Inlining of CID Images
HTML emails frequently reference embedded images using `src="cid:image_identifier"`:
* For Gmail, we collect all parts containing a `Content-ID` header.
* For Outlook, we query `/attachments` filtered by `isInline eq true`.
* The image bytes are converted into Base64 data URIs.
* To handle variations across different email clients, substitution uses a case-insensitive regex pattern: `(?i)cid:{escaped_id}`. This ensures uppercase references like `CID:image001.png` are replaced properly and display without broken image icons.

---

## Background AI Classification Gating

Fumiko includes an automated classification engine running against local Ollama models. 

To maximize accuracy without exhausting API quotas or starving CPU resources, classification uses a two-tier evaluation model:

### 1. Tier 1: Fast Metadata and Snippet Evaluation
Every incoming email is first evaluated using its subject, sender address, and server-provided snippet preview. Because snippets are already downloaded during metadata hydration, Tier 1 incurs zero additional network requests.

### 2. Ambiguity Gating for Tier 2 Promotion
* **High Confidence (>= 0.50)**: The email is cleanly classified in Tier 1 and saved immediately.
* **Clear Non-Match (< 0.25)**: The email is discarded without fetching the body.
* **Ambiguous Match (0.25 to 0.49)**: The email qualifies for Tier 2 body inspection.

### 3. API Quota and Memory Protections
* **Initial Sync Exclusion**: Tier 2 is disabled during the first initial sync pass (`account.sync_cursor` is None). Initial batches run strictly on Tier 1.
* **Bounded Fetch Budget**: A thread-safe atomic counter limits full-body downloads to at most 3 messages per sync pass (`MAX_TIER2_FETCHES_PER_SYNC`).
* **Bounded Inferences**: Classification streams up to 4 inferences in parallel (`for_each_concurrent(4, ...)`).
* **Hallucination Guards**: Model outputs are matched against active database criteria, and unknown or hallucinated labels are discarded.

---

## Provider and Synchronization Checklist

When adding new provider features, modifying sync logic, or touching message parsing, ensure these rules remain intact:

* Never advance the cursor on partial hydration failure: all messages in a discovered batch must be successfully saved before updating `sync_cursor` in SQLite.
* Follow delta pages to completion: never assume a single page contains `@odata.deltaLink` during Outlook initial sync; always exhaust `@odata.nextLink`.
* Validate refresh tokens before account persistence: verify the account can refresh in the background before committing it to storage.
* Respect 429 backoff headers: parse `Retry-After` on Graph API responses before retrying requests.
* Maintain plain-text parity: ensure HTML-only emails in both Gmail and Outlook derive a plain-text fallback via `html2text` for local AI prompts.
* Substitute CID images case-insensitively: use `(?i)cid:` to catch uppercase content identifier references.
* Handle cursor expiration gracefully: map provider history purges (HTTP 404 and 410) to `ProviderError::CursorExpired` and trigger a fresh initial sync.
* Isolate token refresh retries: catch `Unauthorized` errors, invalidate the memory cache, refresh credentials, and retry once.
* Use non-panicking text decoders: run all external message bytes through `encoding_rs` and safe Base64 decoders.
* Bound classification concurrency: never dispatch unthrottled concurrent requests against the local AI model.