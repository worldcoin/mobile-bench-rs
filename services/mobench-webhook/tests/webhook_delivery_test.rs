use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

#[sqlx::test(migrations = "./migrations")]
async fn webhook_post_persists_delivery_and_returns_202(pool: sqlx::PgPool) {
    let app = mobench_webhook::app_for_test_with_pool(pool.clone());
    let payload = r#"{"action":"labeled"}"#;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook")
                .header("X-GitHub-Event", "pull_request")
                .header("X-GitHub-Delivery", "delivery-1")
                .header(
                    "X-Hub-Signature-256",
                    mobench_webhook::webhook::verify::sign_for_test(payload),
                )
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let row: (i64,) = sqlx::query_as(
        "select count(*) from github_webhook_deliveries where delivery_id = $1 and event = $2 and status = 'pending'",
    )
    .bind("delivery-1")
    .bind("pull_request")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn webhook_post_duplicate_delivery_returns_202_without_second_row(pool: sqlx::PgPool) {
    let app = mobench_webhook::app_for_test_with_pool(pool.clone());
    let payload = r#"{"action":"labeled"}"#;

    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook")
                    .header("X-GitHub-Event", "pull_request")
                    .header("X-GitHub-Delivery", "delivery-1")
                    .header(
                        "X-Hub-Signature-256",
                        mobench_webhook::webhook::verify::sign_for_test(payload),
                    )
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    let row: (i64,) =
        sqlx::query_as("select count(*) from github_webhook_deliveries where delivery_id = $1")
            .bind("delivery-1")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(row.0, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn webhook_post_rejects_invalid_signature(pool: sqlx::PgPool) {
    let app = mobench_webhook::app_for_test_with_pool(pool.clone());
    let payload = r#"{"action":"labeled"}"#;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook")
                .header("X-GitHub-Event", "pull_request")
                .header("X-GitHub-Delivery", "delivery-1")
                .header("X-Hub-Signature-256", "sha256=deadbeef")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let row: (i64,) =
        sqlx::query_as("select count(*) from github_webhook_deliveries where delivery_id = $1")
            .bind("delivery-1")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(row.0, 0);
}
