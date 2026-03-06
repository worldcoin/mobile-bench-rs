use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::RwLock;

use crate::config::{Config, TEST_GITHUB_APP_PRIVATE_KEY_PEM};

const USER_AGENT: &str = "mobench-webhook";
const TOKEN_REFRESH_WINDOW: Duration = Duration::minutes(5);

#[derive(Clone)]
pub struct GitHubAppAuth {
    inner: Arc<GitHubAppAuthInner>,
}

struct GitHubAppAuthInner {
    app_id: u64,
    installation_id: u64,
    private_key_pem: String,
    api_base_url: String,
    http: Client,
    cached: RwLock<Option<CachedToken>>,
    installation_requests: AtomicUsize,
}

#[derive(Clone)]
struct CachedToken {
    token: String,
    expires_at: OffsetDateTime,
}

impl GitHubAppAuth {
    pub fn new(config: &Config) -> Result<Self> {
        Self::from_parts(
            config.github_api_base_url.clone(),
            config.github_app_id,
            config.github_installation_id,
            config.github_private_key_pem.clone(),
        )
    }

    pub fn from_parts(
        api_base_url: String,
        app_id: u64,
        installation_id: u64,
        private_key_pem: String,
    ) -> Result<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .context("building GitHub App HTTP client")?;

        Ok(Self {
            inner: Arc::new(GitHubAppAuthInner {
                app_id,
                installation_id,
                private_key_pem,
                api_base_url: api_base_url.trim_end_matches('/').to_string(),
                http,
                cached: RwLock::new(None),
                installation_requests: AtomicUsize::new(0),
            }),
        })
    }

    pub fn for_test(api_base_url: impl Into<String>) -> Self {
        Self::from_parts(
            api_base_url.into(),
            1,
            2,
            TEST_GITHUB_APP_PRIVATE_KEY_PEM.to_string(),
        )
        .expect("test GitHub auth should initialize")
    }

    pub async fn installation_token(&self) -> Result<String> {
        {
            let cached = self.inner.cached.read().await;
            if let Some(cached) = cached.as_ref() {
                if cached.expires_at > OffsetDateTime::now_utc() + TOKEN_REFRESH_WINDOW {
                    return Ok(cached.token.clone());
                }
            }
        }

        let mut cached = self.inner.cached.write().await;
        if let Some(cached_token) = cached.as_ref() {
            if cached_token.expires_at > OffsetDateTime::now_utc() + TOKEN_REFRESH_WINDOW {
                return Ok(cached_token.token.clone());
            }
        }

        let refreshed = self.fetch_installation_token().await?;
        let token = refreshed.token.clone();
        *cached = Some(refreshed);

        Ok(token)
    }

    pub async fn test_installation_request_count(&self) -> usize {
        self.inner.installation_requests.load(Ordering::Relaxed)
    }

    async fn fetch_installation_token(&self) -> Result<CachedToken> {
        self.inner
            .installation_requests
            .fetch_add(1, Ordering::Relaxed);

        let jwt = self.app_jwt()?;
        let response = self
            .inner
            .http
            .post(format!(
                "{}/app/installations/{}/access_tokens",
                self.inner.api_base_url, self.inner.installation_id
            ))
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(jwt)
            .send()
            .await
            .context("requesting GitHub installation token")?
            .error_for_status()
            .context("GitHub installation token request failed")?;
        let payload: InstallationTokenResponse = response
            .json()
            .await
            .context("decoding GitHub installation token response")?;
        let expires_at = OffsetDateTime::parse(&payload.expires_at, &Rfc3339)
            .context("parsing installation token expiry")?;

        Ok(CachedToken {
            token: payload.token,
            expires_at,
        })
    }

    fn app_jwt(&self) -> Result<String> {
        let now = OffsetDateTime::now_utc();
        let claims = AppJwtClaims {
            iat: (now - Duration::seconds(60)).unix_timestamp(),
            exp: (now + Duration::minutes(9)).unix_timestamp(),
            iss: self.inner.app_id,
        };
        let key = EncodingKey::from_rsa_pem(self.inner.private_key_pem.as_bytes())
            .context("parsing GitHub App private key PEM")?;

        encode(&Header::new(Algorithm::RS256), &claims, &key).context("signing GitHub App JWT")
    }
}

#[derive(Debug, Serialize)]
struct AppJwtClaims {
    iat: i64,
    exp: i64,
    iss: u64,
}

#[derive(Debug, Deserialize)]
struct InstallationTokenResponse {
    token: String,
    expires_at: String,
}
