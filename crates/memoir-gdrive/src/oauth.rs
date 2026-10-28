//! Google sign-in for an installed app: Authorization Code + PKCE (RFC 8252)
//! with a loopback redirect (desktop) or a redirect to the app's own URL
//! scheme (iOS), token refresh and revocation.

use std::future::Future;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use url::Url;
use zeroize::Zeroizing;

/// Only files this app created; nothing else in the user's Drive is visible.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
/// How long the browser sign-in may take.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    /// For desktop clients Google treats this as non-confidential; PKCE
    /// protects the flow. iOS clients have none (empty: not sent).
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub revoke_url: String,
}

impl OAuthConfig {
    pub fn google(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            revoke_url: "https://oauth2.googleapis.com/revoke".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OAuthError {
    #[error("sign-in was cancelled")]
    Cancelled,
    #[error("sign-in took too long; try again")]
    TimedOut,
    #[error("Google refused the sign-in: {0}")]
    Denied(String),
    /// The refresh token no longer works (revoked, or expired — Google expires
    /// them after 7 days while the OAuth app is in "Testing").
    #[error("the Google sign-in has expired or was revoked")]
    Expired,
    #[error("network problem during sign-in: {0}")]
    Network(String),
    #[error("unexpected sign-in response: {0}")]
    Protocol(String),
}

/// A short-lived access token.
#[derive(Clone)]
pub struct AccessToken {
    pub token: Zeroizing<String>,
    pub expires_at: Instant,
}

impl AccessToken {
    /// True if it expires within a minute (so it's refreshed a bit early).
    pub fn is_stale(&self) -> bool {
        self.expires_at.saturating_duration_since(Instant::now()) < Duration::from_secs(60)
    }
}

pub struct SignIn {
    pub access: AccessToken,
    /// Long-lived; keep it in the OS keychain.
    pub refresh_token: Zeroizing<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: String,
}

/// Runs the browser sign-in: `open_browser` receives the consent URL; the
/// result arrives on a loopback port. Ends early if `cancel` completes.
pub async fn sign_in(
    config: &OAuthConfig,
    http: &reqwest::Client,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
    cancel: impl Future<Output = ()>,
) -> Result<SignIn, OAuthError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| OAuthError::Network(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| OAuthError::Network(e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");
    authorize(
        config,
        http,
        &redirect_uri,
        open_browser,
        |state| async move { wait_for_redirect(&listener, &state).await },
        cancel,
    )
    .await
}

/// The redirect URI for a Google iOS client: its reversed client ID as a URL
/// scheme (which the app registers), e.g.
/// `com.googleusercontent.apps.123-abc:/oauth2redirect`.
pub fn app_redirect_uri(client_id: &str) -> String {
    let id = client_id.trim_end_matches(".apps.googleusercontent.com");
    format!("com.googleusercontent.apps.{id}:/oauth2redirect")
}

/// Like [`sign_in`], but the browser comes back to the app itself: Google
/// redirects to `redirect_uri` (see [`app_redirect_uri`]), the system opens
/// the app with that URL, and it arrives on `redirects`. Redirects from an
/// earlier, abandoned sign-in (another `state`) are skipped.
pub async fn sign_in_with_app_redirect(
    config: &OAuthConfig,
    http: &reqwest::Client,
    redirect_uri: &str,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
    mut redirects: mpsc::UnboundedReceiver<String>,
    cancel: impl Future<Output = ()>,
) -> Result<SignIn, OAuthError> {
    authorize(
        config,
        http,
        redirect_uri,
        open_browser,
        |state| async move {
            while let Some(url) = redirects.recv().await {
                if let Some(code) = code_from_redirect(&url, &state).transpose() {
                    return code;
                }
            }
            Err(OAuthError::Cancelled)
        },
        cancel,
    )
    .await
}

/// The consent page, then the code exchange. `wait_for_code` gets the
/// expected `state` and resolves to the authorization code.
async fn authorize<F: Future<Output = Result<String, OAuthError>>>(
    config: &OAuthConfig,
    http: &reqwest::Client,
    redirect_uri: &str,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
    wait_for_code: impl FnOnce(String) -> F,
    cancel: impl Future<Output = ()>,
) -> Result<SignIn, OAuthError> {
    let verifier = Zeroizing::new(random_token(48));
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token(24);

    let url = Url::parse_with_params(
        &config.auth_url,
        &[
            ("client_id", config.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("access_type", "offline"),
            // Always ask for consent, so Google issues a refresh token, and
            // which account to use: workspaces live in different accounts,
            // and the browser may be signed in to several.
            ("prompt", "consent select_account"),
        ],
    )
    .map_err(|e| OAuthError::Protocol(e.to_string()))?;
    open_browser(url.as_str()).map_err(OAuthError::Network)?;

    let code = tokio::select! {
        code = wait_for_code(state) => code?,
        () = cancel => return Err(OAuthError::Cancelled),
        () = tokio::time::sleep(SIGN_IN_TIMEOUT) => return Err(OAuthError::TimedOut),
    };

    let response = http
        .post(&config.token_url)
        .form(&with_secret(
            config,
            vec![
                ("code", code.as_str()),
                ("client_id", config.client_id.as_str()),
                ("redirect_uri", redirect_uri),
                ("grant_type", "authorization_code"),
                ("code_verifier", verifier.as_str()),
            ],
        ))
        .send()
        .await
        .map_err(|e| OAuthError::Network(e.to_string()))?;
    let tokens: TokenResponse = token_response(response).await?;
    let refresh_token = tokens
        .refresh_token
        .ok_or_else(|| OAuthError::Protocol("no refresh token issued".into()))?;
    Ok(SignIn {
        access: AccessToken {
            token: Zeroizing::new(tokens.access_token),
            expires_at: Instant::now() + Duration::from_secs(tokens.expires_in),
        },
        refresh_token: Zeroizing::new(refresh_token),
    })
}

/// Exchanges the refresh token for a new access token.
pub async fn refresh(
    config: &OAuthConfig,
    http: &reqwest::Client,
    refresh_token: &str,
) -> Result<AccessToken, OAuthError> {
    let response = http
        .post(&config.token_url)
        .form(&with_secret(
            config,
            vec![
                ("refresh_token", refresh_token),
                ("client_id", config.client_id.as_str()),
                ("grant_type", "refresh_token"),
            ],
        ))
        .send()
        .await
        .map_err(|e| OAuthError::Network(e.to_string()))?;
    let tokens: TokenResponse = token_response(response).await?;
    Ok(AccessToken {
        token: Zeroizing::new(tokens.access_token),
        expires_at: Instant::now() + Duration::from_secs(tokens.expires_in),
    })
}

/// Tells Google to invalidate the token (on disconnect). Best effort.
pub async fn revoke(
    config: &OAuthConfig,
    http: &reqwest::Client,
    token: &str,
) -> Result<(), OAuthError> {
    let response = http
        .post(&config.revoke_url)
        .form(&[("token", token)])
        .send()
        .await
        .map_err(|e| OAuthError::Network(e.to_string()))?;
    if response.status().is_success() || response.status().as_u16() == 400 {
        Ok(()) // 400: already invalid
    } else {
        Err(OAuthError::Protocol(format!(
            "revoke returned {}",
            response.status()
        )))
    }
}

/// Token request parameters plus the client secret, if the client has one
/// (Google rejects an empty one from an iOS client).
fn with_secret<'a>(
    config: &'a OAuthConfig,
    mut form: Vec<(&'a str, &'a str)>,
) -> Vec<(&'a str, &'a str)> {
    if !config.client_secret.is_empty() {
        form.push(("client_secret", config.client_secret.as_str()));
    }
    form
}

async fn token_response(response: reqwest::Response) -> Result<TokenResponse, OAuthError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| OAuthError::Network(e.to_string()))?;
    if status.is_success() {
        return serde_json::from_str(&body).map_err(|e| OAuthError::Protocol(e.to_string()));
    }
    match serde_json::from_str::<TokenError>(&body) {
        Ok(e) if e.error == "invalid_grant" => Err(OAuthError::Expired),
        Ok(e) => Err(OAuthError::Denied(format!(
            "{}: {}",
            e.error, e.error_description
        ))),
        Err(_) if status.is_server_error() => Err(OAuthError::Network(format!(
            "token endpoint returned {status}"
        ))),
        Err(_) => Err(OAuthError::Protocol(format!(
            "token endpoint returned {status}"
        ))),
    }
}

/// Serves the OAuth redirect on the loopback listener; returns the code.
async fn wait_for_redirect(
    listener: &TcpListener,
    expected_state: &str,
) -> Result<String, OAuthError> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| OAuthError::Network(e.to_string()))?;
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        let Some(target) = request
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("GET "))
            .and_then(|l| l.split(' ').next())
        else {
            continue;
        };
        let Ok(url) = Url::parse(&format!("http://localhost{target}")) else {
            continue;
        };
        let param = |k: &str| {
            url.query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
        };

        // Browsers also ask for /favicon.ico etc.
        if param("code").is_none() && param("error").is_none() {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            continue;
        }

        let ok = param("error").is_none() && param("state").as_deref() == Some(expected_state);
        let message = if ok {
            "Memoir is now connected to Google Drive. You can close this tab."
        } else {
            "Memoir could not connect to Google Drive. You can close this tab and try again."
        };
        let body = format!(
            "<!doctype html><meta charset=utf-8><title>Memoir</title>\
             <body style=\"font:16px system-ui;padding:40px\">{message}</body>"
        );
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await;

        if let Some(err) = param("error") {
            return Err(OAuthError::Denied(err));
        }
        if param("state").as_deref() != Some(expected_state) {
            return Err(OAuthError::Protocol("state mismatch".into()));
        }
        return param("code").ok_or_else(|| OAuthError::Protocol("no code".into()));
    }
}

