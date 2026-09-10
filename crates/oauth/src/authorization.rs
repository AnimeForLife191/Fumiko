use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, RedirectUrl, Scope, TokenUrl,
    basic::BasicClient,
    reqwest::{self, ClientBuilder},
    url::Url,
};
use std::{
    io::{BufRead, BufReader, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use super::models::{OAuthProvider, RawCredentials, TokenSet};
use super::tokens::response_to_token_set;
use common::OAuthError;

pub type OAuthClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

pub const REDIRECT_PATH: &str = "/";

/// RAII Guard that ensures the blocking loopback listener is immediately
/// unblocked and terminated if the `run_oauth` future is cancelled or dropped.
struct ListenerGuard {
    cancel: Arc<AtomicBool>,
    port: u16,
    completed: bool,
}

impl Drop for ListenerGuard {
    fn drop(&mut self) {
        if !self.completed {
            cancel_listener(self.cancel.clone(), self.port);
        }
    }
}

pub(crate) fn build_oauth_client(
    credentials: RawCredentials,
    redirect_port: u16,
) -> Result<OAuthClient, OAuthError> {
    let client_id = ClientId::new(credentials.client_id);
    let auth_url = AuthUrl::new(credentials.auth_uri)
        .map_err(|e| OAuthError::InvalidEndpointConfig(Box::new(e)))?;
    let token_url = TokenUrl::new(credentials.token_uri)
        .map_err(|e| OAuthError::InvalidEndpointConfig(Box::new(e)))?;

    // RFC 8252 §7.3 specifies loopback redirects using numeric 127.0.0.1 to avoid
    // operating systems resolving 'localhost' to IPv6 ::1 where no listener is bound.
    let redirect_url = format!("http://127.0.0.1:{redirect_port}{REDIRECT_PATH}");

    let mut client = BasicClient::new(client_id)
        .set_auth_uri(auth_url)
        .set_token_uri(token_url)
        .set_redirect_uri(
            RedirectUrl::new(redirect_url)
                .map_err(|e| OAuthError::InvalidEndpointConfig(Box::new(e)))?,
        );

    if let Some(secret) = credentials.client_secret {
        client = client.set_client_secret(ClientSecret::new(secret));
    }

    Ok(client)
}

/// Executes a complete desktop OAuth 2.0 authorization code flow with PKCE.
///
/// Binds a temporary `TcpListener` to an OS-assigned ephemeral port (`127.0.0.1:0`), opens the
/// system's default browser to the provider's sign-in screen, captures the redirect callback,
/// validates the CSRF state in constant time, and exchanges the authorization code for tokens.
///
/// # Invariants & Security
/// - The local listener runs strictly on `127.0.0.1` and enforces a 60-second overall timeout.
/// - Unrelated network traffic (such as favicon requests or port scans) is served a 404 and discarded
///   without terminating the waiting listener.
///
/// # Errors
/// Returns [`OAuthError::CallbackTimeout`] if the user does not finish within 60 seconds,
/// [`OAuthError::AccessDenied`] if the user cancels on the consent screen, or
/// [`OAuthError::CsrfMismatch`] if the returned state token does not match the request.
pub async fn run_oauth(
    credentials: RawCredentials,
    provider: &OAuthProvider,
    scopes: Vec<Scope>,
) -> Result<TokenSet, OAuthError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| OAuthError::BrowserLaunchFailed(Box::new(e)))?;

    let redirect_port = listener
        .local_addr()
        .map_err(|e| OAuthError::BrowserLaunchFailed(Box::new(e)))?
        .port();

    let oauth_client = build_oauth_client(credentials, redirect_port)?;

    let (pkce_code_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = oauth_client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(pkce_code_challenge);

    for scope in scopes {
        request = request.add_scope(scope);
    }

    for (name, value) in provider.extra_params() {
        request = request.add_extra_param(*name, *value);
    }

    let (auth_url, csrf_token) = request.url();

    let cancel = Arc::new(AtomicBool::new(false));

    let mut guard = ListenerGuard {
        cancel: cancel.clone(),
        port: redirect_port,
        completed: false,
    };

    let callback_task = {
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || {
            receive_authorization_code(listener, redirect_port, cancel)
        })
    };

    if let Err(e) = webbrowser::open(auth_url.as_str()) {
        return Err(OAuthError::BrowserLaunchFailed(Box::new(e)));
    }

    tokio::pin!(callback_task);

    let callback_result = tokio::time::timeout(Duration::from_secs(60), &mut callback_task).await;

    let (code, returned_state) = match callback_result {
        Ok(Ok(inner)) => {
            guard.completed = true;
            inner?
        }
        Ok(Err(join_error)) => return Err(OAuthError::CallbackTaskFailed(Box::new(join_error))),
        Err(_elapsed) => {
            return Err(OAuthError::CallbackTimeout);
        }
    };

    validate_csrf_state(&csrf_token, &returned_state)?;

    let http_client = ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| OAuthError::Network(Box::new(e)))?;

    let token_result = oauth_client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await
        .map_err(|e| OAuthError::TokenExchange(Box::new(e)))?;

    let tokens = response_to_token_set(&token_result, None);

    Ok(tokens)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (&x, &y)| acc | (x ^ y))
        == 0
}

