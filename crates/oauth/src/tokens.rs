use std::str::FromStr;
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

pub fn response_to_token_set(
    token_result: &impl TokenResponse,
    fallback_refresh_token: Option<String>,
) -> TokenSet {
    let access_token = token_result.access_token().secret().to_string();

    // Preserve the existing refresh token if provider omits a replacement during refresh.
    let refresh_token = token_result
        .refresh_token()
        .map(|t| t.secret().to_string())
        .or(fallback_refresh_token);

    let expires_in = token_result
        .expires_in()
        .unwrap_or(Duration::from_secs(3600));

    TokenSet {
        access_token,
        refresh_token,
        expires_at: Instant::now() + expires_in,
    }
}

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

/// Retrieves a fresh access token for a linked account using its stored refresh token.
///
/// Loads the account's long-lived refresh token from the OS keyring, exchanges it with the
/// identity provider for a new access token, and transparently updates the keyring if the
/// provider issued a replacement refresh token (refresh token rotation).
///
/// # Errors
/// Returns [`TokenError::MissingRefreshToken`] if no token exists in the OS keyring,
/// or [`TokenError::Request`] if the HTTP exchange with the provider fails.
pub async fn get_access_token_for_account(
    storage: &Storage,
    account: &LinkedAccount,
    http_client: &ReqwestClient,
) -> Result<TokenSet, TokenError> {
    let refresh_token = storage
        .token_store
        .get_refresh_token(account.id)?
        .ok_or(TokenError::MissingRefreshToken(account.id))?;

    let provider =
        Provider::from_str(account.provider.as_str()).map_err(TokenError::UnsupportedProvider)?;

    let provider_kind = match provider {
        Provider::Gmail => OAuthProvider::Google,
        Provider::Outlook => OAuthProvider::Microsoft,
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

    if let Some(new_refresh_token) = tokens.refresh_token.as_deref() {
        if new_refresh_token != refresh_token {
            storage
                .token_store
                .save_refresh_token(account.id, new_refresh_token)?;
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