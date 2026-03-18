mod support;

#[sqlx::test(migrations = "./migrations")]
async fn check_run_rerequest_dispatches_stored_inputs_for_platform(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();
    harness
        .stub_history_artifact_for_run(424242, "mobench-history-v1")
        .await
        .unwrap();
    harness
        .enqueue_fixture("workflow_run_completed.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();
    harness.clear_dispatched_workflows().await;

    harness
        .enqueue_fixture("check_run_rerequested_ios.json")
        .await
        .unwrap();

    let worked = harness.run_one_delivery().await.unwrap();
    let dispatches = harness.list_dispatches().await.unwrap();
    let workflow_requests = harness.dispatched_workflows().await;

    assert!(worked);
    assert_eq!(dispatches.len(), 1);

    let dispatch = &dispatches[0];
    assert_eq!(dispatch.trigger_source, "check_rerequest");
    assert_eq!(dispatch.requested_by.as_deref(), Some("rerun-bot"));
    assert_eq!(dispatch.workflow_inputs["platform"], "ios");
    assert_eq!(dispatch.workflow_inputs["requested_by"], "rerun-bot");

    assert_eq!(workflow_requests.len(), 1);
    assert_eq!(workflow_requests[0]["ref"], "feature/bench-pr");
    assert_eq!(workflow_requests[0]["inputs"]["trigger_source"], "check_rerequest");
    assert_eq!(workflow_requests[0]["inputs"]["platform"], "ios");
    assert_eq!(workflow_requests[0]["inputs"]["requested_by"], "rerun-bot");
    assert!(workflow_requests[0]["inputs"]["dispatch_id"].as_str().is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn check_run_rerequest_dedupes_active_duplicate_inputs(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();
    harness
        .stub_history_artifact_for_run(424242, "mobench-history-v1")
        .await
        .unwrap();
    harness
        .enqueue_fixture("workflow_run_completed.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();
    harness.clear_dispatched_workflows().await;

    harness
        .enqueue_fixture_as("check_run_rerequested_ios.json", "check_run_rerequested_ios-1")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    harness
        .enqueue_fixture_as("check_run_rerequested_ios.json", "check_run_rerequested_ios-2")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    let dispatches = harness.list_dispatches().await.unwrap();
    let workflow_requests = harness.dispatched_workflows().await;

    assert_eq!(dispatches.len(), 1);
    assert_eq!(workflow_requests.len(), 1);
}
