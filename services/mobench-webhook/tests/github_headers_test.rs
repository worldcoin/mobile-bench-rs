mod support;

#[sqlx::test(migrations = "./migrations")]
async fn webhook_github_requests_always_send_mobench_user_agent(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool).await.unwrap();
    harness
        .stub_history_artifact_for_run(424242, "mobench-history-v1")
        .await
        .unwrap();

    harness
        .enqueue_fixture("issue_comment_mobench_custom.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    harness
        .enqueue_fixture("workflow_run_completed.json")
        .await
        .unwrap();
    harness.run_one_delivery().await.unwrap();

    let recorded_requests = harness.recorded_requests().await;

    assert!(!recorded_requests.is_empty());
    for request in recorded_requests {
        assert_eq!(
            request.user_agent.as_deref(),
            Some(mobench_webhook::github::USER_AGENT),
            "missing mobench user agent for {} {}",
            request.method,
            request.path
        );
    }
}
