//! Token conversion, refresh operations, and unified account secret hydration.

use std::time::{Duration, Instant};

use envcrypt::option_envc;
use oauth2::{RefreshToken, TokenResponse};
use reqwest::Client as ReqwestClient;

use super::authorization::{OAuthClient, build_oauth_client};
use super::load_credentials;
use super::models::{OAuthProvider, TokenSet};
use common::{Provider, TokenError};
use storage::Storage;
use storage::models::LinkedAccount;

/// Converts a provider token response into a safe in-memory [`TokenSet`].
///
/// Automatically falls back to the existing refresh token if the provider omits a replacement
/// during refresh. Applies a proactive 10-minute expiry safety buffer (capped to a maximum
/// 50-minute TTL) so that active background sync loops never race against expired credentials.
pub fn response_to_token_set(
    token_result: &impl TokenResponse,
    fallback_refresh_token: Option<String>,
) -> TokenSet {
    let access_token = token_result.access_token().secret().to_string();

    // RFC 6749 Section 6 permits token endpoints to omit a new refresh token if the existing
    // refresh token remains valid. If omitted, we preserve our existing token.
    let refresh_token = token_result
        .refresh_token()
        .map(|t| t.secret().to_string())
        .or(fallback_refresh_token);

    // Most providers issue access tokens valid for 3600 seconds (1 hour).
    let raw_expires_in = token_result
        .expires_in()
        .unwrap_or(Duration::from_secs(3600));

    // Proactive refresh buffer: Subtract 10 minutes from provider expiration, clamping
    // between 1 minute minimum and 50 minutes maximum. This ensures active sync operations
    // never attempt API requests with a near-expired access token.
    let safe_expires_in = raw_expires_in
        .saturating_sub(Duration::from_secs(10 * 60))
        .max(Duration::from_secs(60))
        .min(Duration::from_secs(50 * 60));

    TokenSet {
        access_token,
        refresh_token,
        expires_at: Instant::now() + safe_expires_in,
    }
}

/// Exchanges a refresh token with the identity provider for a fresh [`TokenSet`].
///
/// # Errors
/// Returns [`TokenError::Request`] if the HTTP exchange with the token endpoint fails.
pub async fn refresh_access_token(
    oauth_client: &OAuthClient,
    http_client: &ReqwestClient,
    refresh_token: &str,
) -> Result<TokenSet, TokenError> {
    let token_result = oauth_client
        .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
        .request_async(http_client)
        .await
        .map_err(|error| TokenError::Request(Box::new(error)))?;

    Ok(response_to_token_set(
        &token_result,
        Some(refresh_token.to_string()),
    ))
}

/// Retrieves a fresh authentication token or password for a linked account.
///
/// Unified across both account types:
/// 1. IMAP Accounts: Retrieves the encrypted App Password from the OS keyring and returns it
///    immediately as the active `access_token` with an in-memory 50-minute TTL.
/// 2. OAuth Accounts: Loads the refresh token from the OS keyring, exchanges it with the
///    identity provider for a fresh access token, and transparently updates the keyring if the
///    provider issued a replacement refresh token (refresh token rotation).
///
/// # Errors
/// Returns [`TokenError::MissingAccountSecret`] if no secret exists in the OS keyring for the account,
/// [`TokenError::Credentials`] if client credentials cannot be resolved,
/// [`TokenError::Request`] if the network exchange fails,
/// or [`TokenError::Storage`] if interacting with the OS keyring fails.
pub async fn get_access_token_for_account(
    storage: &Storage,
    account: &LinkedAccount,
    http_client: &ReqwestClient,
) -> Result<TokenSet, TokenError> {
    // 1. IMAP Account Branch
    // For IMAP accounts, the App Password itself serves as the active secret.
    // Setting an in-memory 50-minute TTL enforces RAM cache eviction hygiene,
    // requiring the password to be re-read from the encrypted OS keyring periodically.
    if account.provider == Provider::Imap || account.imap_host.is_some() {
        let password = storage
            .token_store
            .get_account_secret(account.id)?
            .ok_or(TokenError::MissingAccountSecret(account.id))?;

        return Ok(TokenSet {
            access_token: password,
            refresh_token: None,
            expires_at: Instant::now() + Duration::from_secs(50 * 60),
        });
    }

    // 2. OAuth Account Branch
    let refresh_token = storage
        .token_store
        .get_account_secret(account.id)?
        .ok_or(TokenError::MissingAccountSecret(account.id))?;

    let provider_kind = match account.provider {
        Provider::Gmail => OAuthProvider::Google,
        Provider::Outlook => OAuthProvider::Microsoft,
        Provider::Imap => unreachable!("IMAP handled above"),
    };

    let credentials = match provider_kind {
        OAuthProvider::Google => {
            load_credentials(
                &provider_kind,
                storage,
                option_envc!("GOOGLE_CLIENT_ID"),
                option_envc!("GOOGLE_CLIENT_SECRET"),
            )
            .await?
        }
        OAuthProvider::Microsoft => {
            load_credentials(
                &provider_kind,
                storage,
                option_envc!("MICROSOFT_CLIENT_ID"),
                None,
            )
            .await?
        }
    };

    // Port 0 is used as a placeholder: token refresh uses a direct POST and does not bind a redirect port.
    let oauth_client =
        build_oauth_client(credentials, 0).map_err(|e| TokenError::Request(Box::new(e)))?;

    let tokens = refresh_access_token(&oauth_client, http_client, &refresh_token).await?;

    // Refresh Token Rotation: If the provider issued a new refresh token alongside
    // the new access token, persist the new refresh token to the OS keyring immediately.
    if let Some(new_refresh_token) = tokens.refresh_token.as_deref() {
        if new_refresh_token != refresh_token {
            storage
                .token_store
                .save_account_secret(account.id, new_refresh_token)?;
        }
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::response_to_token_set;

    fn token_response(refresh_token: Option<&str>) -> oauth2::basic::BasicTokenResponse {
        let mut response = serde_json::json!({
            "access_token": "new-access-token",
            "token_type": "Bearer"
        });

        if let Some(refresh_token) = refresh_token {
            response["refresh_token"] = serde_json::Value::String(refresh_token.to_string());
        }

        serde_json::from_value(response).unwrap()
    }

    #[test]
    fn preserves_access_token() {
        let tokens = response_to_token_set(&token_response(None), None);

        assert_eq!(tokens.access_token, "new-access-token");
    }

    #[test]
    fn returned_refresh_token_replaces_old_token() {
        let tokens = response_to_token_set(
            &token_response(Some("new-refresh-token")),
            Some("old-refresh-token".to_string()),
        );

        assert_eq!(tokens.refresh_token.as_deref(), Some("new-refresh-token"));
    }

    #[test]
    fn missing_refresh_token_preserves_old_token() {
        let tokens =
            response_to_token_set(&token_response(None), Some("old-refresh-token".to_string()));

        assert_eq!(tokens.refresh_token.as_deref(), Some("old-refresh-token"));
    }

    #[test]
    fn missing_refresh_token_without_fallback_stays_none() {
        let tokens = response_to_token_set(&token_response(None), None);

        assert_eq!(tokens.refresh_token, None);
    }
}