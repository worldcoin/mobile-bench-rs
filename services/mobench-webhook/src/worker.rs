use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};

use anyhow::Result;
use tracing::{info, warn};

use crate::{AppState, github, webhook::handlers::{DeliveryOutcome, handle_delivery}};

pub const DELIVERY_POLL_INTERVAL: Duration = Duration::from_secs(1);
pub async fn worker_loop(state: AppState) -> Result<()> {
    loop {
        if !run_once(&state).await? {
            tokio::time::sleep(DELIVERY_POLL_INTERVAL).await;
        }
    }
}

pub async fn run_once(state: &AppState) -> Result<bool> {
    let Some(delivery) = state
        .repos
        .deliveries
        .claim_next_with_timeout(state.config.delivery_claim_timeout_secs)
        .await?
    else {
        return Ok(false);
    };
    info!(delivery_id = delivery.delivery_id, attempts = delivery.attempts, "delivery.claimed");

    match handle_delivery(state, &delivery).await {
        Ok(DeliveryOutcome::Processed) => {
            state.repos.deliveries.mark_processed(delivery.id).await?;
            info!(delivery_id = delivery.delivery_id, "delivery.processed");
        }
        Ok(DeliveryOutcome::Ignored) => {
            state.repos.deliveries.mark_ignored(delivery.id).await?;
            info!(delivery_id = delivery.delivery_id, "delivery.ignored");
        }
        Err(err) => {
            let rate_limit_retry_after = github::find_rate_limit_retry_after(&err);
            let retry_after_secs =
                rate_limit_retry_after.unwrap_or_else(|| retry_delay_seconds(&delivery));
            let message = if rate_limit_retry_after.is_some() {
                format!("GitHub rate limit exceeded; retrying in {retry_after_secs}s")
            } else {
                err.to_string()
            };
            warn!(delivery_id = delivery.delivery_id, error = message, "delivery.failed");

            if delivery.attempts >= state.config.delivery_retry_limit {
                state
                    .repos
                    .deliveries
                    .mark_failed(delivery.id, &message)
                    .await?;
            } else {
                state
                    .repos
                    .deliveries
                    .requeue_with_backoff(delivery.id, &message, retry_after_secs)
                    .await?;
            }
        }
    }

    Ok(true)
}

fn retry_delay_seconds(delivery: &crate::db::models::DeliveryRecord) -> i32 {
    let attempt = delivery.attempts.max(1) as u64;
    let base = attempt.saturating_mul(attempt);
    let jitter = stable_jitter_seconds(&delivery.delivery_id);

    (base + jitter).min(60) as i32
}

fn stable_jitter_seconds(delivery_id: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    delivery_id.hash(&mut hasher);
    hasher.finish() % 3
}
