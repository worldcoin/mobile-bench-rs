use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn migrations_create_dispatch_and_result_tables(pool: PgPool) {
    let row: (i64,) = sqlx::query_as(
        "select count(*) from information_schema.tables where table_name in ('github_webhook_deliveries', 'benchmark_dispatches', 'benchmark_workflow_runs', 'benchmark_platform_runs', 'benchmark_results')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, 5);
}
