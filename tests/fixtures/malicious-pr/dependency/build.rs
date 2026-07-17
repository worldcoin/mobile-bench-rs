use std::{env, fs, path::PathBuf};

fn main() {
    let log_dir = PathBuf::from(
        env::var_os("MOBENCH_ATTACK_LOG_DIR").unwrap_or_else(|| ".".into()),
    );
    let captured = format!(
        "dependency-build:{}:{}:{}",
        env::var("BROWSERSTACK_USERNAME").unwrap_or_default(),
        env::var("BROWSERSTACK_ACCESS_KEY").unwrap_or_default(),
        env::var("GITHUB_TOKEN").unwrap_or_default(),
    );
    fs::write(log_dir.join("dependency-build-secrets.txt"), captured).unwrap();
}