fn validate_csrf_state(expected: &CsrfToken, returned: &str) -> Result<(), OAuthError> {
    if !constant_time_eq(expected.secret().as_bytes(), returned.as_bytes()) {
        return Err(OAuthError::CsrfMismatch);
    }
    Ok(())
}

fn receive_authorization_code(
    listener: TcpListener,
    port: u16,
    cancel: Arc<AtomicBool>,
) -> Result<(String, String), OAuthError> {
    loop {
        let (stream, _) = listener
            .accept()
            .map_err(|e| OAuthError::ListenerBindFailed(Box::new(e)))?;

        if cancel.load(Ordering::Relaxed) {
            return Err(OAuthError::Cancelled);
        }

        if let Some(result) = handle_connection(stream, port)? {
            return Ok(result);
        }
    }
}

pub(crate) fn cancel_listener(cancel: Arc<AtomicBool>, port: u16) {
    cancel.store(true, Ordering::Relaxed);
    let _ = TcpStream::connect(("127.0.0.1", port));
}

fn handle_connection(
    mut stream: TcpStream,
    port: u16,
) -> Result<Option<(String, String)>, OAuthError> {
    // Bound socket timeouts so silent or slow TCP connections don't hang the single worker thread.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return Ok(None);
    }

    let Some(path) = request_line.split_whitespace().nth(1) else {
        return Ok(None);
    };

    let parse_result = parse_callback_target(port, path);

    let (status_line, body) = match &parse_result {
        Ok(_) => (
            "HTTP/1.1 200 OK",
            "<html><body>Authentication complete &mdash; you can close this tab \
                and return to the app.</body></html>",
        ),
        Err(OAuthError::InvalidCallbackTarget) => (
            "HTTP/1.1 404 Not Found",
            "<html><body>Not found.</body></html>",
        ),
        Err(_) => (
            "HTTP/1.1 400 Bad Request",
            "<html><body>Authorization did not complete &mdash; you can close this tab \
                and return to the app.</body></html>",
        ),
    };

    let response = format!(
        "{status_line}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );

    // Flush and cleanly shut down the connection so browsers don't surface an early TCP RST.
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);

    match parse_result {
        Ok((code, state)) => Ok(Some((code, state))),
        Err(e @ OAuthError::AccessDenied(_)) => Err(e),
        Err(e @ OAuthError::ProviderCallbackError(_)) => Err(e),
        Err(_) => Ok(None),
    }
}

