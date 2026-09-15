//! Mailbox linking coordination for OAuth and IMAP accounts.

use crate::email_providers::ImapProvider;

use super::EmailProvider;
use super::email_providers::{GmailProvider, OutlookProvider};
use common::{BoxError, Provider};
use envcrypt::option_envc;
use oauth::{OAuthProvider, load_credentials, run_oauth};
use oauth2::Scope;
use storage::{Storage, StorageError};
use uuid::Uuid;

/// Configuration bundle for establishing an OAuth account link.
struct LinkConfig {
    oauth_provider: OAuthProvider,
    app_provider: Provider,
    bundled_client_id: Option<&'static str>,
    bundled_client_secret: Option<&'static str>,
    scopes: Vec<Scope>,
    provider_label: &'static str,
}

/// Coordinates OAuth authentication, user profile discovery, and atomic storage registration.
async fn link_account_with_config<P>(
    storage: &Storage,
    config: LinkConfig,
    provider: &P,
) -> Result<Uuid, BoxError>
where
    P: EmailProvider,
{
    // 1. Resolve client credentials from SQLite settings, Keyring, or compile-time defaults
    let credentials = load_credentials(
        &config.oauth_provider,
        storage,
        config.bundled_client_id,
        config.bundled_client_secret,
    )
    .await?;

    // 2. Execute ephemeral loopback PKCE flow
    let tokens = run_oauth(credentials, &config.oauth_provider, config.scopes).await?;

    // Invariant: An account without a refresh token cannot be automatically synchronized in
    // the background. Validate presence before creating database records to prevent orphaned accounts.
    let refresh_token = tokens
        .refresh_token
        .as_ref()
        .ok_or_else(|| BoxError::from("No refresh token was provided by OAuth provider"))?;

    // 3. Hydrate remote user profile details
    let profile = provider.get_profile(&tokens.access_token).await?;

    // 4. Register account metadata in SQLite
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

    // 5. Commit refresh token to OS Keyring; roll back SQLite row if keyring write fails
    if let Err(e) = storage
        .token_store
        .save_account_secret(account_id, refresh_token)
    {
        let _ = storage.delete_account(account_id).await;
        return Err(Box::new(e));
    }

    Ok(account_id)
}

/// Initiates the OAuth linking flow for a Google / Gmail account.
///
/// Requests `gmail.readonly` access with offline consent to guarantee a refresh token is returned.
///
/// # Errors
/// Returns a [`BoxError`] if authorization fails, the user denies consent,
/// or the account is already linked.
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

/// Initiates the OAuth linking flow for a Microsoft Outlook / Office 365 account.
///
/// Requests `Mail.Read`, `User.Read` (mandatory for profile queries on personal Microsoft accounts),
/// and `offline_access`.
///
/// # Errors
/// Returns a [`BoxError`] if authorization fails, the user denies consent,
/// or the account is already linked.
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

/// Verifies IMAP credentials against the remote server, stores account metadata,
/// and commits the App Password to the OS Keyring.
///
/// Normalizes app passwords by stripping whitespace groupings generated by Apple and Google.
///
/// # Errors
/// Returns a [`BoxError`] if the TLS handshake fails, login authentication is rejected,
/// or the account is already linked.
pub async fn link_imap_account(
    storage: &Storage,
    email: &str,
    password: &str,
    host: &str,
    port: u16,
) -> Result<Uuid, BoxError> {
    // 1. Normalize password by stripping space groupings (e.g. "abcd efgh ijkl mnop")
    let clean_password: String = password.chars().filter(|c| !c.is_whitespace()).collect();
    let provider = ImapProvider::new(host, port, email);

    // 2. Pre-flight login verification before writing records to storage
    provider.get_profile(&clean_password).await?;

    // 3. Register account in SQLite
    let account_id = match storage
        .add_imap_account(Provider::Imap, email, None, host, port)
        .await
    {
        Ok(id) => id,
        Err(StorageError::Conflict) => {
            return Err(format!("The account {email} is already linked").into());
        }
        Err(e) => return Err(e.into()),
    };

    // 4. Save App Password to OS Keyring; roll back SQLite row on failure
    if let Err(e) = storage
        .token_store
        .save_account_secret(account_id, &clean_password)
    {
        let _ = storage.delete_account(account_id).await;
        return Err(Box::new(e));
    }

    Ok(account_id)
}