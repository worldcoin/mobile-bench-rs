use std::ffi::OsStr;
use std::process::{Command, Output};

fn run_mobench<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_mobench"))
        .args(args)
        .output()
        .expect("run mobench binary")
}

#[test]
fn browserstack_profile_run_reports_unsupported_native_capture() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let output = run_mobench([
        "profile",
        "run",
        "--target",
        "ios",
        "--backend",
        "ios-instruments",
        "--provider",
        "browserstack",
        "--crate-path",
        "crates/zk-mobile-bench",
        "--function",
        "zk_mobile_bench::bench_query_proof_generation",
        "--output-dir",
        temp_dir.path().to_str().expect("utf-8 path"),
    ]);

    assert!(!output.status.success(), "expected unsupported run to fail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("BrowserStack"),
        "expected BrowserStack failure context, got:\n{stderr}"
    );
    assert!(
        stderr.contains("unsupported") || stderr.contains("not implemented"),
        "expected unsupported capability explanation, got:\n{stderr}"
    );
}

#[test]
fn browserstack_native_profile_error_is_actionable() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let output = run_mobench([
        "profile",
        "run",
        "--target",
        "ios",
        "--backend",
        "ios-instruments",
        "--provider",
        "browserstack",
        "--crate-path",
        "crates/zk-mobile-bench",
        "--function",
        "zk_mobile_bench::bench_query_proof_generation",
        "--output-dir",
        temp_dir.path().to_str().expect("utf-8 path"),
    ]);

    assert!(!output.status.success(), "expected unsupported run to fail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("BrowserStack native profiling is not implemented"),
        "expected explicit unsupported wording, got:\n{stderr}"
    );
    assert!(
        stderr.contains("local-first profile contract")
            || stderr.contains("planned artifact contract only"),
        "expected explanation that the command only records planned artifacts today, got:\n{stderr}"
    );
    assert!(
        stderr.contains("Use --provider local"),
        "expected an actionable local fallback, got:\n{stderr}"
    );
    assert!(
        stderr.contains("Instruments")
            || stderr.contains("time-profiler.trace")
            || stderr.contains("time-profiler.xml")
            || stderr.contains("flamegraph"),
        "expected iOS artifact clarification, got:\n{stderr}"
    );
}

#[test]
fn profile_run_help_mentions_planned_only_or_execution_scope() {
    let output = run_mobench(["profile", "run", "--help"]);
    assert!(output.status.success(), "expected help to succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("plan")
            || stdout.contains("Plan")
            || stdout.contains("depending on backend/provider support"),
        "expected help to explain whether capture is planned or executed, got:\n{stdout}"
    );
    assert!(
        stdout.contains("BrowserStack") || stdout.contains("browserstack"),
        "expected help to mention BrowserStack capability scope, got:\n{stdout}"
    );
    assert!(
        stdout.contains("--warmup-mode"),
        "expected help to expose warm/cold capture mode, got:\n{stdout}"
    );
}

#[test]
fn profile_run_cli_surface_exposes_or_explicitly_omits_device_selection() {
    let output = run_mobench(["profile", "run", "--help"]);
    assert!(output.status.success(), "expected help to succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--device")
            || stdout.contains("--profile")
            || stdout.contains("--device-matrix")
            || stdout.contains("device selection is unavailable"),
        "expected help to expose device selection or explicitly document its absence, got:\n{stdout}"
    );
}

#[test]
fn profile_diff_help_exposes_runtime_command_surface() {
    let output = run_mobench(["profile", "diff", "--help"]);
    assert!(output.status.success(), "expected diff help to succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--baseline"),
        "expected baseline arg, got:\n{stdout}"
    );
    assert!(
        stdout.contains("--candidate"),
        "expected candidate arg, got:\n{stdout}"
    );
    assert!(
        stdout.contains("--normalize"),
        "expected normalize flag, got:\n{stdout}"
    );
}
