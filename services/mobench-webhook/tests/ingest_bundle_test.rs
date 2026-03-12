mod support;

use std::collections::BTreeMap;

#[sqlx::test(migrations = "./migrations")]
async fn workflow_run_completed_ingests_history_bundle(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();
    harness
        .stub_history_artifact("mobench-history-v1")
        .await
        .unwrap();
    harness
        .enqueue_fixture("workflow_run_completed.json")
        .await
        .unwrap();

    let worked = harness.run_one_delivery().await.unwrap();
    let deliveries = harness.list_deliveries().await.unwrap();
    let workflow_runs = harness.list_workflow_runs().await.unwrap();
    let platform_runs = harness.list_platform_runs().await.unwrap();
    let results = harness.list_results().await.unwrap();
    let check_runs = harness.recorded_check_runs().await;

    assert!(worked);
    assert_eq!(workflow_runs.len(), 1, "{deliveries:#?}");
    assert_eq!(workflow_runs[0].workflow_run_id, 424242);
    assert_eq!(workflow_runs[0].trigger_source, "pr_comment");
    assert_eq!(
        workflow_runs[0].request_command.as_deref(),
        Some("/mobench platform=both iterations=30 warmup=5 device_profile=low-spec")
    );

    assert_eq!(platform_runs.len(), 2);
    assert!(platform_runs.iter().any(|run| run.platform == "ios"));
    assert!(platform_runs.iter().any(|run| run.platform == "android"));
    assert!(platform_runs.iter().all(|run| run.check_run_id.is_some()));
    assert_eq!(results.len(), 2);
    assert_eq!(check_runs.len(), 2);
    assert!(
        check_runs
            .iter()
            .any(|request| request["name"] == "Mobench - ios")
    );
    assert!(
        check_runs
            .iter()
            .any(|request| request["name"] == "Mobench - android")
    );
    assert!(
        results
            .iter()
            .any(|result| result.function_name == "bench_nullifier_proving_only")
    );
    assert!(
        results
            .iter()
            .any(|result| result.function_name == "bench_query_proof_generation")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn foreign_repo_workflow_completion_is_ignored(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();
    harness
        .stub_history_artifact("mobench-history-v1")
        .await
        .unwrap();
    harness
        .enqueue_fixture("workflow_run_completed_other_repo.json")
        .await
        .unwrap();

    let worked = harness.run_one_delivery().await.unwrap();
    let workflow_runs = harness.list_workflow_runs().await.unwrap();
    let platform_runs = harness.list_platform_runs().await.unwrap();
    let results = harness.list_results().await.unwrap();
    let check_runs = harness.recorded_check_runs().await;
    let recorded_requests = harness.recorded_requests().await;

    assert!(worked);
    assert!(workflow_runs.is_empty());
    assert!(platform_runs.is_empty());
    assert!(results.is_empty());
    assert!(check_runs.is_empty());
    assert!(recorded_requests.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn workflow_run_completed_continues_when_one_platform_summary_is_invalid(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool.clone()).await.unwrap();
    harness
        .stub_history_artifact_with_overrides_for_run(
            424242,
            "mobench-history-v1",
            &[("android/summary.json", "{ not valid json")],
        )
        .await
        .unwrap();
    harness
        .enqueue_fixture("workflow_run_completed.json")
        .await
        .unwrap();

    let worked = harness.run_one_delivery().await.unwrap();
    let deliveries = harness.list_deliveries().await.unwrap();
    let platform_runs = harness.list_platform_runs().await.unwrap();
    let results = harness.list_results().await.unwrap();
    let check_runs = harness.recorded_check_runs().await;

    assert!(worked);
    assert_eq!(deliveries[0].status, "processed");
    assert_eq!(platform_runs.len(), 2, "{deliveries:#?}");
    assert!(
        platform_runs
            .iter()
            .any(|run| run.platform == "ios" && run.status == "completed")
    );
    assert!(
        platform_runs
            .iter()
            .any(|run| run.platform == "android" && run.status == "failed")
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].function_name, "bench_nullifier_proving_only");
    assert_eq!(check_runs.len(), 1);
    assert_eq!(check_runs[0]["name"], "Mobench - ios");
}

#[sqlx::test(migrations = "./migrations")]
async fn workflow_run_completed_redelivery_reuses_existing_check_runs(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();
    harness
        .stub_history_artifact("mobench-history-v1")
        .await
        .unwrap();
    harness
        .enqueue_fixture("workflow_run_completed.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    let first_platform_runs = harness.list_platform_runs().await.unwrap();
    let first_check_run_ids = platform_check_run_ids(&first_platform_runs);

    harness
        .enqueue_fixture_as(
            "workflow_run_completed.json",
            "workflow_run_completed-redelivery",
        )
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    let platform_runs = harness.list_platform_runs().await.unwrap();
    let check_runs = harness.recorded_check_runs().await;

    assert_eq!(platform_check_run_ids(&platform_runs), first_check_run_ids);
    assert_eq!(check_runs.len(), 4);
    assert_eq!(
        check_runs[2..]
            .iter()
            .map(|payload| payload["id"].as_i64().unwrap())
            .collect::<std::collections::BTreeSet<_>>(),
        first_check_run_ids.values().copied().collect()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn workflow_run_completed_uses_manifest_regression_threshold_for_server_checks(
    pool: sqlx::PgPool,
) {
    let harness = support::Harness::new(pool.clone()).await.unwrap();
    let repos = support::repos(pool);

    let baseline_workflow = repos
        .runs
        .upsert_workflow_run(mobench_webhook::db::models::UpsertWorkflowRun {
            workflow_run_id: 4001,
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
            check_run_id: Some(9000),
            check_run_name: "Mobench - ios",
            workflow_inputs: serde_json::json!({
                "platform": "ios",
                "device_profile": "low-spec",
                "iterations": "30",
                "warmup": "5",
                "base_ref": "main",
                "regression_threshold_pct": "15.0"
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
            avg_ms: 1100.0,
            median_ms: Some(1095.0),
            p95_ms: Some(1180.0),
            best_ms: 1080.0,
            worst_ms: 1195.0,
            std_dev_ms: Some(21.0),
            cpu_avg_percent: None,
            cpu_peak_percent: None,
            ram_avg_mb: None,
            ram_peak_mb: None,
        })
        .await
        .unwrap();

    let mut manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mobench-history-v1/manifest.json")).unwrap();
    for platform_run in manifest["platform_runs"].as_array_mut().unwrap() {
        platform_run["workflow_inputs"]["regression_threshold_pct"] = serde_json::json!("15.0");
    }
    let manifest_override = manifest.to_string();

    harness
        .stub_history_artifact_with_overrides_for_run(
            424242,
            "mobench-history-v1",
            &[("manifest.json", manifest_override.as_str())],
        )
        .await
        .unwrap();
    harness
        .enqueue_fixture("workflow_run_completed.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    let check_runs = harness.recorded_check_runs().await;
    let ios_check = check_runs
        .iter()
        .find(|payload| payload["name"] == "Mobench - ios")
        .unwrap();

    assert_eq!(ios_check["conclusion"], "success");
    assert_eq!(ios_check["output"]["title"], "1 benchmarks passed");
}

fn platform_check_run_ids(
    platform_runs: &[mobench_webhook::db::models::PlatformRunRecord],
) -> BTreeMap<String, i64> {
    platform_runs
        .iter()
        .map(|run| (run.platform.clone(), run.check_run_id.unwrap()))
        .collect()
}
