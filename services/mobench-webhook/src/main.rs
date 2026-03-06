#[tokio::main]
async fn main() {
    if let Err(err) = mobench_webhook::serve().await {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}
