# Connecting an Email Account with OAuth and App Passwords

Fumiko supports two authentication models: modern OAuth 2.0 PKCE for major cloud providers, and direct IMAP authentication with App Passwords for generic mailboxes or users who prefer not to create cloud developer keys.

This document explains the technical design of Fumiko's desktop authentication flows, how credentials are handled across crates, and how platform quirks are managed.

## Subsystem Division of Work

Connecting and authenticating accounts involves cooperation between two crates:

* **`oauth` crate**: Implements the RFC 8252 dynamic loopback listener, S256 PKCE exchange, constant-time CSRF verification, token refresh loops, and the unified `get_access_token_for_account` credential hydrator.
* **`email_core` crate**: Implements IMAP TLS connection handling, pre-flight live login verification over raw sockets, and protocol-level mailbox commands.

## Supported Authentication Methods

1. **Google OAuth 2.0**: Uses a dynamic loopback authorization code flow with PKCE. Requires user-supplied or bundled Google Cloud Console client credentials.
2. **Microsoft Identity Platform**: Uses PKCE against the `common` endpoint, specifically targeting personal Microsoft accounts (`@outlook.com`, `@hotmail.com`, `@live.com`, `@msn.com`). Organizational (Work/School) accounts are intentionally restricted due to enterprise Publisher Verification requirements.
3. **IMAP with App Passwords**: Connects via TLS to standard IMAP endpoints (Gmail, iCloud, Yahoo, Fastmail, or custom servers). Users generate a dedicated App Password from their provider, bypassing cloud developer console setup entirely.

## Provider Representation and Storage Mapping

To maintain strict type safety, provider identities exist at two levels:

1. **In the Rust Type System**:
   * The `oauth` crate defines `OAuthProvider` with two variants: `Google` and `Microsoft`.
   * The `common` crate defines `Provider` with three variants: `Gmail`, `Outlook`, and `Imap`.
   * `OAuthProvider` implements `From` and `TryFrom` to convert to and from `common::Provider`. Converting from `Provider::Imap` returns an error because IMAP mailboxes do not use OAuth.
2. **In SQLite Storage**:
   * The `common::Provider` enum serializes using lowercase formatting (`#[serde(rename_all = "lowercase")]`).
   * In the SQLite `linked_accounts` table, the `provider` column stores these values as plain text strings: `'gmail'`, `'outlook'`, or `'imap'`.

## The OAuth Sign-In Lifecycle

The OAuth authorization lifecycle coordinates a temporary local web server, the system default browser, and the remote provider through an authorization code flow with PKCE.

### Step-by-Step Flow

1. **Credential Resolution**: Fumiko loads the provider's OAuth client settings via `load_credentials`. If the user saved custom developer credentials in the app, those take priority over bundled development keys.
2. **Ephemeral Loopback Binding**: Fumiko binds a standard `TcpListener` to `127.0.0.1:0`. Passing port `0` tells the operating system kernel to assign an available dynamic port, avoiding port collisions and allowing concurrent logins.
3. **Client and URL Construction**: The OAuth client sets up its endpoints using the dynamically assigned port to construct the exact redirect URI: `http://127.0.0.1:{port}/`.
4. **PKCE and State Generation**: A cryptographically random PKCE verifier and challenge pair is generated (`PkceCodeChallenge::new_random_sha256()`), along with an unguessable CSRF state token (`CsrfToken::new_random`).
5. **Browser Handoff**: Fumiko launches the user's default browser pointing to the provider's sign-in screen, passing the PKCE challenge, state token, required scopes, and provider-specific parameters.
6. **Local Callback Processing**: A background worker on Tokio's blocking thread pool (`receive_authorization_code`) listens for the browser redirect. The worker enforces a 2-second socket read timeout to prevent silent port scans from hanging the thread, parses the authorization code, serves a static HTML confirmation page, and cleanly shuts down the TCP stream.
7. **Immediate Cancellation Support**: If the user clicks Cancel in Fumiko or closes the window, the background task is dropped. This triggers a `ListenerGuard` that connects a dummy TCP stream to the loopback port, unblocking `listener.accept()` and freeing the port right away.
8. **CSRF Validation**: Fumiko compares the returned state string against the original state token using a constant-time byte check (`constant_time_eq`) to eliminate timing side channels.
9. **Token Exchange**: Fumiko makes a direct HTTPS request to the provider token endpoint via `reqwest` with redirects disabled, trading the authorization code and private PKCE verifier for an access token and refresh token.
10. **Secure Storage**: The short-lived access token stays strictly in memory for active syncing. The long-lived refresh token is saved directly into the operating system keyring, and non-sensitive account metadata is written to SQLite.

## The IMAP Authentication Lifecycle

For users connecting without cloud developer credentials:

1. **Server and Credential Collection**: The user enters their email address, server host, port, and provider-generated App Password.
2. **Preset Configuration**: Common endpoints (such as `imap.gmail.com:993` or `imap.mail.me.com:993`) are auto-populated.
3. **Password Normalization**: Generators from Google and Apple display passwords in 4-character spaced groups (like `abcd efgh ijkl mnop`). Fumiko strips whitespace automatically to prevent authentication rejections.
4. **Pre-flight Live Login**: Before writing any data to SQLite, Fumiko opens a TLS socket to the server and executes an IMAP `LOGIN` via `email_core`. If login fails, an actionable error is returned immediately.
5. **Keyring Storage**: Once verified, the App Password is saved into the operating system keyring keyed by the account UUID. The plain password is never saved to SQLite tables.

## Redirect URI Registration and RFC 8252

