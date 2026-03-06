use anyhow::{Context, Result};
use std::{env, net::SocketAddr};

pub const TEST_GITHUB_WEBHOOK_SECRET: &str = "mobench-webhook-test-secret";

#[derive(Clone, Debug)]
pub struct Config {
    pub public_http_addr: SocketAddr,
    pub github_webhook_secret: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let public_http_addr = env::var("PUBLIC_HTTP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()
            .context("parsing PUBLIC_HTTP_ADDR")?;
        let github_webhook_secret = env::var("GITHUB_WEBHOOK_SECRET")
            .unwrap_or_else(|_| "mobench-webhook-dev-secret".to_string());

        Ok(Self {
            public_http_addr,
            github_webhook_secret,
        })
    }

    pub fn for_test() -> Self {
        Self {
            public_http_addr: "127.0.0.1:0".parse().expect("valid test listen address"),
            github_webhook_secret: TEST_GITHUB_WEBHOOK_SECRET.to_string(),
        }
    }
}
