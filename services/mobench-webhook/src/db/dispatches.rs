use anyhow::Result;
use serde_json::Value;
use sqlx::{PgPool, query_as, types::Json};
use uuid::Uuid;

use crate::db::models::BenchmarkDispatch;

#[derive(Clone)]
pub struct DispatchRepository {
    pool: PgPool,
}

impl DispatchRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_queued(
        &self,
        dispatch_id: Uuid,
        repo_owner: &str,
        repo_name: &str,
        head_sha: &str,
        head_ref: &str,
        pr_number: Option<i32>,
        trigger_source: &str,
        requested_by: Option<&str>,
        workflow_inputs: Value,
    ) -> Result<BenchmarkDispatch> {
        let record = query_as::<_, BenchmarkDispatch>(
            r#"
            INSERT INTO benchmark_dispatches (
                dispatch_id,
                repo_owner,
                repo_name,
                head_sha,
                head_ref,
                pr_number,
                trigger_source,
                requested_by,
                workflow_inputs
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id,
                      dispatch_id,
                      repo_owner,
                      repo_name,
                      head_sha,
                      head_ref,
                      pr_number,
                      trigger_source,
                      requested_by,
                      workflow_inputs,
                      status,
                      workflow_run_id
            "#,
        )
        .bind(dispatch_id)
        .bind(repo_owner)
        .bind(repo_name)
        .bind(head_sha)
        .bind(head_ref)
        .bind(pr_number)
        .bind(trigger_source)
        .bind(requested_by)
        .bind(Json(workflow_inputs))
        .fetch_one(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn find_active_duplicate(
        &self,
        repo_owner: &str,
        repo_name: &str,
        head_sha: &str,
        workflow_inputs: &Value,
    ) -> Result<Option<BenchmarkDispatch>> {
        let record = query_as::<_, BenchmarkDispatch>(
            r#"
            SELECT id,
                   dispatch_id,
                   repo_owner,
                   repo_name,
                   head_sha,
                   head_ref,
                   pr_number,
                   trigger_source,
                   requested_by,
                   workflow_inputs,
                   status,
                   workflow_run_id
            FROM benchmark_dispatches
            WHERE repo_owner = $1
              AND repo_name = $2
              AND head_sha = $3
              AND workflow_inputs = $4
              AND status IN ('queued', 'running')
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(repo_owner)
        .bind(repo_name)
        .bind(head_sha)
        .bind(Json(workflow_inputs.clone()))
        .fetch_optional(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn list_all(&self) -> Result<Vec<BenchmarkDispatch>> {
        let records = query_as::<_, BenchmarkDispatch>(
            r#"
            SELECT id,
                   dispatch_id,
                   repo_owner,
                   repo_name,
                   head_sha,
                   head_ref,
                   pr_number,
                   trigger_source,
                   requested_by,
                   workflow_inputs,
                   status,
                   workflow_run_id
            FROM benchmark_dispatches
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(records)
    }
}
