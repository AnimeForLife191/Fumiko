# Connecting an Email Account with OAuth

OAuth allows Fumiko to access a user's mailbox without ever asking for, seeing, or handling their account password. The provider handles the identity verification, two-factor challenges, and user consent screens. Fumiko simply receives short-lived access credentials and an encrypted refresh token scoped strictly to the email features the user opted into.

This document explains the technical design of Fumiko's desktop OAuth 2.0 flow. It is written to serve as both an architectural guide and an educational reference so the subtle protocol decisions, platform quirks, and security tradeoffs made here remain clear as the project evolves.

---

## Supported Providers

Fumiko currently supports two major email ecosystems:

1. **Gmail**, via Google OAuth 2.0.
2. **Outlook / Microsoft 365**, via Microsoft identity platform (supporting both organizational Azure AD accounts and personal Microsoft accounts).

Providers are stored alongside account metadata in SQLite using lowercase canonical identifiers (`gmail` and `outlook`). Any unrecognized provider is rejected during linking rather than coerced into a default type.

---

## The Sign-In Lifecycle

The authorization lifecycle coordinates a local temporary web server, the operating system's default browser, and the remote identity provider through an authorization code flow with PKCE:


### Detailed Step-by-Step

1. **Credential Resolution**: Fumiko loads the provider's OAuth client configuration. If the user provided custom client credentials in storage, those take precedence over development values.
2. **Ephemeral Loopback Binding**: Fumiko binds a `TcpListener` to `127.0.0.1:0`. Passing port `0` instructs the operating system kernel to assign a fresh, available ephemeral port.
3. **Client & URL Construction**: An OAuth client is instantiated with the provider's authorization and token endpoints, using the dynamically assigned port to construct the exact redirect URI (`http://127.0.0.1:{port}/`).
4. **PKCE & State Generation**: A cryptographically random PKCE verifier/challenge pair is generated, along with an unguessable CSRF state token.
5. **Browser Handoff**: Fumiko launches the user's default browser pointing to the provider's authorization URL, appending the PKCE challenge, state token, required scopes, and provider-specific parameters.
6. **Local Callback Processing**: A blocking background worker listens for the browser redirect. The worker enforces short socket read timeouts to prevent silent port scans from hanging the thread, parses the authorization code, serves a friendly confirmation page, and cleanly flushes the TCP socket.
7. **CSRF Validation**: Fumiko validates the returned state string against the original request state using constant-time byte comparison.
8. **Token Exchange**: Fumiko issues a direct backchannel HTTPS request to the provider's token endpoint, trading the authorization code and original PKCE verifier for an access token and refresh token.
9. **Secure Storage**: The access token is held in memory for immediate sync. The long-lived refresh token is committed directly to the operating system keyring, while non-sensitive account metadata is written to SQLite.

---

## Redirect URI Registration and RFC 8252

Fumiko registers a **portless loopback redirect URI** in the developer consoles of both Google and Microsoft:

```
http://localhost/ or http://127.0.0.1/
```

