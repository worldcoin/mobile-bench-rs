use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};

use anyhow::Result;

use crate::{AppState, webhook::handlers::handle_delivery};

pub const DELIVERY_POLL_INTERVAL: Duration = Duration::from_secs(1);
pub const DELIVERY_RETRY_LIMIT: i32 = 5;

pub async fn worker_loop(state: AppState) -> Result<()> {
    loop {
        if !run_once(&state).await? {
            tokio::time::sleep(DELIVERY_POLL_INTERVAL).await;
        }
    }
}

pub async fn run_once(state: &AppState) -> Result<bool> {
    let Some(delivery) = state.repos.deliveries.claim_next().await? else {
        return Ok(false);
    };

    match handle_delivery(state, &delivery).await {
        Ok(()) => state.repos.deliveries.mark_processed(delivery.id).await?,
        Err(err) => {
            let message = err.to_string();

            if delivery.attempts >= DELIVERY_RETRY_LIMIT {
                state
                    .repos
                    .deliveries
                    .mark_failed(delivery.id, &message)
                    .await?;
            } else {
                state
                    .repos
                    .deliveries
                    .requeue_with_backoff(delivery.id, &message, retry_delay_seconds(&delivery))
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
