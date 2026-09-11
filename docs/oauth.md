# Connecting an Email Account with OAuth

OAuth allows Fumiko to access a user's mailbox without ever asking for, seeing, or storing their account password. The email provider handles identity verification, two-factor challenges, and consent screens. Fumiko simply receives short-lived access credentials and an encrypted refresh token scoped strictly to reading mail.

This document explains the technical design of Fumiko's desktop OAuth 2.0 flow in the `oauth` crate. It covers our protocol choices, platform quirks, and security tradeoffs.

---

## Supported Providers

Fumiko currently supports two email ecosystems:

1. **Gmail**, via Google OAuth 2.0.
2. **Outlook / Microsoft 365**, via the Microsoft identity platform.

For Microsoft, the app targets the multi-tenant `common` endpoint, supporting both personal accounts and work or school accounts. In practice, there is a key permissions difference:

* **Personal Microsoft accounts** (`@outlook.com`, `@hotmail.com`, `@live.com`): Connect immediately with standard user consent.
* **Work or school accounts** (Office 365 / Entra ID): The protocol supports them, but Microsoft restricts unverified multi-tenant apps from organizational tenants by default. Unless an institutional IT administrator has enabled user consent or explicitly approves the client ID, signing into a work or school account will show a "Need admin approval" screen.

Providers are stored alongside account metadata in SQLite using lowercase identifiers (`gmail` and `outlook`). Any unrecognized provider string is rejected during linking rather than coerced into a default.

---

## The Sign-In Lifecycle

The authorization lifecycle coordinates a temporary local web server, the system default browser, and the remote provider through an authorization code flow with PKCE.

### Step-by-Step Flow

1. **Credential Resolution**: Fumiko loads the provider's OAuth client settings. If the user saved custom developer credentials in the app, those take priority over bundled development keys.
2. **Ephemeral Loopback Binding**: Fumiko binds a standard `TcpListener` to `127.0.0.1:0`. Passing port `0` tells the operating system kernel to assign an available dynamic port.
3. **Client and URL Construction**: The OAuth client sets up its endpoints using the dynamically assigned port to construct the exact redirect URI: `http://127.0.0.1:{port}/`.
4. **PKCE and State Generation**: A cryptographically random PKCE verifier and challenge pair is generated, along with an unguessable CSRF state token.
5. **Browser Handoff**: Fumiko launches the user's default browser pointing to the provider's sign-in screen, passing the PKCE challenge, state token, required scopes, and extra provider parameters.
6. **Local Callback Processing**: A background worker listens for the browser redirect. The worker enforces short socket read timeouts to prevent silent port scans from hanging the thread, parses the authorization code, serves a confirmation page, and cleanly flushes the TCP socket.
7. **Immediate Cancellation Support**: If the user clicks Cancel in Fumiko or closes the window, the background task is dropped. This triggers a `ListenerGuard` that makes a quick dummy connection to the loopback port, unblocking `listener.accept()` and freeing the port right away.
8. **CSRF Validation**: Fumiko compares the returned state string against the original state token using a constant-time byte check.
9. **Token Exchange**: Fumiko makes a direct HTTPS request to the provider token endpoint, trading the authorization code and PKCE verifier for an access token and a refresh token.
10. **Secure Storage**: The access token stays strictly in memory for active syncing. The long-lived refresh token is saved directly into the operating system keyring, and non-sensitive account metadata is written to SQLite.

---

## Redirect URI Registration and RFC 8252

Fumiko registers an explicit, portless loopback redirect URI in developer consoles:

```
http://127.0.0.1/
```

### Why Ephemeral Ports Work
Under RFC 8252 Section 7.3 (OAuth 2.0 for Native Apps), identity providers supporting desktop and native clients allow loopback redirect URIs to match any port at runtime. When registering a native client with Google (Application Type: Desktop App) or Microsoft (Platform: Mobile and desktop applications), the provider checks the scheme, host, and path, while allowing the port component to vary dynamically.

This eliminates two major failure modes:
* **Port Collisions**: If another application or a zombie process is holding a hardcoded port open, sign-in will not crash or fail to bind.
* **Concurrent Logins**: A user can link multiple accounts in parallel without separate attempts fighting over the same local socket.

### Why We Strictly Use `127.0.0.1` Over `localhost`
Many developer guides suggest registering `http://localhost`, but Fumiko registers, binds, and redirects strictly using the numeric IPv4 address `http://127.0.0.1/`.

Modern operating systems and web browsers frequently resolve the hostname `localhost` to the IPv6 loopback address `::1` first. If an application only binds its listener to IPv4 `127.0.0.1`, a browser redirecting to `localhost` will try IPv6 first and fail immediately with a connection refused error. RFC 8252 specifically recommends using the explicit literal IPv4 loopback address to avoid this ambiguity across platforms.

### Network Binding: `127.0.0.1` vs `localhost`
While developer portals often suggest `http://localhost`, Fumiko binds strictly to `127.0.0.1` and uses the numeric IPv4 address in the redirect URI. Many operating systems resolve the word `localhost` to IPv6 `::1` first. If a socket is only bound to IPv4, a browser redirecting to `localhost` can fail with a connection refused error. Using explicit IPv4 prevents that issue.

---

## Why Both PKCE and State Are Mandatory

