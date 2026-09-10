use serde_json::{Value, from_str};

use common::CredentialError;
use storage::Storage;

use super::models::{OAuthProvider, RawCredentials};

/// Extracts the `client_id` and `client_secret` from a downloaded Google `credentials.json` file.
///
/// Expects the standard Google Cloud Console desktop client format with an `"installed"` JSON root.
///
/// # Errors
/// Returns [`CredentialError::MissingGoogleClientId`] or [`CredentialError::MissingGoogleClientSecret`]
/// if either key is missing from the `"installed"` object.
pub fn parse_google_credentials_json(contents: &str) -> Result<(String, String), CredentialError> {
    let raw_json: Value = from_str(contents)?;
    let installed = &raw_json["installed"];

    let client_id = installed["client_id"]
        .as_str()
        .ok_or(CredentialError::MissingGoogleClientId)?
        .to_string();

    let client_secret = installed["client_secret"]
        .as_str()
        .ok_or(CredentialError::MissingGoogleClientSecret)?
        .to_string();

    Ok((client_id, client_secret))
}

/// Resolves OAuth credentials for the specified provider.
///
/// Prioritizes user-configured credentials saved in SQLite settings and Keyring, falling back
/// to compile-time bundled credentials if no user overrides exist.
///
/// # Errors
/// Returns [`CredentialError::MissingClientId`] or [`CredentialError::MissingGoogleClientSecret`]
/// if neither user-supplied nor bundled credentials are available.
pub async fn load_credentials(
    provider: &OAuthProvider,
    storage: &Storage,
    bundled_client_id: Option<&'static str>,
    bundled_client_secret: Option<&'static str>,
) -> Result<RawCredentials, CredentialError> {
    match provider {
        OAuthProvider::Google => {
            let user_credentials = storage.get_user_google_credentials().await?;

            let (client_id, client_secret) = match user_credentials {
                Some((id, secret)) => (id, secret),
                None => (
                    bundled_client_id
                        .ok_or(CredentialError::MissingClientId { provider: "Google" })?
                        .to_string(),
                    bundled_client_secret
                        .ok_or(CredentialError::MissingGoogleClientSecret)?
                        .to_string(),
                ),
            };

            Ok(RawCredentials {
                client_id,
                client_secret: Some(client_secret),
                auth_uri: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                token_uri: "https://oauth2.googleapis.com/token".to_string(),
            })
        }
        OAuthProvider::Microsoft => {
            let user_client_id = storage.get_user_microsoft_client_id().await?;

            let client_id = match user_client_id {
                Some(id) => id,
                None => bundled_client_id
                    .ok_or(CredentialError::MissingClientId {
                        provider: "Microsoft",
                    })?
                    .to_string(),
            };

            let tenant_id = "common";

            Ok(RawCredentials {
                client_id,
                client_secret: None,
                auth_uri: format!(
                    "https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/authorize"
                ),
                token_uri: format!(
                    "https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token"
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_google_credentials() {
        let json = r#"
        {
            "installed": {
                "client_id": "test-client-id",
                "client_secret": "test-client-secret"
            }
        }
        "#;

        let result = parse_google_credentials_json(json);
        let (client_id, client_secret) = result.unwrap();

        assert_eq!(client_id, "test-client-id");
        assert_eq!(client_secret, "test-client-secret");
    }

    #[test]
    fn rejects_invalid_google_credentials() {
        let json = r#"
        {
            "not_google_credentials": true
        }
        "#;

        assert!(parse_google_credentials_json(json).is_err());
    }

    #[test]
    fn rejects_malformed_google_credentials_json() {
        let json = r#"
        {
            "installed": {
                "client_id": "test-client-id",
                "client_secret":
        "#;

        assert!(parse_google_credentials_json(json).is_err());
    }

    #[test]
    fn rejects_google_credentials_without_client_id() {
        let json = r#"
        {
            "installed": {
                "client_secret": "test-client-secret"
            }
        }
        "#;

        assert!(parse_google_credentials_json(json).is_err());
    }

    #[test]
    fn rejects_google_credentials_without_client_secret() {
        let json = r#"
        {
            "installed": {
                "client_id": "test-client-id"
            }
        }
        "#;

        assert!(parse_google_credentials_json(json).is_err());
    }
}