### Why Ephemeral Ports Work
Under [RFC 8252 §7.3 (OAuth 2.0 for Native Apps)](https://datatracker.ietf.org/doc/html/rfc8252#section-7.3), identity providers supporting desktop/native clients allow loopback redirect URIs to match any runtime port. When registering a native client with Google (Application Type: *Desktop App*) or Microsoft (Platform: *Mobile and desktop applications*), the provider validates the scheme, host, and path, while allowing the port component to vary dynamically.

This eliminates two major failure modes of fixed-port listeners:
* **Port Collisions**: If another application (or a zombie instance of Fumiko) is holding a hardcoded port open, sign-in will not crash or fail to bind.
* **Concurrent Logins**: A user can link multiple accounts in parallel without separate attempts fighting over the same local socket.

### Network Binding Note: `127.0.0.1` vs. `localhost`
While registration in developer portals often uses `http://localhost`, Fumiko binds its underlying listener to `127.0.0.1` and constructs the redirect URI using the explicit numeric IPv4 loopback address. Modern operating systems and browsers frequently resolve the hostname `localhost` to the IPv6 loopback address `::1` first. If a listener is only bound to IPv4 `127.0.0.1`, a browser redirecting to `http://localhost:{port}/` can hit an IPv6 `Connection Refused` error. Using explicit IPv4 routing eliminates this ambiguity.

---

## Why Both PKCE and State Are Mandatory

Although PKCE and state both involve random values sent during authorization, they protect against completely different attack vectors. Neither is a substitute for the other.

* **PKCE (Proof Key for Code Exchange)**: Protects the authorization code during transit against code interception. If a malicious process on the local machine snoops on the browser redirect and steals the code, it cannot redeem it because it lacks the private PKCE verifier held in memory.
* **State Token**: Protects the client application from accepting arbitrary or forged responses. If an attacker attempts to trick Fumiko into linking an attacker-controlled mailbox by injecting a stray callback, the random state value will not match the initiated request, and Fumiko rejects the payload immediately.

---

## Client Credentials & Build-Time Secrets

### Public vs. Confidential Clients
* **Google** operates as a confidential client in our desktop configuration, requiring both a `client_id` and a `client_secret`.
* **Microsoft** uses a public client registration for desktop applications, requiring only a `client_id` with no client secret.

### The `option_envc!` Macro and `build.rs` Isolation
To allow development builds without requiring manual environment exports, bundled development credentials are encrypted directly into the binary at compile time via `envcrypt::option_envc!`.

Because `option_envc!` resolves at **compile time**, it reads variables from the environment that `cargo` runs in. To make local development seamless, each crate using `option_envc!` has a `build.rs` script that loads the workspace-root `.env` file via `dotenvy` and forwards them using `cargo:rustc-env`.

**Important Build Invariant**: Directives emitted by `cargo:rustc-env` in a crate's `build.rs` apply **only to that specific crate**. They do not propagate up or down the dependency graph. Any crate in the workspace that calls `option_envc!` directly must maintain its own `build.rs` pointed at the root `.env`.

Production builds and end-users can override bundled values by entering custom credentials via the settings UI, which are stored in the local database and always take precedence over bundled fallback values.

---

## Token Storage and Keyring Lifecycle

Fumiko enforces a strict boundary between non-sensitive metadata and long-lived authentication secrets:

* **SQLite Database**: Stores account IDs, email addresses, display names, sync cursors, and provider identifiers. It never contains refresh tokens, client secrets, or raw passwords.
* **Operating System Keyring**: Refresh tokens are stored directly in the OS credential vault (Apple Keychain, Windows Credential Manager, or Linux Secret Service / Keyutils), keyed by the account's unique `Uuid`.



### Seamless Refresh Token Rotation
When Fumiko uses a refresh token to obtain a new access token, OAuth providers can optionally return a **replacement refresh token** (refresh token rotation).
* If the provider issues a new refresh token, Fumiko overwrites the existing keyring entry with the new value.
* If the provider omits a replacement, Fumiko preserves the existing token. 

This prevents account de-authorization when providers issue single-use refresh tokens.

---

## Error Classification and Edge-Case Handling

Rather than collapsing all failures into a generic error, the `oauth` crate differentiates failures by their root cause:



1. **User Denial (`error=access_denied`)**: The user clicked "Cancel" on the provider consent screen. This is treated as a routine user action, surfaced cleanly in the UI without alarming error traces.
2. **Provider Errors (`server_error`, `temporarily_unavailable`)**: The provider failed internally. Kept distinct from user cancellations so the UI can recommend retrying later.
3. **Malformed or Unrelated Callbacks**: Stray local traffic (such as a browser pre-fetching `/favicon.ico` or port scans) is served a `404 Not Found` and discarded without terminating the listener loop. Only a valid code or a terminal provider error concludes the attempt.
4. **Timeouts and Worker Pinning**: If the user abandons the browser window, the flow times out after one minute. Because `callback_task` is an asynchronous `JoinHandle` that may be checked or awaited across multiple cancellation branches, it is pinned with `tokio::pin!(callback_task)` to allow clean, non-blocking select/timeout handling without moving errors.
5. **CSRF State Mismatches**: If the returned state differs from the initiated state, the exchange is rejected immediately to prevent cross-site request forgery.

---

## Scopes & The Microsoft Personal Account Gotcha

Scopes are supplied by the caller rather than hardcoded in the OAuth library, keeping the OAuth client modular and feature-agnostic.

### Recommended Baseline Scopes
* **Gmail**: `https://www.googleapis.com/auth/gmail.readonly`
* **Outlook**: `https://graph.microsoft.com/Mail.Read`, `https://graph.microsoft.com/User.Read`, `offline_access`

### The `User.Read` Gotcha on Personal Microsoft Accounts
Microsoft Graph uses the multi-tenant `https://login.microsoftonline.com/common` endpoint to authenticate both organizational (work/school) and personal (MSA / live.com / hotmail.com) accounts.

**Critical Gotcha**: On personal Microsoft accounts, requesting only `Mail.Read` will succeed during sign-in, but subsequent calls to `GET /me` (to fetch the user's profile and email address) will fail with an opaque **`401 UnknownError`**. 

Microsoft Graph strictly requires the **`User.Read`** scope alongside `Mail.Read` to populate basic profile fields on personal accounts. Always include `User.Read` in the initial scope list.

---

## Known Limitations

* **No Active UI Cancel Handle**: If a user switches back to Fumiko and wants to dismiss a pending login, there is currently no exposed channel to cancel the blocking loopback listener immediately from the frontend. The attempt must wait out the one minute timeout before the port is released.
* **End-to-End Live Token Tests**: Unit tests cover state validation, callback parsing, URL decoding, and timeout cancellation. However, live token exchanges are not covered in CI because they require live mock identity provider endpoints.

---

## Security & Architectural Checklist

When modifying authentication logic or adding new OAuth providers, verify these invariants:

* **Loopback Only**: Always bind to `127.0.0.1` and never listen on public interfaces (`0.0.0.0`).
* **Dynamic Ports**: Use ephemeral OS-assigned ports (`:0`) to eliminate port collisions.
* **PKCE Mandatory**: Always create a fresh S256 PKCE challenge for every sign-in attempt, even for confidential clients with client secrets.
* **Constant-Time State Validation**: Validate CSRF state tokens byte-by-byte without short-circuiting on the first mismatched character.
* **Bounded Socket Operations**: Enforce read/write timeouts on incoming TCP streams so stalled sockets do not block the worker.
* **Flush Sockets Before Close**: Explicitly flush and shut down TCP connections before dropping them to avoid client-side connection resets.
* **Keyring Isolation**: Store refresh tokens exclusively in the operating system keyring; keep database schemas clean of authentication secrets.
* **Redacted Logging**: Implement explicit, redacted `Debug` formatters on credential structures to prevent accidental token dumps in application logs.
* **Build Isolation Awareness**: Remember that `build.rs` environment exports via `cargo:rustc-env` do not cross crate boundaries.