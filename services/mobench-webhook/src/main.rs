#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mobench_webhook=info".into()),
        )
        .with_target(false)
        .init();

    if let Err(err) = mobench_webhook::serve().await {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}
