//! Data structures and credential representations for OAuth flows.

use common::Provider;
use std::fmt;
use std::time::Instant;

/// Supported OAuth 2.0 identity providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthProvider {
    /// Google Identity Platform (Gmail readonly scopes).
    Google,
    /// Microsoft Identity Platform / Microsoft Entra ID (Graph Mail.Read scopes).
    Microsoft,
}

impl OAuthProvider {
    /// Returns provider-specific URL query parameters required during authorization.
    ///
    /// * Google: Requests access_type=offline and prompt=consent. Google only issues a refresh
    ///   token during the initial authorization or when explicit consent is re-prompted.
    /// * Microsoft: Requests prompt=select_account to allow users to choose which account to authenticate.
    pub fn extra_params(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            OAuthProvider::Google => &[("access_type", "offline"), ("prompt", "consent")],
            OAuthProvider::Microsoft => &[("prompt", "select_account")],
        }
    }
}

impl TryFrom<Provider> for OAuthProvider {
    type Error = Provider;

    /// Attempts to convert a storage [`Provider`] into an [`OAuthProvider`].
    ///
    /// Returns `Err(provider)` if the provider is [`Provider::Imap`], as IMAP accounts
    /// authenticate via direct App Passwords rather than OAuth 2.0.
    fn try_from(provider: Provider) -> Result<Self, Self::Error> {
        match provider {
            Provider::Gmail => Ok(OAuthProvider::Google),
            Provider::Outlook => Ok(OAuthProvider::Microsoft),
            Provider::Imap => Err(provider),
        }
    }
}

impl From<OAuthProvider> for Provider {
    fn from(provider: OAuthProvider) -> Self {
        match provider {
            OAuthProvider::Google => Provider::Gmail,
            OAuthProvider::Microsoft => Provider::Outlook,
        }
    }
}

/// Raw client credentials and endpoint URLs for initiating an OAuth flow.
pub struct RawCredentials {
    pub(crate) client_id: String,
    pub(crate) client_secret: Option<String>,
    pub(crate) auth_uri: String,
    pub(crate) token_uri: String,
}

impl RawCredentials {
    /// Creates a new `RawCredentials` descriptor.
    pub fn new(
        client_id: impl Into<String>,
        client_secret: Option<String>,
        auth_uri: impl Into<String>,
        token_uri: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret,
            auth_uri: auth_uri.into(),
            token_uri: token_uri.into(),
        }
    }
}

impl fmt::Debug for RawCredentials {
    /// Explicitly redacts `client_secret` to prevent credential exposure in diagnostic logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawCredentials")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("auth_uri", &self.auth_uri)
            .field("token_uri", &self.token_uri)
            .finish()
    }
}

/// Access and refresh token pair returned upon successful OAuth exchange or token refresh.
pub struct TokenSet {
    /// Short-lived bearer token used for authenticating provider API requests.
    pub access_token: String,
    /// Long-lived token used to obtain fresh access tokens. None for IMAP accounts.
    pub refresh_token: Option<String>,
    /// Monotonic deadline after which the cached access token must be refreshed.
    pub expires_at: Instant,
}

impl fmt::Debug for TokenSet {
    /// Explicitly redacts all token secrets to prevent credential leaks in logging macros.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_at", &self.expires_at)
            .finish()
    }
}