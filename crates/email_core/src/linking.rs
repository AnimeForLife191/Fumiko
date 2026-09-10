use super::EmailProvider;
use super::email_providers::{GmailProvider, OutlookProvider};
use common::{BoxError, Provider};
use envcrypt::option_envc;
use oauth::{OAuthProvider, load_credentials, run_oauth};
use oauth2::Scope;
use storage::{Storage, StorageError};
use uuid::Uuid;

struct LinkConfig {
    oauth_provider: OAuthProvider,
    app_provider: Provider,
    bundled_client_id: Option<&'static str>,
    bundled_client_secret: Option<&'static str>,
    scopes: Vec<Scope>,
    provider_label: &'static str,
}

async fn link_account_with_config<P>(
    storage: &Storage,
    config: LinkConfig,
    provider: &P,
) -> Result<Uuid, BoxError>
where
    P: EmailProvider,
{
    let credentials = load_credentials(
        &config.oauth_provider,
        storage,
        config.bundled_client_id,
        config.bundled_client_secret,
    )
    .await?;

    let tokens = run_oauth(credentials, &config.oauth_provider, config.scopes).await?;

    // Invariant: An account without a refresh token cannot be automatically synchronized in
    // the background. We validate its presence before creating DB records to avoid orphaned accounts.
    let refresh_token = tokens
        .refresh_token
        .as_ref()
        .ok_or_else(|| BoxError::from("No refresh token was provided by OAuth provider"))?;

    let profile = provider.get_profile(&tokens.access_token).await?;

    let account_id = match storage
        .add_account(
            config.app_provider,
            &profile.email_address,
            profile.display_name.as_deref(),
        )
        .await
    {
        Ok(id) => id,
        Err(StorageError::Conflict) => {
            return Err(format!(
                "The {} account {} is already linked",
                config.provider_label, profile.email_address
            )
            .into());
        }
        Err(e) => return Err(e.into()),
    };

    // If saving the refresh token fails, roll back the newly created account record to keep state clean.
    if let Err(e) = storage
        .token_store
        .save_refresh_token(account_id, refresh_token)
    {
        let _ = storage.delete_account(account_id).await;
        return Err(Box::new(e));
    }

    Ok(account_id)
}

/// Initiates the OAuth linking flow for a Google / Gmail account.
///
/// Requests `gmail.readonly` access with offline consent to guarantee a refresh token is returned.
pub async fn link_gmail_account(storage: &Storage) -> Result<Uuid, BoxError> {
    let provider = GmailProvider::new(reqwest::Client::new());
    link_account_with_config(
        storage,
        LinkConfig {
            oauth_provider: OAuthProvider::Google,
            app_provider: common::Provider::Gmail,
            bundled_client_id: option_envc!("GOOGLE_CLIENT_ID"),
            bundled_client_secret: option_envc!("GOOGLE_CLIENT_SECRET"),
            scopes: vec![Scope::new(
                "https://www.googleapis.com/auth/gmail.readonly".to_string(),
            )],
            provider_label: "Gmail",
        },
        &provider,
    )
    .await
}

/// Initiates the OAuth linking flow for a Microsoft Outlook / 365 account.
///
/// Requests `Mail.Read`, `User.Read` (mandatory for personal account profiles), and `offline_access`.
pub async fn link_outlook_account(storage: &Storage) -> Result<Uuid, BoxError> {
    let provider = OutlookProvider::new(reqwest::Client::new());

    link_account_with_config(
        storage,
        LinkConfig {
            oauth_provider: OAuthProvider::Microsoft,
            app_provider: common::Provider::Outlook,
            bundled_client_id: option_envc!("MICROSOFT_CLIENT_ID"),
            bundled_client_secret: None,
            scopes: vec![
                Scope::new("https://graph.microsoft.com/Mail.Read".to_string()),
                Scope::new("https://graph.microsoft.com/User.Read".to_string()),
                // offline_access is mandatory for Microsoft Graph to issue a refresh token.
                Scope::new("offline_access".to_string()),
            ],
            provider_label: "Outlook",
        },
        &provider,
    )
    .await
}