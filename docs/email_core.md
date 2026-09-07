# Email Synchronization and Provider Notes

Email synchronization in Fumiko is mostly about keeping the local mailbox fast, responsive, and consistent without downloading the entire internet every time we check for new mail. Email providers have completely different data models, history mechanisms, and payload structures. Fumiko translates those differences into a predictable, provider-agnostic representation that our local storage and UI can easily consume.

This document explains the architecture of the `email_core` crate, describes how our synchronization lifecycle works, and highlights the rules and protocol invariants we need to maintain as the application grows.

## What we are solving

A naive email client downloads entire messages (headers, plain text, HTML bodies, and multi-megabyte attachments) all at once during synchronization. That approach falls apart quickly:

* Inbox sync becomes sluggish and bandwidth-heavy on large mailboxes.
* Mobile and desktop databases balloon with cached message bodies the user may never open.
* Network hiccups during large sync passes cause partial writes and corrupted mailbox state.
* Differences between Google's History API and Microsoft Graph's Delta Query turn the synchronization engine into a tangled web of provider-specific if/else checks.

Our design separates synchronization into distinct stages: discovery, metadata hydration, lazy full-message retrieval, and background AI classification.

## The two-phase sync model: Discovery vs. Hydration

Synchronization does not download message bodies by default. Instead, it operates in two separate phases:

Phase 1: Discovery
Query Provider (Delta Query or History API)
Returns: New IDs, Trashed IDs, Deleted IDs, Restored IDs, Next Cursor

Phase 2: Hydration
Fetch Lightweight Metadata (Subject, From, Date, Snippet, Read Status)
Persist headers to local storage in bounded concurrent batches (10 at a time)

By requesting only summary headers (`Subject`, `From`, `internalDate`, and read status) along with the preview snippet during inbox sync, we can synchronize hundreds of new emails in seconds with minimal network overhead. 

The full MIME payload (the HTML markup, plain-text fallback, attachment lists, and inline images) is deferred until the user clicks on a message to view it (`fetch_full_message`), with a small exception for quota-capped Tier 2 AI classification on ambiguous emails.

## Cursors and state tracking

To avoid re-scanning the entire inbox on every sync pass, Fumiko uses incremental synchronization cursors. Because providers represent mailbox state differently, the `SyncCursor` type encapsulates whatever token the provider needs to resume from its last known state.

### Gmail: History IDs

Google tracks changes using a monotonically increasing `historyId`. 
* Initial Sync: We list the latest message IDs in the user's `INBOX` up to `max_results` and query `users.getProfile` to record the current baseline `historyId`.
* Incremental Sync: We query `/users/me/history?startHistoryId={cursor}` for events (`messagesAdded`, `messagesDeleted`, `labelsAdded`, `labelsRemoved`). Moving a message to Trash or Spam in Gmail is reported as a label addition (`TRASH` or `SPAM`), not a deletion.
* Cursor Expiration: Gmail prunes history records after a certain retention window (often a few days or weeks). When a cursor is too old, Gmail returns an HTTP 404 Not Found. We map this to `ProviderError::CursorExpired`, which tells `SyncService` to discard the stale cursor and perform a fresh initial sync.

### Outlook: Graph Delta Links

Microsoft Graph tracks folder changes via delta queries (`/me/mailFolders/inbox/messages/delta`).
* Initial Sync: Graph returns results across multiple pages. Intermediate pages contain an `@odata.nextLink` URL, while the final page contains an `@odata.deltaLink`. We must traverse all `nextLink` pages during the initial pass to obtain that `deltaLink`. Saving anything earlier will leave the sync engine without a valid delta cursor.
* Incremental Sync: The stored `SyncCursor` is the full `@odata.deltaLink` URL itself. Requesting that URL returns only the changes that occurred since that link was minted. Graph reports moves to Deleted Items and permanent purges with an `@removed` annotation.
* Cursor Expiration: If a delta link becomes invalid or expires, Graph responds with an HTTP 410 Gone. We map this to `ProviderError::CursorExpired` to trigger a clean re-sync baseline.

## Invariant: Safe cursor advancement

A critical bug in many email clients is the silent loss of messages caused by premature cursor updates. 

If we discover 50 new message IDs, but the network drops while fetching metadata for message 30, we must not advance the sync cursor in SQLite. Advancing the cursor past this batch means the provider will never report those 50 messages as new again, permanently hiding messages 30 through 50 from the user's inbox.

In `SyncService`:
1. Metadata is fetched for all newly discovered IDs.
2. If any message metadata fails to load, a warning is logged and an internal error flag is raised.
3. The sync cursor in the local database is updated only if all items in the batch were successfully hydrated.
4. If a partial failure occurs, the cursor remains at its previous position, allowing the next sync cycle to re-discover and fetch the missing messages cleanly.

## Account linking and token safety

