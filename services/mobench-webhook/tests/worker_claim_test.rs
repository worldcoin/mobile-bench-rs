mod support;

use serde_json::json;

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
    let harness = support::Harness::new(pool.clone()).await.unwrap();
    harness
        .enqueue_fixture("pull_request_labeled_bench.json")
        .await
        .unwrap();

    let worked = harness.run_one_delivery().await.unwrap();

    assert!(worked);

    let row: (String, i32) = sqlx::query_as(
        "select status, attempts from github_webhook_deliveries where delivery_id = $1",
    )
    .bind("pull_request_labeled_bench.json")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, "processed");
    assert_eq!(row.1, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn worker_reclaims_timed_out_processing_delivery(pool: sqlx::PgPool) {
    let harness = support::Harness::new(pool.clone()).await.unwrap();
    harness
        .enqueue_fixture("pull_request_labeled_bench.json")
        .await
        .unwrap();
    sqlx::query(
        r#"
        UPDATE github_webhook_deliveries
        SET status = 'processing',
            claimed_at = now() - interval '10 minutes'
        WHERE delivery_id = 'pull_request_labeled_bench.json'
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let worked = harness.run_one_delivery().await.unwrap();

    assert!(worked);

    let row: (String, i32) = sqlx::query_as(
        "select status, attempts from github_webhook_deliveries where delivery_id = $1",
    )
    .bind("pull_request_labeled_bench.json")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, "processed");
    assert_eq!(row.1, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn worker_marks_unknown_events_ignored(pool: sqlx::PgPool) {
    support::repos(pool.clone())
        .deliveries
        .insert_pending("delivery-ignored", "ping", None, json!({}))
        .await
        .unwrap();
    let state = support::app_state(pool.clone());

    let worked = mobench_webhook::worker::run_once(&state).await.unwrap();

    assert!(worked);

    let row: (String, i32) = sqlx::query_as(
        "select status, attempts from github_webhook_deliveries where delivery_id = $1",
    )
    .bind("delivery-ignored")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, "ignored");
    assert_eq!(row.1, 1);
}
