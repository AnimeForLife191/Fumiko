use std::fmt;
use std::time::Instant;

/// Supported OAuth 2.0 identity providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthProvider {
    Google,
    Microsoft,
}

impl OAuthProvider {
    /// Returns provider-specific query parameters required during authorization.
    ///
    /// - **Google**: Appends `access_type=offline` and `prompt=consent` to ensure a long-lived refresh token is returned.
    /// - **Microsoft**: Appends `prompt=select_account` to allow switching accounts instead of auto-selecting active SSO sessions.
    pub fn extra_params(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            OAuthProvider::Google => &[
                // Requests a refresh token so the app can sync in the background.
                ("access_type", "offline"),
                // Forces consent to ensure Google re-issues a refresh token if re-linking.
                ("prompt", "consent"),
            ],
            OAuthProvider::Microsoft => &[
                // Prompts account picker instead of silently auto-selecting the active SSO session.
                ("prompt", "select_account"),
            ],
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
    /// Short-lived bearer token used for authenticated provider API requests.
    pub access_token: String,
    /// Long-lived token used to obtain new access tokens when expired.
    pub refresh_token: Option<String>,
    /// Monotonic instant when the access token expires.
    pub expires_at: Instant,
}

impl fmt::Debug for TokenSet {
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

