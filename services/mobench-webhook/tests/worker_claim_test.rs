mod support;

#[sqlx::test(migrations = "./migrations")]
async fn worker_claims_pending_delivery_once(pool: sqlx::PgPool) {
    let repos = support::repos(pool.clone());
    repos
        .deliveries
        .insert_fixture(
            "delivery-1",
            "pull_request",
            Some("labeled"),
            r#"{"action":"labeled"}"#,
        )
        .await
        .unwrap();

    let claimed = repos
        .deliveries
        .claim_next()
        .await
        .unwrap()
        .expect("delivery");
    let second = repos.deliveries.claim_next().await.unwrap();

    assert_eq!(claimed.delivery_id, "delivery-1");
    assert_eq!(claimed.status, "processing");
    assert_eq!(claimed.attempts, 1);
    assert!(second.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn worker_run_once_marks_delivery_processed(pool: sqlx::PgPool) {
    support::seed_labeled_pull_request_delivery(pool.clone(), "delivery-1")
        .await
        .unwrap();
    let state = support::app_state(pool.clone());

    let worked = mobench_webhook::worker::run_once(&state).await.unwrap();

    assert!(worked);

    let row: (String, i32) = sqlx::query_as(
        "select status, attempts from github_webhook_deliveries where delivery_id = $1",
    )
    .bind("delivery-1")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, "processed");
    assert_eq!(row.1, 1);
}
