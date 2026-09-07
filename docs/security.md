# Security Notes

Security in Fumiko is mostly about limiting what the application can receive, where sensitive values can be stored, and which requests the system is willing to trust. OAuth handles the user password and remote sign-in session. Fumiko should only receive the minimum permissions and tokens it needs for the email features the user chose.

This document describes the security model for the desktop application, explains our threat boundaries, and highlights the rules that must remain in place as the project grows.

## What we are protecting

The most sensitive values in the application lifecycle are:

* The authorization code returned by the provider
* The PKCE verifier and CSRF state token for active sign-in attempts
* Short-lived access tokens held in memory
* Long-lived refresh tokens stored on disk
* User-configured OAuth client secrets
* Email message content, headers, and attachments returned by providers

A nearby local process, a compromised log file, or an accidental database backup should not be enough to obtain long-lived access to a user mailbox.

No design can protect a machine that is already fully compromised at the root level. The goal is to reduce exposure, reject forged or unrelated callbacks, prevent timing leaks, and avoid turning ordinary database or log files into credential dumps.

## Loopback callback and local network boundaries

The desktop OAuth flow listens strictly on `127.0.0.1` using a fresh, operating system-assigned ephemeral port for every sign-in attempt. It is not an internet-facing server and must never bind to `0.0.0.0`.

### Socket timeouts and connection isolation

Because the listener runs locally, rogue local processes or port scanners can open connections to the ephemeral port. To prevent these connections from hanging the listener:

* Accepted TCP sockets enforce a strict 2-second read and write timeout. A connection that sends no data will not block the listener thread indefinitely.
* The listener parses only the first HTTP request line and checks the exact expected callback path. Unrelated requests (such as browser favicon requests) receive a 404 response and are dropped without terminating the sign-in attempt.
* The listener response page is a static confirmation message. It never echoes authorization codes, tokens, client secrets, or state values back to the browser.
* Sockets are explicitly flushed and cleanly shut down before closing to prevent browser connection reset errors.

### Numeric IP routing over localhost

The callback listener binds to `127.0.0.1` and constructs redirect URIs using the explicit IPv4 loopback address. Many operating systems resolve the word `localhost` to the IPv6 address `::1` first. Using `127.0.0.1` prevents modern browsers from failing with connection refused errors when attempting IPv6 loopback routing.

## Cryptographic protections in transit

### PKCE protects the code exchange

Proof Key for Code Exchange (PKCE) creates a cryptographically random verifier and a corresponding SHA-256 challenge. The challenge travels with the initial browser authorization request, while the private verifier stays in memory.

When Fumiko exchanges the authorization code for tokens, the provider requires the original verifier. Even if another local process intercepts the authorization code from the browser redirect, the code alone is useless without the private verifier. PKCE is generated for every authorization flow, including confidential flows that use a client secret.

### Constant-time CSRF state validation

Each authorization request creates a random state token. The provider echoes that value back with the redirect, and Fumiko validates it before accepting the authorization code.

State comparison uses constant-time byte validation rather than standard short-circuiting string comparisons. This eliminates microarchitectural timing side channels that could leak state token bytes. State tokens are single-use, request-specific, and never written to persistent logs.

## Credential storage and secret lifecycle

### Access tokens in memory

Access tokens are short-lived. They remain in memory only, cached with a 50-minute proactive time-to-live. They are never written to SQLite, persistent cache files, URLs, or log messages.

### Refresh tokens in the operating system keyring

Refresh tokens last longer and require stronger protection. Fumiko stores them exclusively in the native operating system credential vault (Apple Keychain, Windows Credential Manager, or Linux Secret Service), keyed by the account UUID.

* Refresh tokens are never stored in SQLite database tables.
* When an account is unlinked or deleted, its keyring entry is deleted first before removing the database row.
* If a provider returns a replacement refresh token during token rotation, the keyring entry is updated. If the provider omits a replacement, the existing token is preserved.

### Redacted debug formatting

