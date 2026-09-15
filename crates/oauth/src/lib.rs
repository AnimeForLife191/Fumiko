//! Desktop OAuth 2.0 PKCE authentication for Gmail and Microsoft Outlook.
//!
//! Implements RFC 8252 dynamic loopback redirection on 127.0.0.1:0, S256 PKCE challenge
//! negotiation, constant-time CSRF state verification, and refresh token rotation.
//!
//! # Security Architecture
//! * Ephemeral Loopback Binding: Binds to 127.0.0.1:0 to allow the operating system kernel
//!   to dynamically assign an unallocated port. Redirect URIs use explicit IPv4 literals to prevent
//!   operating system resolution issues with "localhost" resolving to IPv6 ::1.
//! * PKCE Protection: A cryptographically random SHA-256 code challenge is sent to the identity
//!   provider, while the private verifier is retained strictly in memory for the code exchange.
//! * Constant-Time CSRF Validation: Incoming callback state tokens are verified using bitwise
//!   XOR accumulation to prevent microarchitectural timing side channels.
//! * Proactive Token Expiry Buffering: Access tokens are cached in memory with a 10-minute
//!   safety margin (capped at 50 minutes TTL) and are never written to SQLite tables or persistent logs.

mod authorization;
mod credentials;
mod models;
mod tokens;

pub use authorization::run_oauth;
pub use credentials::{load_credentials, parse_google_credentials_json};
pub use models::{OAuthProvider, RawCredentials, TokenSet};
pub use tokens::get_access_token_for_account;