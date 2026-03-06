use anyhow::{Context, Result};
use std::{env, net::SocketAddr};

#[derive(Clone, Debug)]
pub struct Config {
    pub public_http_addr: SocketAddr,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let public_http_addr = env::var("PUBLIC_HTTP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()
            .context("parsing PUBLIC_HTTP_ADDR")?;

        Ok(Self { public_http_addr })
    }
}
