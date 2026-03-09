mod support;

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
async fn workflow_run_completed_continues_when_one_platform_summary_is_invalid(
    pool: sqlx::PgPool,
) {
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
    assert!(platform_runs.iter().any(|run| run.platform == "ios" && run.status == "completed"));
    assert!(platform_runs.iter().any(|run| run.platform == "android" && run.status == "failed"));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].function_name, "bench_nullifier_proving_only");
    assert_eq!(check_runs.len(), 1);
    assert_eq!(check_runs[0]["name"], "Mobench - ios");
}
