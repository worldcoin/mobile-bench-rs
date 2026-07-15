//! Compile-time receipt for the published v0.1.43 `mobench` Rust surface.

use mobench::config::{
    AndroidConfig, BenchmarksConfig, BrowserStackConfig, IosConfig, MobenchConfig, ProjectConfig,
};
use mobench::{
    DeviceSelection, ExtractedBenchmarkResult, MobileTarget, Report, RunRequest, RunResult,
};
use serde::Deserialize;

const BASELINE: &str = include_str!("fixtures/contracts/v0.1.43/baseline.json");
const HOST_PERFORMANCE: &str = include_str!("fixtures/contracts/v0.1.43/host-performance.json");
const FINDINGS: &str = include_str!("fixtures/contracts/v0.1.43/findings.json");

#[derive(Deserialize)]
struct BaselineReceipt {
    implementation_revision: String,
    default_branch: String,
    release_oracle: ReleaseOracle,
    published_crates: PublishedCrates,
    workspace: WorkspaceReceipt,
}

#[derive(Deserialize)]
struct ReleaseOracle {
    tag: String,
    revision: String,
    draft: bool,
    prerelease: bool,
}

#[derive(Deserialize)]
struct PublishedCrates {
    mobench: String,
    #[serde(rename = "mobench-sdk")]
    mobench_sdk: String,
    #[serde(rename = "mobench-macros")]
    mobench_macros: String,
}

#[derive(Deserialize)]
struct WorkspaceReceipt {
    version: String,
    edition: String,
    rust_version: String,
}

#[derive(Deserialize)]
struct HostPerformanceReceipt {
    implementation_revision: String,
    release_oracle_revision: String,
    harness: HostPerformanceHarness,
    measurements: Vec<HostPerformanceMeasurement>,
}

#[derive(Deserialize)]
struct HostPerformanceHarness {
    command: String,
    sample_size: u32,
    warm_up_seconds: u32,
    measurement_seconds: u32,
}

#[derive(Deserialize)]
struct HostPerformanceMeasurement {
    benchmark: String,
    estimate_interval_microseconds: EstimateInterval,
}

#[derive(Deserialize)]
struct EstimateInterval {
    lower: f64,
    upper: f64,
}

#[derive(Deserialize)]
struct FindingsReceipt {
    decision: String,
    audit_revision: String,
    implementation_revision: String,
    findings: Vec<FindingReceipt>,
}

#[derive(Deserialize)]
struct FindingReceipt {
    id: String,
    regression: String,
    owner: String,
    first_passing_phase: u32,
    removal_gate: String,
}

#[test]
fn v0_1_43_root_exports_remain_nameable() {
    let _: Option<MobileTarget> = None;
    let _: Option<DeviceSelection> = None;
    let _: Option<RunRequest> = None;
    let _: Option<Report> = None;
    let _: Option<RunResult> = None;
    let _: Option<ExtractedBenchmarkResult> = None;

    let _ = mobench::run;
    let _ = mobench::run_request;
    let _ = mobench::extract_benchmark_summary;
}

#[test]
fn v0_1_43_config_exports_remain_nameable() {
    let _: Option<MobenchConfig> = None;
    let _: Option<ProjectConfig> = None;
    let _: Option<AndroidConfig> = None;
    let _: Option<IosConfig> = None;
    let _: Option<BenchmarksConfig> = None;
    let _: Option<BrowserStackConfig> = None;
    assert_eq!(mobench::config::CONFIG_FILE_NAME, "mobench.toml");
}

#[test]
fn adr_000_baseline_receipt_matches_the_workspace_contract() {
    let receipt: BaselineReceipt = serde_json::from_str(BASELINE).expect("valid ADR-000 receipt");
    assert_eq!(receipt.implementation_revision.len(), 40);
    assert_eq!(receipt.default_branch, "main");
    assert_eq!(receipt.release_oracle.tag, "v0.1.43");
    assert_eq!(receipt.release_oracle.revision.len(), 40);
    assert!(!receipt.release_oracle.draft);
    assert!(!receipt.release_oracle.prerelease);
    assert_eq!(receipt.published_crates.mobench, env!("CARGO_PKG_VERSION"));
    assert_eq!(receipt.published_crates.mobench_sdk, "0.1.43");
    assert_eq!(receipt.published_crates.mobench_macros, "0.1.43");
    assert_eq!(receipt.workspace.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(receipt.workspace.edition, "2024");
    assert_eq!(receipt.workspace.rust_version, "1.85");
}

#[test]
fn adr_000_host_performance_receipt_is_complete_and_ordered() {
    let receipt: HostPerformanceReceipt =
        serde_json::from_str(HOST_PERFORMANCE).expect("valid host performance receipt");
    assert_eq!(receipt.implementation_revision.len(), 40);
    assert_eq!(receipt.release_oracle_revision.len(), 40);
    assert!(receipt.harness.command.contains("--bench host_contracts"));
    assert_eq!(receipt.harness.sample_size, 20);
    assert_eq!(receipt.harness.warm_up_seconds, 1);
    assert_eq!(receipt.harness.measurement_seconds, 1);

    let benchmark_names = receipt
        .measurements
        .iter()
        .map(|measurement| measurement.benchmark.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        benchmark_names,
        [
            "config/parse_run_config",
            "config/parse_device_matrix",
            "summary/render_markdown",
            "summary/render_csv",
            "profile/render_markdown",
            "browserstack/extract_results",
        ]
    );
    for measurement in &receipt.measurements {
        assert!(measurement.estimate_interval_microseconds.lower > 0.0);
        assert!(
            measurement.estimate_interval_microseconds.upper
                >= measurement.estimate_interval_microseconds.lower
        );
    }
}

#[test]
fn adr_000_finding_map_is_complete_and_traceable() {
    let receipt: FindingsReceipt = serde_json::from_str(FINDINGS).expect("valid finding map");
    assert_eq!(receipt.decision, "ADR-000");
    assert_eq!(receipt.audit_revision.len(), 40);
    assert_eq!(receipt.implementation_revision.len(), 40);
    assert_eq!(receipt.findings.len(), 15);

    let ids = receipt
        .findings
        .iter()
        .map(|finding| finding.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        ids.len(),
        receipt.findings.len(),
        "finding IDs must be unique"
    );
    for finding in &receipt.findings {
        assert!(!finding.regression.trim().is_empty());
        assert!(!finding.owner.trim().is_empty());
        assert_eq!(finding.first_passing_phase, 1);
        assert!(!finding.removal_gate.trim().is_empty());
    }
}
