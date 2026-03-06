use axum::{Json, Router, routing::post};
use serde_json::json;

#[tokio::test]
async fn installation_token_is_cached_until_refresh_window() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/app/installations/{installation_id}/access_tokens",
        post(|| async {
            Json(json!({
                "token": "installation-token",
                "expires_at": "2099-01-01T00:00:00Z"
            }))
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let auth = mobench_webhook::github::auth::GitHubAppAuth::for_test(format!("http://{addr}"));

    let first = auth.installation_token().await.unwrap();
    let second = auth.installation_token().await.unwrap();

    assert_eq!(first, second);
    assert_eq!(auth.test_installation_request_count().await, 1);
}