/// The authorization code from the URL Google redirected to; None if the
/// redirect belongs to another sign-in (its `state` differs).
fn code_from_redirect(
    redirected: &str,
    expected_state: &str,
) -> Result<Option<String>, OAuthError> {
    let url = Url::parse(redirected).map_err(|e| OAuthError::Protocol(e.to_string()))?;
    let param = |k: &str| {
        url.query_pairs()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.into_owned())
    };
    if param("state").as_deref() != Some(expected_state) {
        return Ok(None);
    }
    if let Some(err) = param("error") {
        return Err(OAuthError::Denied(err));
    }
    param("code")
        .map(Some)
        .ok_or_else(|| OAuthError::Protocol("no code".into()))
}

fn random_token(bytes: usize) -> String {
    let mut buf = Zeroizing::new(vec![0u8; bytes]);
    getrandom::fill(&mut buf).expect("OS random number generator");
    URL_SAFE_NO_PAD.encode(&*buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_redirect_uri_is_the_reversed_client_id() {
        assert_eq!(
            app_redirect_uri("123-abc.apps.googleusercontent.com"),
            "com.googleusercontent.apps.123-abc:/oauth2redirect"
        );
    }

    #[test]
    fn code_from_redirect_checks_state() {
        let uri = app_redirect_uri("123-abc.apps.googleusercontent.com");
        assert_eq!(
            code_from_redirect(&format!("{uri}?state=s1&code=c1"), "s1"),
            Ok(Some("c1".into()))
        );
        assert_eq!(
            code_from_redirect(&format!("{uri}?state=other&code=c1"), "s1"),
            Ok(None)
        );
        assert_eq!(
            code_from_redirect(&format!("{uri}?error=access_denied&state=other"), "s1"),
            Ok(None)
        );
        assert_eq!(
            code_from_redirect(&format!("{uri}?error=access_denied&state=s1"), "s1"),
            Err(OAuthError::Denied("access_denied".into()))
        );
    }

    #[test]
    fn empty_client_secret_is_not_sent() {
        let ios = OAuthConfig::google("id", "");
        assert!(
            !with_secret(&ios, vec![])
                .iter()
                .any(|(k, _)| *k == "client_secret")
        );
        let desktop = OAuthConfig::google("id", "secret");
        assert_eq!(
            with_secret(&desktop, vec![]),
            vec![("client_secret", "secret")]
        );
    }
}
