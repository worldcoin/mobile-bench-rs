use std::process::{Command, Output};

fn run(binary: &str, args: &[&str]) -> Output {
    Command::new(binary)
        .args(args)
        .output()
        .expect("run mobench CLI binary")
}

fn assert_version_output(output: Output) {
    assert!(
        output.status.success(),
        "version command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("mobench ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn direct_mobench_invocation_keeps_its_arguments() {
    assert_version_output(run(env!("CARGO_BIN_EXE_mobench"), &["--version"]));
}

#[test]
fn cargo_subcommand_invocation_strips_the_injected_name() {
    assert_version_output(run(
        env!("CARGO_BIN_EXE_cargo-mobench"),
        &["mobench", "--version"],
    ));
}

#[test]
fn cargo_mobench_invocation_works_through_cargo() {
    let bin_dir = tempfile::tempdir().expect("temporary PATH directory");
    let wrapper_name = format!("cargo-mobench{}", std::env::consts::EXE_SUFFIX);
    std::fs::copy(
        env!("CARGO_BIN_EXE_cargo-mobench"),
        bin_dir.path().join(wrapper_name),
    )
    .expect("copy cargo-mobench onto PATH");

    let mut paths = vec![bin_dir.path().to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let path = std::env::join_paths(paths).expect("join PATH");
    let output = Command::new(env!("CARGO"))
        .args(["mobench", "--version"])
        .env("PATH", path)
        .output()
        .expect("run cargo mobench");

    assert_version_output(output);
}

#[test]
fn cargo_mobench_binary_also_accepts_direct_invocation() {
    assert_version_output(run(env!("CARGO_BIN_EXE_cargo-mobench"), &["--version"]));
}
