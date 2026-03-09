mod support;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

#[sqlx::test(migrations = "./migrations")]
async fn private_api_lists_ingested_history(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool.clone()).await.unwrap();
    harness
        .stub_history_artifact_for_run(424242, "mobench-history-v1")
        .await
        .unwrap();
    harness
        .enqueue_fixture("workflow_run_completed.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    let workflow_runs = harness.list_workflow_runs().await.unwrap();
    let platform_runs = harness.list_platform_runs().await.unwrap();
    let ios_run = platform_runs
        .iter()
        .find(|run| run.platform == "ios")
        .unwrap();

    let app = mobench_webhook::private_app_for_test_with_pool(pool);

    let healthz = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(healthz.status(), StatusCode::OK);

    let workflow_list = json_body(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/workflow-runs?branch=main&limit=10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(workflow_list["workflow_runs"].as_array().unwrap().len(), 1);

    let workflow_detail = json_body(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/workflow-runs/{}", workflow_runs[0].workflow_run_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(workflow_detail["workflow_run"]["workflow_run_id"], 424242);
    assert_eq!(
        workflow_detail["workflow_run"]["platform_runs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let platform_detail = json_body(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/platform-runs/{}", ios_run.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(platform_detail["platform_run"]["platform"], "ios");
    assert_eq!(
        platform_detail["platform_run"]["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let trends = json_body(
        app.oneshot(
            Request::builder()
                .uri("/api/trends?function=bench_query_proof_generation&platform=android&device_name=Pixel%208&branch=main&limit=50")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(trends["points"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn private_api_compare_uses_latest_successful_baseline(pool: sqlx::PgPool) {
    let repos = support::repos(pool.clone());

    let baseline_workflow = repos
        .runs
        .upsert_workflow_run(mobench_webhook::db::models::UpsertWorkflowRun {
            workflow_run_id: 1001,
            workflow_run_attempt: 1,
            repo_owner: "world",
            repo_name: "mobile-bench-rs",
            workflow_name: "Mobile Benchmarks",
            head_sha: "main-sha",
            head_ref: "main",
            base_ref: Some("main"),
            pr_number: None,
            trigger_source: "workflow_dispatch",
            requested_by: Some("octocat"),
            request_command: None,
            mobench_version: Some("0.1.15"),
            mobench_ref: Some("refs/heads/main"),
            conclusion: Some("success"),
        })
        .await
        .unwrap();
    let baseline_platform = repos
        .runs
        .upsert_platform_run(mobench_webhook::db::models::UpsertPlatformRun {
            workflow_run_uuid: baseline_workflow.id,
            platform: "ios",
            check_run_id: Some(9001),
            check_run_name: "Mobench - ios",
            workflow_inputs: serde_json::json!({
                "platform": "ios",
                "device_profile": "low-spec",
                "iterations": "30",
                "warmup": "5"
            }),
            device_profile: Some("low-spec"),
            device_name: "iPhone 14",
            os_version: "16.0",
            iterations: 30,
            warmup: 5,
            status: "completed",
        })
        .await
        .unwrap();
    repos
        .results
        .upsert_result(mobench_webhook::db::models::UpsertBenchmarkResult {
            platform_run_uuid: baseline_platform.id,
            function_name: "bench_nullifier_proving_only",
            function_label: "bench_nullifier_proving_only",
            avg_ms: 100.0,
            median_ms: Some(98.0),
            p95_ms: Some(110.0),
            best_ms: 95.0,
            worst_ms: 120.0,
            std_dev_ms: Some(4.0),
            cpu_avg_percent: None,
            cpu_peak_percent: None,
            ram_avg_mb: None,
            ram_peak_mb: None,
        })
        .await
        .unwrap();

    let candidate_workflow = repos
        .runs
        .upsert_workflow_run(mobench_webhook::db::models::UpsertWorkflowRun {
            workflow_run_id: 1002,
            workflow_run_attempt: 1,
            repo_owner: "world",
            repo_name: "mobile-bench-rs",
            workflow_name: "Mobile Benchmarks",
            head_sha: "feature-sha",
            head_ref: "feature/bench-pr",
            base_ref: Some("main"),
            pr_number: Some(123),
            trigger_source: "pr_comment",
            requested_by: Some("octocat"),
            request_command: Some("/mobench platform=ios"),
            mobench_version: Some("0.1.15"),
            mobench_ref: Some("refs/heads/feature/bench-pr"),
            conclusion: Some("success"),
        })
        .await
        .unwrap();
    let candidate_platform = repos
        .runs
        .upsert_platform_run(mobench_webhook::db::models::UpsertPlatformRun {
            workflow_run_uuid: candidate_workflow.id,
            platform: "ios",
            check_run_id: Some(9002),
            check_run_name: "Mobench - ios",
            workflow_inputs: serde_json::json!({
                "platform": "ios",
                "device_profile": "low-spec",
                "iterations": "30",
                "warmup": "5"
            }),
            device_profile: Some("low-spec"),
            device_name: "iPhone 14",
            os_version: "16.0",
            iterations: 30,
            warmup: 5,
            status: "completed",
        })
        .await
        .unwrap();
    repos
        .results
        .upsert_result(mobench_webhook::db::models::UpsertBenchmarkResult {
            platform_run_uuid: candidate_platform.id,
            function_name: "bench_nullifier_proving_only",
            function_label: "bench_nullifier_proving_only",
            avg_ms: 125.0,
            median_ms: Some(122.0),
            p95_ms: Some(138.0),
            best_ms: 118.0,
            worst_ms: 146.0,
            std_dev_ms: Some(5.0),
            cpu_avg_percent: None,
            cpu_peak_percent: None,
            ram_avg_mb: None,
            ram_peak_mb: None,
        })
        .await
        .unwrap();

    let app = mobench_webhook::private_app_for_test_with_pool(pool);
    let compare = json_body(
        app.oneshot(
            Request::builder()
                .uri(format!(
                    "/api/compare?platform_run_id={}&baseline_branch=main&threshold_pct=5.0",
                    candidate_platform.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;

    assert_eq!(
        compare["baseline_platform_run_id"],
        serde_json::Value::String(baseline_platform.id.to_string())
    );
    assert_eq!(compare["rows"].as_array().unwrap().len(), 1);
    assert_eq!(compare["rows"][0]["function_name"], "bench_nullifier_proving_only");
    assert_eq!(compare["rows"][0]["label"], "regressed");
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
