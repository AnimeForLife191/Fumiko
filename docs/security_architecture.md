# Security and Threat Model Notes

Security in Fumiko is about limiting what the application can receive, where sensitive values can be stored, and which requests the system is willing to trust. OAuth handles the user password and remote sign-in session. Fumiko should only receive the minimum permissions and tokens it needs for the email features the user chose.

This document describes the security model for the desktop application, explains our threat boundaries, and highlights the rules that must remain in place as the project grows.

---

## What We Are Protecting

The most sensitive values in the application lifecycle are:

* The authorization code returned by the provider
* The PKCE verifier and CSRF state token for active sign-in attempts
* Short-lived access tokens held in memory
* Long-lived refresh tokens stored on disk
* User-configured OAuth client secrets
* Email message content, headers, and attachments returned by providers

A nearby local process, a compromised log file, or an accidental database backup should not be enough to obtain long-lived access to a user mailbox.

No design can protect a machine that is already fully compromised at the root level. The goal is to reduce exposure, reject forged or unrelated callbacks, prevent timing leaks, and avoid turning ordinary database or log files into credential dumps.

---

## Loopback Callback and Local Network Boundaries

The desktop OAuth flow listens strictly on `127.0.0.1` using a fresh, operating system-assigned dynamic port for every sign-in attempt. It is not an internet-facing server and must never bind to `0.0.0.0`.

### Socket Timeouts and Connection Isolation
Because the listener runs locally, rogue local processes or background port scanners can open connections to the ephemeral port. To prevent these connections from hanging the listener:

* Accepted TCP sockets enforce a strict 2-second read and write timeout. A connection that sends no data will not block the listener thread indefinitely.
* The listener parses only the first HTTP request line and checks the exact expected callback path. Unrelated requests (such as browser favicon requests) receive a 404 response and are dropped without terminating the sign-in attempt.
* The listener response page is a static confirmation message. It never echoes authorization codes, tokens, client secrets, or state values back to the browser.
* Sockets are explicitly flushed and cleanly shut down before closing to prevent browser connection reset errors.
* Immediate Cancellation: If the user clicks Cancel in the UI, an RAII `ListenerGuard` triggers an internal loopback ping to unblock `listener.accept()` and free the port immediately, without waiting for a timeout.

### Numeric IP Routing Over Localhost
The callback listener binds to `127.0.0.1` and constructs redirect URIs using the explicit IPv4 loopback address. Many operating systems resolve the word `localhost` to the IPv6 address `::1` first. Using `127.0.0.1` prevents modern browsers from failing with connection refused errors when attempting IPv6 loopback routing.

---

## Cryptographic Protections in Transit

### PKCE Protects the Code Exchange
Proof Key for Code Exchange (PKCE) creates a cryptographically random verifier and a corresponding SHA-256 challenge. The challenge travels with the initial browser authorization request, while the private verifier stays in memory.

When Fumiko exchanges the authorization code for tokens, the provider requires the original verifier. Even if another local process intercepts the authorization code from the browser redirect, the code alone is useless without the private verifier. PKCE is generated for every authorization flow, including confidential flows that use a client secret.

### Constant-Time CSRF State Validation
Each authorization request creates a random state token. The provider echoes that value back with the redirect, and Fumiko validates it before accepting the authorization code.

State comparison uses constant-time byte validation rather than standard short-circuiting string comparisons. This eliminates microarchitectural timing side channels that could leak state token bytes. State tokens are single-use, request-specific, and never written to persistent logs.

---

## Credential Storage and Secret Lifecycle

### Access Tokens in Memory
Access tokens are short-lived. They remain in memory only, cached with a 50-minute proactive time-to-live. They are never written to SQLite, persistent cache files, URLs, or log messages.

### Refresh Tokens and Secrets in the OS Keyring
Refresh tokens and custom OAuth client secrets require stronger protection. Fumiko stores them exclusively in the native operating system credential vault (Apple Keychain, Windows Credential Manager, or Linux Secret Service).

* Refresh tokens are never stored in SQLite database tables.
* Public Client IDs live in SQLite settings, while sensitive Client Secrets live in the keyring.
* Automatic Rollback: If saving a custom client ID to SQLite settings fails, any associated client secret previously written to the keyring is rolled back and removed.
* Keyring-First Deletion: When an account is unlinked or deleted, its keyring entry is removed first before deleting the database row. If the keyring call fails, the database row stays intact so the user can retry.
* Refresh Token Rotation: If a provider returns a replacement refresh token during token refresh, the keyring entry is updated. If the provider omits a replacement, the existing token is preserved.

### Redacted Debug Formatting
Data structures that hold credentials (`RawCredentials` and `TokenSet`) implement custom `fmt::Debug` formatters that explicitly replace secrets with `[REDACTED]`. This guarantees that diagnostics or error logging macros cannot accidentally dump plaintext tokens into log files.

---

## Webview and Rendering Security

Rendering untrusted third-party HTML emails in a desktop application presents cross-site scripting (XSS) and data exfiltration risks. Fumiko isolates email content using a multi-layer pipeline:

