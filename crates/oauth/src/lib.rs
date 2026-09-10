//! Desktop OAuth 2.0 PKCE authentication for Gmail and Microsoft Outlook.
//!
//! Implements RFC 8252 dynamic loopback redirection on `127.0.0.1:0`, S256 PKCE challenge
//! negotiation, constant-time CSRF state verification, and refresh token rotation.

mod authorization;
mod credentials;
mod models;
mod tokens;

pub use authorization::run_oauth;
pub use credentials::{load_credentials, parse_google_credentials_json};
pub use models::{OAuthProvider, RawCredentials, TokenSet};
pub use tokens::get_access_token_for_account;