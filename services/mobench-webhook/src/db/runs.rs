use anyhow::Result;
use sqlx::{PgPool, query_as, types::Json};
use uuid::Uuid;

use crate::db::models::{
    PlatformRunRecord, PlatformRunWithWorkflowRecord, TrendPointRecord, UpsertPlatformRun,
    UpsertWorkflowRun, WorkflowRunRecord,
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

    pub async fn list_workflow_runs(&self) -> Result<Vec<WorkflowRunRecord>> {
        let records = query_as::<_, WorkflowRunRecord>(
            r#"
            SELECT id,
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
            FROM benchmark_workflow_runs
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(records)
    }

    pub async fn list_workflow_runs_filtered(
        &self,
        branch: Option<&str>,
        pr_number: Option<i32>,
        limit: i64,
    ) -> Result<Vec<WorkflowRunRecord>> {
        let records = query_as::<_, WorkflowRunRecord>(
            r#"
            SELECT id,
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
            FROM benchmark_workflow_runs
            WHERE ($1::text IS NULL OR head_ref = $1 OR base_ref = $1)
              AND ($2::integer IS NULL OR pr_number = $2)
            ORDER BY created_at DESC
            LIMIT $3
            "#,
        )
        .bind(branch)
        .bind(pr_number)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(records)
    }

    pub async fn get_workflow_run(&self, workflow_run_id: i64) -> Result<Option<WorkflowRunRecord>> {
        let record = query_as::<_, WorkflowRunRecord>(
            r#"
            SELECT id,
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
            FROM benchmark_workflow_runs
            WHERE workflow_run_id = $1
            "#,
        )
        .bind(workflow_run_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn list_platform_runs(&self) -> Result<Vec<PlatformRunRecord>> {
        let records = query_as::<_, PlatformRunRecord>(
            r#"
            SELECT id,
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
            FROM benchmark_platform_runs
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(records)
    }

    pub async fn list_platform_runs_for_workflow(
        &self,
        workflow_run_uuid: Uuid,
    ) -> Result<Vec<PlatformRunRecord>> {
        let records = query_as::<_, PlatformRunRecord>(
            r#"
            SELECT id,
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
            FROM benchmark_platform_runs
            WHERE workflow_run_uuid = $1
            ORDER BY platform ASC
            "#,
        )
        .bind(workflow_run_uuid)
        .fetch_all(&self.pool)
        .await?;

        Ok(records)
    }

    pub async fn get_platform_run_with_workflow(
        &self,
        platform_run_uuid: Uuid,
    ) -> Result<Option<PlatformRunWithWorkflowRecord>> {
        let record = query_as::<_, PlatformRunWithWorkflowRecord>(
            r#"
            SELECT platform_runs.id,
                   platform_runs.workflow_run_uuid,
                   platform_runs.platform,
                   platform_runs.check_run_id,
                   platform_runs.check_run_name,
                   platform_runs.workflow_inputs,
                   platform_runs.device_profile,
                   platform_runs.device_name,
                   platform_runs.os_version,
                   platform_runs.iterations,
                   platform_runs.warmup,
                   platform_runs.status,
                   workflow_runs.workflow_run_id,
                   workflow_runs.repo_owner,
                   workflow_runs.repo_name,
                   workflow_runs.head_sha,
                   workflow_runs.head_ref,
                   workflow_runs.base_ref,
                   workflow_runs.pr_number,
                   workflow_runs.trigger_source,
                   workflow_runs.requested_by,
                   workflow_runs.request_command
            FROM benchmark_platform_runs AS platform_runs
            JOIN benchmark_workflow_runs AS workflow_runs
              ON workflow_runs.id = platform_runs.workflow_run_uuid
            WHERE platform_runs.id = $1
            "#,
        )
        .bind(platform_run_uuid)
        .fetch_optional(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn get_platform_run_by_check_run_id(
        &self,
        check_run_id: i64,
    ) -> Result<Option<PlatformRunWithWorkflowRecord>> {
        let record = query_as::<_, PlatformRunWithWorkflowRecord>(
            r#"
            SELECT platform_runs.id,
                   platform_runs.workflow_run_uuid,
                   platform_runs.platform,
                   platform_runs.check_run_id,
                   platform_runs.check_run_name,
                   platform_runs.workflow_inputs,
                   platform_runs.device_profile,
                   platform_runs.device_name,
                   platform_runs.os_version,
                   platform_runs.iterations,
                   platform_runs.warmup,
                   platform_runs.status,
                   workflow_runs.workflow_run_id,
                   workflow_runs.repo_owner,
                   workflow_runs.repo_name,
                   workflow_runs.head_sha,
                   workflow_runs.head_ref,
                   workflow_runs.base_ref,
                   workflow_runs.pr_number,
                   workflow_runs.trigger_source,
                   workflow_runs.requested_by,
                   workflow_runs.request_command
            FROM benchmark_platform_runs AS platform_runs
            JOIN benchmark_workflow_runs AS workflow_runs
              ON workflow_runs.id = platform_runs.workflow_run_uuid
            WHERE platform_runs.check_run_id = $1
            "#,
        )
        .bind(check_run_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn find_latest_successful_baseline(
        &self,
        candidate_platform_run_uuid: Uuid,
        branch: &str,
    ) -> Result<Option<PlatformRunWithWorkflowRecord>> {
        let record = query_as::<_, PlatformRunWithWorkflowRecord>(
            r#"
            WITH candidate AS (
                SELECT platform, device_name, os_version
                FROM benchmark_platform_runs
                WHERE id = $1
            )
            SELECT platform_runs.id,
                   platform_runs.workflow_run_uuid,
                   platform_runs.platform,
                   platform_runs.check_run_id,
                   platform_runs.check_run_name,
                   platform_runs.workflow_inputs,
                   platform_runs.device_profile,
                   platform_runs.device_name,
                   platform_runs.os_version,
                   platform_runs.iterations,
                   platform_runs.warmup,
                   platform_runs.status,
                   workflow_runs.workflow_run_id,
                   workflow_runs.repo_owner,
                   workflow_runs.repo_name,
                   workflow_runs.head_sha,
                   workflow_runs.head_ref,
                   workflow_runs.base_ref,
                   workflow_runs.pr_number,
                   workflow_runs.trigger_source,
                   workflow_runs.requested_by,
                   workflow_runs.request_command
            FROM benchmark_platform_runs AS platform_runs
            JOIN benchmark_workflow_runs AS workflow_runs
              ON workflow_runs.id = platform_runs.workflow_run_uuid
            JOIN candidate
              ON candidate.platform = platform_runs.platform
             AND candidate.device_name = platform_runs.device_name
             AND candidate.os_version = platform_runs.os_version
            WHERE platform_runs.id <> $1
              AND platform_runs.status = 'completed'
              AND COALESCE(workflow_runs.conclusion, 'success') = 'success'
              AND workflow_runs.head_ref = $2
            ORDER BY workflow_runs.created_at DESC
            LIMIT 1
            "#,
        )
        .bind(candidate_platform_run_uuid)
        .bind(branch)
        .fetch_optional(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn list_trend_points(
        &self,
        function_name: &str,
        platform: &str,
        device_name: &str,
        branch: Option<&str>,
        limit: i64,
    ) -> Result<Vec<TrendPointRecord>> {
        let records = query_as::<_, TrendPointRecord>(
            r#"
            SELECT platform_runs.id AS platform_run_id,
                   workflow_runs.workflow_run_id,
                   workflow_runs.head_sha,
                   workflow_runs.head_ref,
                   results.function_name,
                   results.function_label,
                   results.avg_ms,
                   results.median_ms,
                   results.p95_ms,
                   platform_runs.device_name,
                   platform_runs.os_version
            FROM benchmark_results AS results
            JOIN benchmark_platform_runs AS platform_runs
              ON platform_runs.id = results.platform_run_uuid
            JOIN benchmark_workflow_runs AS workflow_runs
              ON workflow_runs.id = platform_runs.workflow_run_uuid
            WHERE results.function_name = $1
              AND platform_runs.platform = $2
              AND platform_runs.device_name = $3
              AND ($4::text IS NULL OR workflow_runs.head_ref = $4 OR workflow_runs.base_ref = $4)
            ORDER BY workflow_runs.created_at DESC
            LIMIT $5
            "#,
        )
        .bind(function_name)
        .bind(platform)
        .bind(device_name)
        .bind(branch)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(records)
    }

    pub async fn set_check_run_id(
        &self,
        platform_run_uuid: Uuid,
        check_run_id: i64,
    ) -> Result<Option<PlatformRunRecord>> {
        let record = query_as::<_, PlatformRunRecord>(
            r#"
            UPDATE benchmark_platform_runs
            SET check_run_id = $2
            WHERE id = $1
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
        .bind(platform_run_uuid)
        .bind(check_run_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(record)
    }
}