Although PKCE and state tokens both use random strings, they protect against completely different attack vectors. You need both.

* **PKCE (Proof Key for Code Exchange)**: Protects the authorization code while in transit. If another local process snoops on the browser redirect and steals the code, it cannot redeem it because it does not have the private PKCE verifier held in Fumiko's memory.
* **State Token**: Protects the client app against cross-site request forgery. If someone tries to trick Fumiko into linking an attacker-controlled mailbox by injecting a stray callback, the state token will not match, and Fumiko rejects the request.

---

## Client Credentials and Build-Time Secrets

### Public vs Confidential Clients
* **Google** acts as a confidential client in our desktop configuration, requiring both a `client_id` and a `client_secret`.
* **Microsoft** uses a public native client registration for desktop applications, requiring only a `client_id` without any client secret.

### The `option_envc!` Macro and `build.rs` Isolation
To allow development builds without requiring manual environment exports, bundled development credentials are encrypted directly into the binary at compile time via `envcrypt::option_envc!`.

Because `option_envc!` runs at compile time, each crate using it maintains its own `build.rs` script that loads the workspace root `.env` file via `dotenvy` and forwards the values using `cargo:rustc-env`. Directives emitted by `cargo:rustc-env` only apply to that specific crate and do not leak across dependencies.

User-provided credentials entered through the in-app settings always take precedence over bundled development values.

---

## Token Storage and Keyring Lifecycle

Fumiko keeps a clear boundary between non-sensitive metadata and sensitive authentication secrets:

* **SQLite Database**: Stores account IDs, email addresses, display names, sync cursors, provider identifiers, and custom Client IDs (saved in the local settings table). It never contains client secrets, refresh tokens, or account passwords.
* **Operating System Keyring**: Sensitive secrets live exclusively in the native OS credential vault (Apple Keychain, Windows Credential Manager, or Linux Secret Service). This includes account refresh tokens (indexed by account UUID) and user-supplied Google Client Secrets.

If a user copies, inspects, or backs up their local SQLite database file, no usable secrets or refresh tokens are ever exposed.

### Refresh Token Rotation
When Fumiko uses a refresh token to fetch a new access token, the provider may optionally return a replacement refresh token:
* If the provider returns a new refresh token, Fumiko updates the keyring entry with the new value.
* If the provider omits a replacement, Fumiko keeps the existing token.

This keeps accounts working smoothly even when providers enforce single-use refresh tokens.

---

## Error Handling and Edge Cases

The `oauth` crate separates failures by their actual cause so the UI can respond appropriately:

1. **User Denial (`error=access_denied`)**: The user clicked Cancel on the provider consent screen. This is treated as a routine action rather than an alarming crash.
2. **Provider Errors (`server_error`, `temporarily_unavailable`)**: The provider had an issue on their side. These are flagged so the UI can suggest retrying shortly.
3. **Malformed or Unrelated Callbacks**: Stray local traffic (like a browser asking for `/favicon.ico` or background port scans) receives a 404 response and is ignored without closing the listener loop.
4. **Timeouts and Cancellation**: If the user abandons the browser window, the attempt expires after 120 seconds. If the user clicks Cancel in Fumiko, the listener task drops immediately and unblocks the socket via an internal loopback ping.
5. **CSRF State Mismatches**: If the returned state does not match what was sent, the request is rejected immediately using constant-time byte comparison.

---

## Scopes and the Microsoft Personal Account Quirk

Scopes are passed in by the caller rather than hardcoded into the OAuth client, keeping the authentication library modular.

### Baseline Scopes
* **Gmail**: `https://www.googleapis.com/auth/gmail.readonly`
* **Outlook**: `https://graph.microsoft.com/Mail.Read`, `https://graph.microsoft.com/User.Read`, `offline_access`

### The `User.Read` Requirement on Personal Microsoft Accounts
Microsoft Graph uses the multi-tenant `common` endpoint to authenticate both work/school accounts and personal accounts (`@outlook.com`, `@hotmail.com`, `@live.com`).

On personal Microsoft accounts, requesting only `Mail.Read` will let the user sign in, but subsequent calls to `GET /me` (to fetch their display name and email address) will fail with an unexpected `401 UnknownError`. Microsoft Graph strictly requires the `User.Read` scope alongside `Mail.Read` to access basic profile details on personal accounts. Always include `User.Read`.

---

## Known Limitations

* **Live Token CI Tests**: Unit tests cover state validation, callback parsing, URL decoding, and cancellation pings. However, full end-to-end token exchanges are not run in standard CI because they require live mock identity provider endpoints.

---

## Security Checklist

When touching authentication code or adding new providers, keep these rules in place:

* Always bind desktop callbacks strictly to `127.0.0.1` and never listen on public interfaces (`0.0.0.0`).
* Use dynamic OS-assigned ports (`:0`) to avoid port conflicts.
* Generate a fresh S256 PKCE challenge for every sign-in attempt, even for clients with secrets.
* Validate CSRF state tokens byte-by-byte in constant time.
* Enforce read and write timeouts on incoming TCP connections so idle connections do not block the thread.
* Explicitly flush and shut down sockets before closing them to prevent browser reset errors.
* Store refresh tokens exclusively in the operating system keyring, never in SQLite tables.
* Implement redacted `Debug` formatters on credential structs to prevent accidental secret leaks in application logs.