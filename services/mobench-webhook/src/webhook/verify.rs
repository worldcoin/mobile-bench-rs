use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::config::TEST_GITHUB_WEBHOOK_SECRET;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug)]
pub enum VerifyError {
    MissingSignature,
    InvalidSignatureFormat,
    InvalidSignatureEncoding,
    SignatureMismatch,
}

pub fn verify_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> Result<(), VerifyError> {
    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|value| value.to_str().ok())
        .ok_or(VerifyError::MissingSignature)?;
    let encoded = signature
        .strip_prefix("sha256=")
        .ok_or(VerifyError::InvalidSignatureFormat)?;
    let provided = hex::decode(encoded).map_err(|_| VerifyError::InvalidSignatureEncoding)?;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts arbitrary key sizes");
    mac.update(body);
    mac.verify_slice(&provided)
        .map_err(|_| VerifyError::SignatureMismatch)?;

    Ok(())
}

pub fn sign_for_test(body: &str) -> String {
    sign(TEST_GITHUB_WEBHOOK_SECRET, body)
}

fn sign(secret: &str, body: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts arbitrary key sizes");
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}