### 1. Style Cleansing
Before HTML elements are scrubbed, regular expressions inspect all `<style>` blocks. `javascript:` protocol handlers are purged, and `@import` directives are commented out to prevent CSS-based data exfiltration to external servers.

### 2. Ammonia Structural Sanitization
The HTML markup is sanitized through Ammonia:
* Allowed URL schemes are strictly limited to `data`, `cid`, `http`, `https`, `mailto`, and `tel`.
* Dangerous executable elements (`<script>`, `<object>`, `<embed>`, `<iframe>`) are stripped.
* Inline event handlers (such as `onload` or `onclick`) are purged.
* Hardcoded `target` attributes on anchor tags are stripped, forcing all links to inherit our document base target.

### 3. Sandboxed Iframes and Navigation Trapping
Sanitized email markup is rendered inside an isolated `<iframe>` with `<base target="_top">` and strict sandbox flags:

sandbox="allow-same-origin allow-top-navigation-by-user-activation"
code Code

* JavaScript execution and forms remain completely disabled inside the frame.
* When a user clicks a link, the click is permitted to navigate the top-level browsing context.
* Dioxus desktop's `.with_navigation_handler()` intercepts the navigation before the webview loads it. If the link targets an external protocol (`http://`, `https://`, or `mailto:`), it launches the user's default system browser via `webbrowser::open()` and returns `false`. This prevents external websites from ever loading inside or replacing the Fumiko desktop application window.

---

## Synchronization, Storage, and AI Safety

### Safe Cursor Advancement
When synchronizing new mail, the sync cursor in SQLite is updated only if all message metadata in that batch was successfully retrieved and saved. If network issues cause a partial metadata fetch failure, the cursor is not advanced. This ensures that transient network drops never permanently skip unread messages.

### Idempotent Database Operations
Database queries are written defensively against duplicate or out-of-order provider events:
* `save_email` uses `ON CONFLICT DO UPDATE` to safely update read flags and missing snippets without crashing on duplicate message notifications.
* `mark_trashed` treats unsaved messages as safe no-ops rather than throwing not-found errors, preventing synchronization from crashing on messages that were trashed on another device before being synced locally.
* Database limits and trash retention periods are bounded to prevent integer overflow and memory exhaustion.

### Message Body and Charset Safety
Email bodies can contain arbitrary legacy character encodings. All raw body bytes are decoded through `encoding_rs` to replace invalid byte sequences safely rather than panicking.

Inline images referenced by Content-ID are converted into self-contained Base64 data URIs using case-insensitive substitution. This allows local webviews to render emails with images without running an unauthenticated local HTTP proxy server.

### Local AI Classification Boundaries
When running background email classification through local Ollama models:
* **Bounded Concurrency**: Inferences stream with bounded concurrency (up to 4 in parallel) to prevent CPU starvation.
* **Grammar-Constrained Schemas**: Prompts enforce structured JSON schemas via GBNF grammar constraints, preventing models from emitting formatting errors or leaking markdown blocks.
* **Safe Queueing Timeouts**: The classifier client uses an explicit 90-second timeout so requests queued behind single-slot inference do not drop prematurely.
* **Hallucination Guards**: Model output is strictly validated against the active database criteria list. If the model hallucinates an unknown label, the result is discarded.
* **Confidence Thresholds**: Classifications below 0.50 confidence are rejected to avoid miscategorizing user mail.

---

## Failure Shapes and Error Reporting

The application distinguishes several different failure shapes rather than collapsing everything into a generic error:

1. **User Denial (`error=access_denied`)**: The user explicitly cancelled the consent screen. This is treated as a routine user action, not an application error.
2. **Provider Errors**: Server downtime or temporary provider outages are isolated so the user can be prompted to retry later.
3. **Malformed Callbacks**: Unrelated requests or stray port scans receive a 404 response and are dropped without ending the pending listener.
4. **Timeouts and Cancellation**: If the user abandons the browser window, the attempt expires after 120 seconds (2 minutes). If the user clicks Cancel in Fumiko, the listener thread is cleanly unblocked and terminated immediately.

User-facing messages state that the connection was cancelled or could not be completed, omitting raw tokens, internal URLs, or authorization codes.

---

## Security Checklist

* Bind desktop callbacks strictly to `127.0.0.1` on dynamic ephemeral ports.
* Enforce socket read and write timeouts on loopback connections.
* Validate callback paths and perform constant-time comparisons on CSRF state tokens.
* Generate fresh PKCE challenges for every authorization attempt.
* Redact all tokens, refresh tokens, and client secrets from `Debug` logging formatters.
* Store refresh tokens and client secrets exclusively in the operating system keyring, never in SQLite.
* Delete keyring credentials before deleting database account records.
* Automatically roll back keyring secrets if setting writes fail in SQLite.
* Advance synchronization cursors only after full batch hydration succeeds.
* Render email bodies inside sandboxed iframes with script execution disabled.
* Intercept webview navigations and route external links to the default system browser.
* Run external email bytes through safe charset decoders without panicking.
* Validate AI classification outputs against active criteria and enforce confidence thresholds.
* Keep bundled development credentials out of source control and out of plaintext runtime environment variables.