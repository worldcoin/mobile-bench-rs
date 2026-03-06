use anyhow::Result;
use sqlx::PgPool;

pub fn app_state(pool: PgPool) -> mobench_webhook::AppState {
    mobench_webhook::AppState::for_test(pool)
}

pub fn repos(pool: PgPool) -> mobench_webhook::db::Repositories {
    mobench_webhook::db::Repositories::new(pool)
}

pub async fn seed_labeled_pull_request_delivery(
    pool: PgPool,
    delivery_id: &str,
) -> Result<mobench_webhook::db::models::DeliveryRecord> {
    repos(pool)
        .deliveries
        .insert_fixture(
            delivery_id,
            "pull_request",
            Some("labeled"),
            r#"{"action":"labeled"}"#,
        )
        .await
}