Data structures that hold credentials (`RawCredentials` and `TokenSet`) implement custom `fmt::Debug` formatters that explicitly replace secrets with `[REDACTED]`. This guarantees that diagnostics or error logging macros cannot accidentally dump plaintext tokens into log files.

## Build-time secrets and development configuration

Google requires a client secret for our desktop configuration, while Microsoft uses a public client.

* Development credentials are encrypted into the binary at compile time using `envcrypt::option_envc!`. They do not sit in plaintext inside the executable.
* Directives emitted by `cargo:rustc-env` in a `build.rs` script apply only to that specific crate. They do not leak into upstream or downstream dependencies. Every crate calling `option_envc!` must maintain its own `build.rs` pointed at the gitignored `.env` file.
* User-provided credentials entered through the application settings take precedence over bundled development values and are stored securely (client IDs in SQLite settings, client secrets in the OS keyring).

## Synchronization, storage, and AI safety

### Safe cursor advancement

When synchronizing new mail, the sync cursor in SQLite is updated only if all message metadata in that batch was successfully retrieved and saved. If network issues cause a partial metadata fetch failure, the cursor is not advanced. This ensures that transient network drops never permanently skip unread messages.

### Idempotent database operations

Database queries are written defensively against duplicate or out-of-order provider events:

* `save_email` uses `ON CONFLICT DO UPDATE` to safely update read flags and missing snippets without crashing on duplicate message notifications.
* `mark_trashed` treats unsaved messages as safe no-ops rather than throwing not-found errors, preventing synchronization from crashing on messages that were trashed on another device before being synced locally.
* Database limits and trash retention periods are bounded to prevent integer overflow and memory exhaustion.

### Message body and charset safety

Email bodies can contain arbitrary legacy character encodings. All raw body bytes are decoded through `encoding_rs` to replace invalid byte sequences safely rather than panicking.

Inline images referenced by Content-ID are converted into self-contained base64 data URIs. This allows local webviews to render emails with images without running an unauthenticated local HTTP proxy server.

### Local AI classification boundaries

When running background email classification through local Ollama models:

* Bounded Concurrency: Inferences run with bounded concurrency (up to 4 in parallel) to prevent CPU and GPU starvation.
* Hallucination Guards: Model output is strictly validated against the active database criteria list. If the model hallucinates an unknown label, the result is discarded.
* Confidence Thresholds: Classifications below 0.50 confidence are rejected to avoid miscategorizing user mail.

## Failure shapes and error reporting

The application distinguishes several different failure shapes rather than collapsing everything into a generic error:

1. User Denial (`error=access_denied`): The user explicitly cancelled the consent screen. This is treated as a routine user action, not an application error.
2. Provider Errors: Server downtime or temporary provider outages are isolated so the user can be prompted to retry later.
3. Malformed Callbacks: Unrelated requests or stray port scans receive a 404 response and are dropped without ending the pending listener.
4. Timeouts: If the user abandons the browser window, the attempt expires after 1 minute, and the listener thread is cleanly unblocked and terminated.

User-facing messages state that the connection was cancelled or could not be completed, omitting raw tokens, internal URLs, or authorization codes.

## Security checklist

* Bind desktop callbacks strictly to `127.0.0.1` on dynamic ephemeral ports.
* Enforce socket read and write timeouts on loopback connections.
* Validate callback paths and perform constant-time comparisons on CSRF state tokens.
* Generate fresh PKCE challenges for every authorization attempt.
* Redact all tokens, refresh tokens, and client secrets from `Debug` logging formatters.
* Store refresh tokens exclusively in the operating system keyring, never in SQLite.
* Delete keyring credentials before deleting database account records.
* Advance synchronization cursors only after full batch hydration succeeds.
* Run external email bytes through safe charset decoders without panicking.
* Validate AI classification outputs against active criteria and enforce confidence thresholds.
* Keep bundled development credentials out of source control and out of plaintext runtime environment variables.