use std::process::{Command, Output};

use serde::Deserialize;

const CONTRACT: &str = include_str!("fixtures/contracts/v0.1.43/cli-help.json");

#[derive(Debug, Deserialize)]
struct HelpContract {
    commands: Vec<HelpCommand>,
}

#[derive(Debug, Deserialize)]
struct HelpCommand {
    path: Vec<String>,
    contains: Vec<String>,
}

fn invoke(binary: &str, path: &[String], trailing: &str) -> Output {
    Command::new(binary)
        .args(path)
        .arg(trailing)
        .env_remove("MOBENCH_LOG")
        .output()
        .unwrap_or_else(|error| panic!("failed to run {binary:?} for {path:?}: {error}"))
}

fn assert_clean_success(output: &Output, binary: &str, path: &[String]) -> String {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{binary} {path:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{binary} {path:?} wrote stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone())
        .unwrap_or_else(|error| panic!("{binary} {path:?} emitted non-UTF-8 help: {error}"))
}

#[test]
fn v0_1_43_help_tree_and_cargo_wrapper_are_stable() {
    let contract: HelpContract = serde_json::from_str(CONTRACT).expect("valid help contract");
    let direct = env!("CARGO_BIN_EXE_mobench");
    let cargo_wrapper = env!("CARGO_BIN_EXE_cargo-mobench");

    for command in contract.commands {
        let direct_output = invoke(direct, &command.path, "--help");
        let direct_help = assert_clean_success(&direct_output, direct, &command.path);
        for expected in &command.contains {
            assert!(
                direct_help.contains(expected),
                "mobench {:?} help omitted {expected:?}:\n{direct_help}",
                command.path
            );
        }

        let wrapper_output = invoke(cargo_wrapper, &command.path, "--help");
        let wrapper_help = assert_clean_success(&wrapper_output, cargo_wrapper, &command.path);
        let normalized_wrapper_help = wrapper_help.replace("cargo-mobench", "mobench");
        assert_eq!(
            normalized_wrapper_help, direct_help,
            "direct and cargo-wrapper help drifted for {:?}",
            command.path
        );
    }
}

#[test]
fn current_version_and_exit_streams_are_stable() {
    for binary in [
        env!("CARGO_BIN_EXE_mobench"),
        env!("CARGO_BIN_EXE_cargo-mobench"),
    ] {
        let output = invoke(binary, &[], "--version");
        let stdout = assert_clean_success(&output, binary, &[]);
        assert_eq!(
            stdout.trim(),
            format!("mobench {}", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn v0_1_43_parse_errors_use_clap_exit_code_and_stderr() {
    for binary in [
        env!("CARGO_BIN_EXE_mobench"),
        env!("CARGO_BIN_EXE_cargo-mobench"),
    ] {
        let output = Command::new(binary)
            .arg("not-a-command")
            .output()
            .expect("run invalid command");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).expect("UTF-8 clap error");
        assert!(stderr.contains("unrecognized subcommand 'not-a-command'"));
        assert!(stderr.contains("Usage:"));

        let output = Command::new(binary)
            .arg("compare")
            .output()
            .expect("run incomplete command");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).expect("UTF-8 clap error");
        assert!(stderr.contains("required arguments were not provided"));
        assert!(stderr.contains("--baseline <BASELINE>"));
        assert!(stderr.contains("--candidate <CANDIDATE>"));
    }
}
