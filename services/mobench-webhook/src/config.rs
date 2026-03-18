use anyhow::{Context, Result};
use std::{env, net::SocketAddr};

pub const TEST_GITHUB_WEBHOOK_SECRET: &str = "mobench-webhook-test-secret";
pub const TEST_GITHUB_APP_PRIVATE_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCS8rRhupl8hYey
/UHyd8vqKgR6SiOqRTPWYBhVynYyGiO0Xdj+HhNUIp4lVvarJouOjuZ49H1DseK6
HUPg+XLqP1rjX1fvLLW+gQbSpu1heAhbSEqGg7DJDZtOsSmm1mj7hAZF67NPm6CI
neO83v71p0aBOM9iJWxnCBvx8Qn1R7ptSfXyIs9tSiehV4QKALit9Age32vEg4js
xN0yeMGhOysp8xtE8qsYDtvXNEYTrm0K7PbUrjWqOJf1mkO9W1yMT2GC/xReLBC1
MoEclYqw/av1WAj4kJrPuK98D9oD0IZPP/vkg9z5crV0ZRpoOW5tg6hn66qGurE6
z3Bx1KY5AgMBAAECggEALqcUQoSx7vkbvGUwNzD2Ucj2M/buvMOMsg4/G5mUDdMD
q+MxnXfh7g+xgqxJ0suBegh/Pj5suH20vB7Hapj3dUwY6F/gNIROzQT8rAsoCQ5J
JOXeFzee/C2wNXsEfL3MhbGEJlWuz2LaxBTQdSmc61OojCUDnibAdDN8X8MFRNP+
LIxWYoT09hDJVu5hbKi8dwJP/8IU94rj5zMS2tgQ+FV6j3cdQefBF0ImFYRvHPDB
GsxI+KLdNqumAjYrSwaL3REhauKqOZGDeztfT2oQNd5Bsa6NUWdFPBGP4c+FhP3n
i2pfpzyBcFv20+YC73QK2LC8rBurwTWMZIkPn9qy6wKBgQDJNnGStqqz+wc5dp5q
FrpfWQxZ0qhzOy8yZkJ5Hxdf2o2FZ59hCGWAw+Q48fZz+PVgNrZOjQxJiWgKVrUT
SCaKOvCxnC/xswX6km1vmbnwwEUhH7R+43bYZM+tYOhAS+s/j/Wqu6t9NWVqwY4u
QJOsoYctk34tFEZpKVO6KVf44wKBgQC69b1DvguywKu6wjxvNZWb2wUJqLhfsLv5
VcFw+jByF5uE4kcrLfDCRmVFsRj5qa0+d9sk6A1WOU0PThGxNxejYJVqMacEcoq/
8cDJQNfUI2tVvO+SLcyslIyFVAMPZGWAHbq/rXEbEadKZIcMRkWkTssnxkrhiMHj
BAkkjrN7MwKBgCddxX6kymUIGfO+J2QuKix5aZcxRc+1ppg/tYWo9ZPFWKbfDSmY
0PMOYFpbzJIIBUnbqnNX2S2R+o77Q9YP6aplua2mFyM8mVDa0RpigyR9GYlPgKrK
qffQUWlkakwaDl6TNcc/jF+k0MDAOacG7115BM9/6dG8A8KTWVQ0vodtAoGBALdv
FlSZ6UZoDdY+byc57uEtQkMY3FVexP+86P/dKZ03lmoJzYQLHhavFPwM55FFrmoF
FOmgGD3KGNZ0ZZ13jWTKBa9cqh8N9x6epuWPDnPgkJZdury03QVS9pB2Pk5i1cee
47TfCoNhjb7K5UngxMGSYXdT8fJYyMrhEcthi63LAoGAeVquBKvsD58ERE6JoWz+
AU139pm3xGx9A/7OEs3jlYirJSpPL4M6p/GXTJD+FwpShcDkGFQPUOgXS3X/6N3h
QRHYVy6opPLRvtLFgmuB0mLhSfeukPtEXnKqPYJiKgXUUGfhYrHtmJt9g6ujARV5
FB+EWpNAfyRm2yyamT6/epQ=
-----END PRIVATE KEY-----"#;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub public_http_addr: SocketAddr,
    pub private_http_addr: SocketAddr,
    pub delivery_retry_limit: i32,
    pub delivery_claim_timeout_secs: i32,
    pub github_webhook_secret: String,
    pub github_api_base_url: String,
    pub github_app_id: u64,
    pub github_installation_id: u64,
    pub github_private_key_pem: String,
    pub github_owner: String,
    pub github_repo: String,
    pub github_workflow_id: String,
    pub github_compile_gate_workflow_id: String,
}

