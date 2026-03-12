mod support;

#[sqlx::test(migrations = "./migrations")]
async fn mobench_comment_with_custom_overrides_creates_dispatch(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();
    harness
        .enqueue_fixture("issue_comment_mobench_custom.json")
        .await
        .unwrap();

    let worked = harness.run_one_delivery().await.unwrap();
    let dispatches = harness.list_dispatches().await.unwrap();
    let workflow_requests = harness.dispatched_workflows().await;

    assert!(worked);
    assert_eq!(dispatches.len(), 1);

    let dispatch = &dispatches[0];
    assert_eq!(dispatch.trigger_source, "pr_comment");
    assert_eq!(dispatch.requested_by.as_deref(), Some("octocat"));
    assert_eq!(dispatch.workflow_inputs["platform"], "ios");
    assert_eq!(dispatch.workflow_inputs["device_profile"], "low-spec");
    assert_eq!(dispatch.workflow_inputs["iterations"], "50");
    assert_eq!(dispatch.workflow_inputs["warmup"], "5");
    assert_eq!(dispatch.workflow_inputs["ios_device"], "iPhone 15");
    assert_eq!(dispatch.workflow_inputs["ios_os_version"], "17");
    assert_eq!(dispatch.workflow_inputs["pr_number"], "123");
    assert_eq!(dispatch.workflow_inputs["requested_by"], "octocat");
    assert_eq!(dispatch.workflow_inputs["base_ref"], "release/1.2");

    assert_eq!(workflow_requests.len(), 1);
    assert_eq!(
        workflow_requests[0]["inputs"]["request_command"],
        "/mobench platform=ios iterations=50 ios_device=iPhone 15 ios_os_version=17"
    );
    assert_eq!(
        workflow_requests[0]["inputs"]["trigger_source"],
        "pr_comment"
    );
    assert_eq!(workflow_requests[0]["inputs"]["base_ref"], "release/1.2");
}

#[sqlx::test(migrations = "./migrations")]
async fn foreign_repo_comment_is_ignored(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();
    harness
        .enqueue_fixture("issue_comment_mobench_custom_other_repo.json")
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
