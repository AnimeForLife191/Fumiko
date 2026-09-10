pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("invalid OAuth endpoint configuration: {0}")]
    InvalidEndpointConfig(#[source] BoxError),
    #[error("OAuth callback failed: {0}")]
    CallbackTaskFailed(#[source] BoxError),
    #[error("invalid OAuth callback target")]
    InvalidCallbackTarget,
    #[error("OAuth callback is missing the authorization code")]
    MissingAuthorizationCode,
    #[error("OAuth callback is missing the state parameter")]
    MissingState,
    #[error("authorization was denied: {0}")]
    AccessDenied(String),
    #[error("timed out waiting for the OAuth callback")]
    CallbackTimeout,
    #[error("failed to open the browser: {0}")]
    BrowserLaunchFailed(#[source] BoxError),
    #[error("failed to bind the OAuth callback listener: {0}")]
    ListenerBindFailed(#[source] BoxError),
    #[error("OAuth flow was cancelled")]
    Cancelled,
    #[error("CSRF state mismatch")]
    CsrfMismatch,
    #[error("network error during token exchange: {0}")]
    Network(#[source] BoxError),
    #[error("token exchange failed: {0}")]
    TokenExchange(#[source] BoxError),
    #[error("OAuth callback contained an empty authorization code")]
    EmptyAuthorizationCode,
    #[error("the provider reported an error: {0}")]
    ProviderCallbackError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("invalid uuid: {0}")]
    InvalidUuid(#[from] uuid::Error),
    #[error("a record with this value already exists")]
    Conflict,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("email not found")]
    NotFound,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("database or keyring error: {0}")]
    Storage(#[from] StorageError),
    #[error("invalid credentials JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("Google credentials are missing client_id")]
    MissingGoogleClientId,
    #[error("Google credentials are missing client_secret")]
    MissingGoogleClientSecret,
    #[error("no {provider} client ID is configured")]
    MissingClientId { provider: &'static str },
    #[error("no Google client secret is configured")]
    MissingGoogleClientSecretConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("credential error: {0}")]
    Credentials(#[from] CredentialError),
    #[error("OAuth request failed")]
    Request(#[source] BoxError),
    #[error("account {0} has no stored refresh token")]
    MissingRefreshToken(uuid::Uuid),
    #[error("unsupported provider: {0}")]
    UnsupportedProvider(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("sync cursor expired - full resync is required")]
    CursorExpired,
    #[error("access token expired or invalid")]
    Unauthorized,
    #[error("invalid provider data: {0}")]
    InvalidData(String),
    #[error("network or provider error: {0}")]
    Other(#[from] BoxError),
}

impl From<reqwest::Error> for ProviderError {
    fn from(e: reqwest::Error) -> Self {
        if e.status() == Some(reqwest::StatusCode::UNAUTHORIZED) {
            ProviderError::Unauthorized
        } else {
            ProviderError::Other(Box::new(e))
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("database error: {0}")]
    Database(#[from] StorageError),
    #[error("account not found: {0}")]
    AccountNotFound(uuid::Uuid),
    #[error("no stored refresh token for account {0} — account needs re-linking")]
    MissingRefreshToken(uuid::Uuid),
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("oauth error: {0}")]
    OAuth(#[from] BoxError),
    #[error("unsupported provider: {0}")]
    UnsupportedProvider(String),
}

impl From<TokenError> for SyncError {
    fn from(error: TokenError) -> Self {
        match error {
            TokenError::Storage(error) => Self::Database(error),
            TokenError::MissingRefreshToken(account_id) => Self::MissingRefreshToken(account_id),
            TokenError::UnsupportedProvider(provider) => Self::UnsupportedProvider(provider),
            error => Self::OAuth(Box::new(error)),
        }
    }
}