impl Config {
    pub fn matches_repo(&self, owner: &str, repo: &str) -> bool {
        self.github_owner.eq_ignore_ascii_case(owner) && self.github_repo.eq_ignore_ascii_case(repo)
    }

    pub fn from_env() -> Result<Self> {
        let database_url = env::var("DATABASE_URL").unwrap_or_default();
        let public_http_addr = env::var("PUBLIC_HTTP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()
            .context("parsing PUBLIC_HTTP_ADDR")?;
        let private_http_addr = env::var("PRIVATE_HTTP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8081".to_string())
            .parse()
            .context("parsing PRIVATE_HTTP_ADDR")?;
        let delivery_retry_limit = env::var("DELIVERY_RETRY_LIMIT")
            .unwrap_or_else(|_| "5".to_string())
            .parse()
            .context("parsing DELIVERY_RETRY_LIMIT")?;
        let delivery_claim_timeout_secs = env::var("DELIVERY_CLAIM_TIMEOUT_SECS")
            .unwrap_or_else(|_| "300".to_string())
            .parse()
            .context("parsing DELIVERY_CLAIM_TIMEOUT_SECS")?;
        let github_webhook_secret = env::var("GITHUB_WEBHOOK_SECRET")
            .unwrap_or_else(|_| "mobench-webhook-dev-secret".to_string());
        let github_api_base_url = env::var("GITHUB_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.github.com".to_string());
        let github_app_id = env::var("GITHUB_APP_ID")
            .unwrap_or_else(|_| "0".to_string())
            .parse()
            .context("parsing GITHUB_APP_ID")?;
        let github_installation_id = env::var("GITHUB_INSTALLATION_ID")
            .unwrap_or_else(|_| "0".to_string())
            .parse()
            .context("parsing GITHUB_INSTALLATION_ID")?;
        let github_private_key_pem = env::var("GITHUB_APP_PRIVATE_KEY")
            .or_else(|_| env::var("GITHUB_PRIVATE_KEY_PEM"))
            .unwrap_or_default();
        let github_owner = env::var("GITHUB_OWNER").unwrap_or_else(|_| "world".to_string());
        let github_repo = env::var("GITHUB_REPO").unwrap_or_else(|_| "mobile-bench-rs".to_string());
        let github_workflow_id =
            env::var("GITHUB_WORKFLOW_ID").unwrap_or_else(|_| "mobile-bench.yml".to_string());
        let github_compile_gate_workflow_id = env::var("GITHUB_COMPILE_GATE_WORKFLOW_ID")
            .unwrap_or_else(|_| "compile-gate.yml".to_string());

        Ok(Self {
            database_url,
            public_http_addr,
            private_http_addr,
            delivery_retry_limit,
            delivery_claim_timeout_secs,
            github_webhook_secret,
            github_api_base_url,
            github_app_id,
            github_installation_id,
            github_private_key_pem,
            github_owner,
            github_repo,
            github_workflow_id,
            github_compile_gate_workflow_id,
        })
    }

    pub fn for_test() -> Self {
        Self {
            database_url: "postgresql://postgres@127.0.0.1:55432/postgres".to_string(),
            public_http_addr: "127.0.0.1:0".parse().expect("valid test listen address"),
            private_http_addr: "127.0.0.1:0".parse().expect("valid test listen address"),
            delivery_retry_limit: 5,
            delivery_claim_timeout_secs: 300,
            github_webhook_secret: TEST_GITHUB_WEBHOOK_SECRET.to_string(),
            github_api_base_url: "http://127.0.0.1:1".to_string(),
            github_app_id: 1,
            github_installation_id: 2,
            github_private_key_pem: TEST_GITHUB_APP_PRIVATE_KEY_PEM.to_string(),
            github_owner: "world".to_string(),
            github_repo: "mobile-bench-rs".to_string(),
            github_workflow_id: "mobile-bench.yml".to_string(),
            github_compile_gate_workflow_id: "compile-gate.yml".to_string(),
        }
    }
}