fn parse_callback_target(port: u16, target: &str) -> Result<(String, String), OAuthError> {
    let full_url = format!("http://127.0.0.1:{port}{target}");
    let parsed = Url::parse(&full_url).map_err(|_| OAuthError::InvalidCallbackTarget)?;

    if parsed.path() != REDIRECT_PATH {
        return Err(OAuthError::InvalidCallbackTarget);
    }

    if let Some((_, error)) = parsed.query_pairs().find(|(key, _)| key == "error") {
        let description = parsed
            .query_pairs()
            .find(|(key, _)| key == "error_description")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_else(|| error.clone().into_owned());

        return Err(if error == "access_denied" {
            OAuthError::AccessDenied(description)
        } else {
            OAuthError::ProviderCallbackError(description)
        });
    }

    let code = parsed
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .ok_or(OAuthError::MissingAuthorizationCode)?;

    if code.trim().is_empty() {
        return Err(OAuthError::EmptyAuthorizationCode);
    }

    let state = parsed
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or(OAuthError::MissingState)?;

    Ok((code, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_client_with_public_credentials() {
        let credentials = RawCredentials {
            client_id: "client-id".to_string(),
            client_secret: None,
            auth_uri: "https://example.com/authorize".to_string(),
            token_uri: "https://example.com/token".to_string(),
        };

        assert!(build_oauth_client(credentials, 9876).is_ok());
    }

    #[test]
    fn builds_client_with_confidential_credentials() {
        let credentials = RawCredentials {
            client_id: "client-id".to_string(),
            client_secret: Some("shh".to_string()),
            auth_uri: "https://example.com/authorize".to_string(),
            token_uri: "https://example.com/token".to_string(),
        };

        assert!(build_oauth_client(credentials, 9876).is_ok());
    }

    #[test]
    fn rejects_malformed_auth_uri() {
        let credentials = RawCredentials {
            client_id: "client-id".to_string(),
            client_secret: None,
            auth_uri: "not a url".to_string(),
            token_uri: "https://example.com/token".to_string(),
        };

        assert!(build_oauth_client(credentials, 9876).is_err());
    }

    #[test]
    fn rejects_malformed_token_uri() {
        let credentials = RawCredentials {
            client_id: "client-id".to_string(),
            client_secret: None,
            auth_uri: "https://example.com/authorize".to_string(),
            token_uri: "not a url".to_string(),
        };

        assert!(build_oauth_client(credentials, 9876).is_err());
    }

    #[test]
    fn builds_different_redirect_uris_for_different_ports() {
        let creds = || RawCredentials {
            client_id: "client-id".to_string(),
            client_secret: None,
            auth_uri: "https://example.com/authorize".to_string(),
            token_uri: "https://example.com/token".to_string(),
        };

        let client_a = build_oauth_client(creds(), 9876).unwrap();
        let client_b = build_oauth_client(creds(), 9877).unwrap();

        assert_ne!(
            client_a.redirect_uri().map(|u| u.as_str().to_string()),
            client_b.redirect_uri().map(|u| u.as_str().to_string())
        );
    }

    #[test]
    fn rejects_mismatched_csrf_state() {
        let expected = CsrfToken::new("expected-state".to_string());
        let result = validate_csrf_state(&expected, "different-state");
        assert!(matches!(result, Err(OAuthError::CsrfMismatch)));
    }

    #[test]
    fn accepts_matching_csrf_state() {
        let expected = CsrfToken::new("expected-state".to_string());
        assert!(validate_csrf_state(&expected, "expected-state").is_ok());
    }

    #[test]
    fn csrf_state_comparison_is_case_sensitive() {
        let expected = CsrfToken::new("Expected-State".to_string());
        let result = validate_csrf_state(&expected, "expected-state");
        assert!(matches!(result, Err(OAuthError::CsrfMismatch)));
    }

    #[test]
    fn parses_valid_callback() {
        let result = parse_callback_target(9876, "/?code=abc123&state=expected-state");
        assert_eq!(
            result.unwrap(),
            ("abc123".to_string(), "expected-state".to_string())
        );
    }

    #[test]
    fn decodes_url_encoded_callback_parameters() {
        let result = parse_callback_target(9876, "/?code=abc%20123&state=state%2Bvalue");
        assert_eq!(
            result.unwrap(),
            ("abc 123".to_string(), "state+value".to_string())
        );
    }

    #[test]
    fn ignores_extraneous_query_parameters() {
        let result = parse_callback_target(
            9876,
            "/?code=abc123&state=expected-state&session_state=xyz&scope=Mail.Read",
        );
        assert_eq!(
            result.unwrap(),
            ("abc123".to_string(), "expected-state".to_string())
        );
    }

    #[test]
    fn detects_access_denied_callback() {
        let result = parse_callback_target(
            9876,
            "/?error=access_denied&error_description=User+denied+access",
        );
        assert!(
            matches!(result, Err(OAuthError::AccessDenied(msg)) if msg == "User denied access")
        );
    }

    #[test]
    fn access_denied_takes_priority_over_missing_code() {
        let result = parse_callback_target(9876, "/?error=access_denied&state=expected-state");
        assert!(matches!(result, Err(OAuthError::AccessDenied(_))));
    }

    #[test]
    fn falls_back_to_raw_error_code_when_description_missing() {
        let result = parse_callback_target(9876, "/?error=access_denied");
        assert!(matches!(result, Err(OAuthError::AccessDenied(msg)) if msg == "access_denied"));
    }

    #[test]
    fn decodes_url_encoded_error_description() {
        let result = parse_callback_target(
            9876,
            "/?error=access_denied&error_description=The%20user%20cancelled%20the%20flow.",
        );
        assert!(
            matches!(result, Err(OAuthError::AccessDenied(msg)) if msg == "The user cancelled the flow.")
        );
    }

    #[test]
    fn rejects_wrong_callback_path() {
        let result = parse_callback_target(9876, "/unexpected?code=abc123&state=expected-state");
        assert!(matches!(result, Err(OAuthError::InvalidCallbackTarget)));
    }

    #[test]
    fn rejects_callback_without_code() {
        let result = parse_callback_target(9876, "/?state=expected-state");
        assert!(matches!(result, Err(OAuthError::MissingAuthorizationCode)));
    }

    #[test]
    fn rejects_callback_without_state() {
        let result = parse_callback_target(9876, "/?code=abc123");
        assert!(matches!(result, Err(OAuthError::MissingState)));
    }

    #[test]
    fn rejects_callback_with_no_query_string() {
        let result = parse_callback_target(9876, "/");
        assert!(matches!(result, Err(OAuthError::MissingAuthorizationCode)));
    }

    #[test]
    fn rejects_empty_code_value() {
        let result = parse_callback_target(9876, "/?code=&state=expected-state");
        assert!(matches!(result, Err(OAuthError::EmptyAuthorizationCode)));
    }

    #[test]
    fn handles_duplicate_query_parameters_by_taking_first_match() {
        let result = parse_callback_target(9876, "/?code=first&code=second&state=expected-state");
        assert_eq!(result.unwrap().0, "first");
    }

    #[test]
    fn stops_listening_once_cancelled() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        let handle =
            std::thread::spawn(move || receive_authorization_code(listener, port, cancel_clone));

        std::thread::sleep(Duration::from_millis(50));
        cancel_listener(cancel, port);

        let result = handle.join().unwrap();
        assert!(matches!(result, Err(OAuthError::Cancelled)));
    }

    #[test]
    fn unrelated_path_does_not_terminate_the_listener() {
        let result = parse_callback_target(9876, "/favicon.ico");
        assert!(matches!(result, Err(OAuthError::InvalidCallbackTarget)));
    }
}
