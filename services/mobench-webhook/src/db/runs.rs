use anyhow::Result;
use sqlx::{PgPool, query_as, types::Json};

use crate::db::models::{
    PlatformRunRecord, UpsertPlatformRun, UpsertWorkflowRun, WorkflowRunRecord,
};

#[derive(Clone)]
pub struct RunRepository {
    pool: PgPool,
}

impl RunRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn upsert_workflow_run(
        &self,
        input: UpsertWorkflowRun<'_>,
    ) -> Result<WorkflowRunRecord> {
        let record = query_as::<_, WorkflowRunRecord>(
            r#"
            INSERT INTO benchmark_workflow_runs (
                workflow_run_id,
                workflow_run_attempt,
                repo_owner,
                repo_name,
                workflow_name,
                head_sha,
                head_ref,
                base_ref,
                pr_number,
                trigger_source,
                requested_by,
                request_command,
                mobench_version,
                mobench_ref,
                conclusion
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ON CONFLICT (workflow_run_id) DO UPDATE
            SET workflow_run_attempt = EXCLUDED.workflow_run_attempt,
                repo_owner = EXCLUDED.repo_owner,
                repo_name = EXCLUDED.repo_name,
                workflow_name = EXCLUDED.workflow_name,
                head_sha = EXCLUDED.head_sha,
                head_ref = EXCLUDED.head_ref,
                base_ref = EXCLUDED.base_ref,
                pr_number = EXCLUDED.pr_number,
                trigger_source = EXCLUDED.trigger_source,
                requested_by = EXCLUDED.requested_by,
                request_command = EXCLUDED.request_command,
                mobench_version = EXCLUDED.mobench_version,
                mobench_ref = EXCLUDED.mobench_ref,
                conclusion = EXCLUDED.conclusion
            RETURNING id,
                      workflow_run_id,
                      workflow_run_attempt,
                      repo_owner,
                      repo_name,
                      workflow_name,
                      head_sha,
                      head_ref,
                      base_ref,
                      pr_number,
                      trigger_source,
                      requested_by,
                      request_command,
                      mobench_version,
                      mobench_ref,
                      conclusion
            "#,
        )
        .bind(input.workflow_run_id)
        .bind(input.workflow_run_attempt)
        .bind(input.repo_owner)
        .bind(input.repo_name)
        .bind(input.workflow_name)
        .bind(input.head_sha)
        .bind(input.head_ref)
        .bind(input.base_ref)
        .bind(input.pr_number)
        .bind(input.trigger_source)
        .bind(input.requested_by)
        .bind(input.request_command)
        .bind(input.mobench_version)
        .bind(input.mobench_ref)
        .bind(input.conclusion)
        .fetch_one(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn upsert_platform_run(
        &self,
        input: UpsertPlatformRun<'_>,
    ) -> Result<PlatformRunRecord> {
        let record = query_as::<_, PlatformRunRecord>(
            r#"
            INSERT INTO benchmark_platform_runs (
                workflow_run_uuid,
                platform,
                check_run_id,
                check_run_name,
                workflow_inputs,
                device_profile,
                device_name,
                os_version,
                iterations,
                warmup,
                status
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (workflow_run_uuid, platform) DO UPDATE
            SET check_run_id = EXCLUDED.check_run_id,
                check_run_name = EXCLUDED.check_run_name,
                workflow_inputs = EXCLUDED.workflow_inputs,
                device_profile = EXCLUDED.device_profile,
                device_name = EXCLUDED.device_name,
                os_version = EXCLUDED.os_version,
                iterations = EXCLUDED.iterations,
                warmup = EXCLUDED.warmup,
                status = EXCLUDED.status
            RETURNING id,
                      workflow_run_uuid,
                      platform,
                      check_run_id,
                      check_run_name,
                      workflow_inputs,
                      device_profile,
                      device_name,
                      os_version,
                      iterations,
                      warmup,
                      status
            "#,
        )
        .bind(input.workflow_run_uuid)
        .bind(input.platform)
        .bind(input.check_run_id)
        .bind(input.check_run_name)
        .bind(Json(input.workflow_inputs))
        .bind(input.device_profile)
        .bind(input.device_name)
        .bind(input.os_version)
        .bind(input.iterations)
        .bind(input.warmup)
        .bind(input.status)
        .fetch_one(&self.pool)
        .await?;

        Ok(record)
    }
}
