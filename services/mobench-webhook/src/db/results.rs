use anyhow::Result;
use sqlx::{PgPool, query_as};

use crate::db::models::{BenchmarkResultRecord, UpsertBenchmarkResult};

#[derive(Clone)]
pub struct ResultRepository {
    pool: PgPool,
}

impl ResultRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn upsert_result(
        &self,
        input: UpsertBenchmarkResult<'_>,
    ) -> Result<BenchmarkResultRecord> {
        let record = query_as::<_, BenchmarkResultRecord>(
            r#"
            INSERT INTO benchmark_results (
                platform_run_uuid,
                function_name,
                function_label,
                avg_ms,
                median_ms,
                p95_ms,
                best_ms,
                worst_ms,
                std_dev_ms,
                cpu_avg_percent,
                cpu_peak_percent,
                ram_avg_mb,
                ram_peak_mb
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (platform_run_uuid, function_name) DO UPDATE
            SET function_label = EXCLUDED.function_label,
                avg_ms = EXCLUDED.avg_ms,
                median_ms = EXCLUDED.median_ms,
                p95_ms = EXCLUDED.p95_ms,
                best_ms = EXCLUDED.best_ms,
                worst_ms = EXCLUDED.worst_ms,
                std_dev_ms = EXCLUDED.std_dev_ms,
                cpu_avg_percent = EXCLUDED.cpu_avg_percent,
                cpu_peak_percent = EXCLUDED.cpu_peak_percent,
                ram_avg_mb = EXCLUDED.ram_avg_mb,
                ram_peak_mb = EXCLUDED.ram_peak_mb
            RETURNING id,
                      platform_run_uuid,
                      function_name,
                      function_label,
                      avg_ms,
                      median_ms,
                      p95_ms,
                      best_ms,
                      worst_ms,
                      std_dev_ms,
                      cpu_avg_percent,
                      cpu_peak_percent,
                      ram_avg_mb,
                      ram_peak_mb
            "#,
        )
        .bind(input.platform_run_uuid)
        .bind(input.function_name)
        .bind(input.function_label)
        .bind(input.avg_ms)
        .bind(input.median_ms)
        .bind(input.p95_ms)
        .bind(input.best_ms)
        .bind(input.worst_ms)
        .bind(input.std_dev_ms)
        .bind(input.cpu_avg_percent)
        .bind(input.cpu_peak_percent)
        .bind(input.ram_avg_mb)
        .bind(input.ram_peak_mb)
        .fetch_one(&self.pool)
        .await?;

        Ok(record)
    }
}
