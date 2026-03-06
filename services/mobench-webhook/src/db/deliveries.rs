use anyhow::Result;
use serde_json::Value;
use sqlx::{PgPool, query_as, types::Json};

use crate::db::models::DeliveryRecord;

#[derive(Clone)]
pub struct DeliveryRepository {
    pool: PgPool,
}

impl DeliveryRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert_fixture(
        &self,
        delivery_id: &str,
        event: &str,
        action: Option<&str>,
        payload: &str,
    ) -> Result<DeliveryRecord> {
        let payload: Value = serde_json::from_str(payload)?;
        self.insert_pending(delivery_id, event, action, payload)
            .await
    }

    pub async fn insert_pending(
        &self,
        delivery_id: &str,
        event: &str,
        action: Option<&str>,
        payload: Value,
    ) -> Result<DeliveryRecord> {
        let record = query_as::<_, DeliveryRecord>(
            r#"
            INSERT INTO github_webhook_deliveries (delivery_id, event, action, payload)
            VALUES ($1, $2, $3, $4)
            RETURNING id, delivery_id, event, action, payload, status, attempts, last_error
            "#,
        )
        .bind(delivery_id)
        .bind(event)
        .bind(action)
        .bind(Json(payload))
        .fetch_one(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn claim_next(&self) -> Result<Option<DeliveryRecord>> {
        let mut tx = self.pool.begin().await?;
        let record = query_as::<_, DeliveryRecord>(
            r#"
            WITH next_delivery AS (
                SELECT id
                FROM github_webhook_deliveries
                WHERE status = 'pending'
                  AND available_at <= now()
                ORDER BY received_at
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            )
            UPDATE github_webhook_deliveries AS deliveries
            SET attempts = deliveries.attempts + 1,
                claimed_at = now(),
                status = 'processing'
            FROM next_delivery
            WHERE deliveries.id = next_delivery.id
            RETURNING deliveries.id,
                      deliveries.delivery_id,
                      deliveries.event,
                      deliveries.action,
                      deliveries.payload,
                      deliveries.status,
                      deliveries.attempts,
                      deliveries.last_error
            "#,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(record)
    }
}