For OAuth providers, Fumiko registers an explicit, portless loopback redirect URI in developer consoles:

```
http://127.0.0.1/
```

### Why Ephemeral Ports Work
Under RFC 8252 Section 7.3 (OAuth 2.0 for Native Apps), identity providers supporting desktop and native clients allow loopback redirect URIs to match any port at runtime. When registering a native client with Google (Desktop App) or Microsoft (Mobile and desktop applications), the provider checks the scheme, host, and path, while allowing the port component to vary dynamically.

This eliminates two major failure modes:
* **Port Collisions**: If another application or a zombie process is holding a hardcoded port open, sign-in will not crash or fail to bind.
* **Concurrent Logins**: A user can link multiple accounts in parallel without separate attempts fighting over the same local socket.

### Why We Strictly Use 127.0.0.1 Over localhost
Modern operating systems and web browsers frequently resolve the hostname `localhost` to the IPv6 loopback address `::1` first. If an application only binds its listener to IPv4 `127.0.0.1`, a browser redirecting to `localhost` will try IPv6 first and fail immediately with a connection refused error. RFC 8252 specifically recommends using the explicit literal IPv4 loopback address to avoid this ambiguity across platforms.

## Unified Secret Storage in the OS Keyring

Fumiko maintains a strict separation between non-sensitive metadata and sensitive authentication secrets:

* **SQLite Database**: Stores account IDs, email addresses, display names, sync cursors, provider identifiers, IMAP hosts/ports, and custom Client IDs. It never contains client secrets, refresh tokens, or passwords.
* **Operating System Keyring**: Sensitive credentials live exclusively in the native OS credential vault (Apple Keychain, Windows Credential Manager, or Linux Secret Service).

Each account has one primary authentication secret stored in the OS keyring under its account UUID string:
* For OAuth accounts: an encrypted **OAuth refresh token**.
* For IMAP accounts: an encrypted **App Password**.

The `TokenStore` abstraction manages both through unified `save_account_secret`, `get_account_secret`, and `delete_account_secret` methods. When an account is deleted or wiped, its secret is purged from the keyring regardless of whether it connected via OAuth or IMAP.

### Credential Caching and Memory Safety
To avoid frequent OS IPC calls into system keychains during background sync loops, active credentials are cached in memory using a `TokenSet`:
* **OAuth Access Tokens**: Providers typically issue access tokens with a 60-minute lifetime. Fumiko applies a proactive 10-minute safety buffer (capped at 50 minutes TTL) so active sync loops never attempt requests with an expiring token. When expired, the refresh token is used to exchange for a new access token.
* **IMAP App Passwords**: App passwords do not expire on the provider's server. However, Fumiko assigns them the same in-memory 50-minute TTL to enforce RAM cache eviction hygiene. After 50 minutes, the password is removed from memory and safely re-read on demand from the encrypted OS keyring.

## Scopes and the Microsoft Personal Account Quirk

Scopes are passed in by the caller rather than hardcoded into the OAuth client:

### Baseline Scopes
* **Gmail**: `https://www.googleapis.com/auth/gmail.readonly`
* **Outlook**: `https://graph.microsoft.com/Mail.Read`, `https://graph.microsoft.com/User.Read`, `offline_access`

### The User.Read and offline_access Requirements on Personal Microsoft Accounts
Microsoft Identity's `common` endpoint is designed to accept both consumer and organizational identities. However, because enterprise and educational Microsoft 365 tenants block unverified multi-tenant apps by default without official Publisher Verification (requiring legal entity registration and D-U-N-S audits), Fumiko focuses exclusively on personal consumer accounts (`@outlook.com`, `@hotmail.com`, `@live.com`, `@msn.com`).

Connecting personal Microsoft accounts involves two mandatory scope requirements alongside `Mail.Read`:

* **`User.Read` (Profile Discovery):** On personal Microsoft accounts, requesting only `Mail.Read` permits the user to sign in, but subsequent calls to `GET /me` (to fetch their display name and email address for account records) will fail with an unexpected `401 UnknownError`. Microsoft Graph strictly requires `User.Read` alongside `Mail.Read` to access basic profile details on consumer accounts. Always include `User.Read`.
* **`offline_access` (Refresh Token Issuance):** Unlike Google (which controls refresh tokens via an `access_type=offline` URL query parameter), Microsoft Identity v2.0 requires an explicit `offline_access` scope. If `offline_access` is omitted, Microsoft returns only a short-lived access token (valid for roughly 60 minutes) and omits the `refresh_token` entirely. For a persistent desktop inbox watcher, this scope is mandatory so background sync workers can rotate expired tokens directly from the OS keyring without forcing the user to re-authenticate in the browser every hour.

## Security Checklist

When touching authentication code or adding new providers, keep these rules in place:

* Bind desktop OAuth callbacks strictly to `127.0.0.1` and never listen on public interfaces (`0.0.0.0`).
* Use dynamic OS-assigned ports (`:0`) to avoid port conflicts.
* Generate a fresh S256 PKCE challenge for every sign-in attempt, even for clients with secrets.
* Validate CSRF state tokens byte-by-byte in constant time using XOR accumulation.
* Enforce read and write timeouts on incoming TCP connections so idle connections do not block the thread.
* Store refresh tokens and app passwords exclusively in the operating system keyring, never in SQLite tables.
* Normalize app passwords: trim whitespace and strip space groups before initiating IMAP verification.
* Enforce pre-flight credential verification: never save an account record to SQLite before verifying the connection with the remote provider.
* Implement redacted `Debug` formatters on credential structs to prevent accidental secret leaks in application logs.