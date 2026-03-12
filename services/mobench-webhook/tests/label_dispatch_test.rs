mod support;

#[sqlx::test(migrations = "./migrations")]
async fn bench_label_dispatches_default_inputs_once(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();
    harness
        .enqueue_fixture("pull_request_labeled_bench.json")
        .await
        .unwrap();

    let worked = harness.run_one_delivery().await.unwrap();
    let dispatches = harness.list_dispatches().await.unwrap();
    let workflow_requests = harness.dispatched_workflows().await;

    assert!(worked);
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].trigger_source, "label");
    assert_eq!(dispatches[0].workflow_inputs["platform"], "both");
    assert_eq!(dispatches[0].workflow_inputs["device_profile"], "low-spec");
    assert_eq!(dispatches[0].workflow_inputs["iterations"], "30");
    assert_eq!(dispatches[0].workflow_inputs["warmup"], "5");
    assert_eq!(dispatches[0].workflow_inputs["base_ref"], "release/1.2");
    assert_eq!(workflow_requests.len(), 1);
    assert_eq!(workflow_requests[0]["inputs"]["base_ref"], "release/1.2");
}

#[sqlx::test(migrations = "./migrations")]
async fn foreign_repo_label_is_ignored(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();
    harness
        .enqueue_fixture("pull_request_labeled_bench_other_repo.json")
        .await
        .unwrap();

    let worked = harness.run_one_delivery().await.unwrap();
    let dispatches = harness.list_dispatches().await.unwrap();
    let workflow_requests = harness.dispatched_workflows().await;
    let recorded_requests = harness.recorded_requests().await;

    assert!(worked);
    assert!(dispatches.is_empty());
    assert!(workflow_requests.is_empty());
    assert!(recorded_requests.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn bench_label_duplicate_delivery_inputs_are_deduped(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();

    harness
        .enqueue_fixture("pull_request_labeled_bench.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    harness
        .enqueue_fixture("pull_request_labeled_bench.json")
        .await
        .unwrap_err();

    harness
        .enqueue_fixture("pull_request_labeled_bench-duplicate.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    let dispatches = harness.list_dispatches().await.unwrap();
    let workflow_requests = harness.dispatched_workflows().await;

    assert_eq!(dispatches.len(), 1);
    assert_eq!(workflow_requests.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn failed_workflow_ingest_unblocks_same_inputs_for_future_label_dispatch(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();

    harness
        .enqueue_fixture("pull_request_labeled_bench.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    harness
        .enqueue_fixture("workflow_run_completed.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    let dispatches = harness.list_dispatches().await.unwrap();
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].status, "failed");
    assert_eq!(dispatches[0].workflow_run_id, Some(424242));

    harness
        .enqueue_fixture("pull_request_labeled_bench-duplicate.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    let dispatches = harness.list_dispatches().await.unwrap();
    let workflow_requests = harness.dispatched_workflows().await;

    assert_eq!(dispatches.len(), 2);
    assert_eq!(workflow_requests.len(), 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn bench_label_rate_limit_requeues_delivery_using_retry_after(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool.clone()).await.unwrap();
    harness.stub_workflow_dispatch_rate_limited(17).await;
    harness
        .enqueue_fixture("pull_request_labeled_bench.json")
        .await
        .unwrap();

    let worked = harness.run_one_delivery().await.unwrap();

    assert!(worked);

    let row: (String, i32, Option<String>, i64) = sqlx::query_as(
        r#"
        select status,
               attempts,
               last_error,
               greatest(0, extract(epoch from available_at - now())::bigint) as retry_after_secs
        from github_webhook_deliveries
        where delivery_id = $1
        "#,
    )
    .bind("pull_request_labeled_bench.json")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, "pending");
    assert_eq!(row.1, 1);
    assert!(row.2.unwrap_or_default().contains("rate limit"));
    assert!(
        row.3 >= 15,
        "expected retry delay to honor Retry-After, got {}",
        row.3
    );
}