Linking an account involves an OAuth 2.0 handshake, token exchange, and initial storage registration. 

To prevent broken or un-syncable accounts from polluting the database:
* We check that a valid `refresh_token` exists in the provider response before writing the account row to SQLite.
* The refresh token is saved immediately into the operating system keyring. If keyring storage fails, the newly created account row is rolled back and deleted.
* Access tokens are cached in memory with a safety margin (50 minutes instead of the standard 60-minute expiry) to prevent requests from racing with token expiration.
* If an API request fails with `ProviderError::Unauthorized` (HTTP 401), the sync engine automatically invalidates the memory cache, triggers an OAuth refresh flow with the provider, and retries the sync pass once before giving up.

## Message parsing, encodings, and inline images

Email formatting is historically messy. Different providers deliver bodies and attachments in completely different structures:

### MIME Trees vs. Derived Bodies

* Gmail delivers messages as recursive MIME trees (`MessagePartFull`). We recursively walk the tree to find `text/plain` and `text/html` parts. Gmail occasionally moves large message bodies out of the inline payload into attachment storage (`body.attachment_id`), requiring an extra fetch step which our parser handles transparently.
* Outlook returns a single body representation (HTML by default). To populate both views without a second network roundtrip, we accept the HTML payload and derive the plain-text fallback locally using `html2text`.

### Charsets and Base64 Quirks

Gmail message parts are encoded in URL-safe Base64 and can use arbitrary legacy charsets (like ISO-8859-1 or Windows-1252). 
* We decode the Base64 data with a fallback for unpadded strings.
* We detect the charset from the `Content-Type` header and decode the raw bytes through `encoding_rs` so invalid byte sequences are safely replaced rather than panicking or producing garbage text.

### Inlining CID Images

HTML emails frequently reference embedded images using `src="cid:image_identifier"`. 
* For Gmail, we collect all parts containing a `Content-ID` header.
* For Outlook, we query `/attachments` filtered by `isInline eq true`.
* The image bytes are converted into base64 data URIs and substituted into the HTML body. This allows our UI webviews to render complete emails with inline graphics without needing authenticated local proxy servers or custom image loaders.

## Background AI Classification and Tier 2 Gating

Fumiko includes an automated classification engine running against local Ollama models. 

To maximize accuracy without exhausting Google/Microsoft API quotas or starving local CPU resources, classification uses a two-tier promotion model:

### 1. Tier 1: Fast Metadata and Snippet Evaluation

Every new incoming email is first evaluated using its subject, sender address, and server-provided snippet preview. Because snippets are already downloaded during metadata hydration, Tier 1 incurs zero additional API requests.

### 2. Ambiguity Gating for Tier 2 Promotion

* High Confidence (>= 0.50): The email is cleanly classified in Tier 1 and saved immediately.
* Clear Non-Match (< 0.25): The email is discarded without fetching the body.
* Ambiguous Match (0.25 to 0.49): The email qualifies for Tier 2 body inspection.

### 3. API Quota and Concurrency Protections

To protect API limits and system memory during Tier 2 promotion:
* Initial Sync Exclusion: Tier 2 is disabled during the first initial sync pass (`account.sync_cursor` is None). Initial 100-email batches run strictly on Tier 1.
* Bounded Fetch Budget: A thread-safe atomic budget limits full-body downloads to at most 3 messages per sync pass (`MAX_TIER2_FETCHES_PER_SYNC`).
* Bounded Inferences: Classification streams up to 4 concurrent inferences in parallel (`for_each_concurrent(4, ...)`).
* Hallucination Guards: Model outputs are matched against active database criteria, and hallucinated or unlisted labels are discarded.

## Provider and synchronization checklist

When adding new provider features, modifying sync logic, or touching message parsing, ensure these invariants remain intact:

* Never advance the cursor on partial hydration failure: All messages in a discovered batch must be successfully saved before updating `sync_cursor` in the database.
* Follow delta pages to completion: Never assume a single page contains `@odata.deltaLink` during Outlook initial sync; always exhaust `@odata.nextLink`.
* Validate refresh tokens before account persistence: Ensure the account can be refreshed in the background before committing it to storage.
* Keep inbox sync lightweight: Never fetch full message bodies or non-inline attachment bytes during initial sync.
* Enforce Tier 2 fetch caps: Always gate full-body classification behind initial-sync checks, ambiguity thresholds, and strict batch budgets.
* Handle cursor expiration gracefully: Map provider history purges (404 and 410) to `ProviderError::CursorExpired` and trigger a fresh initial sync.
* Isolate token refresh retries: Catch `Unauthorized` errors, invalidate the memory cache, refresh credentials, and retry once.
* Use non-panicking text decoders: Run all external message bytes through `encoding_rs` and safe Base64 decoders.
* Bound classification concurrency: Never run unthrottled concurrent requests against the local AI model.