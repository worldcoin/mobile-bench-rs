//! Deterministic contract consumed by mobench.org.
//!
//! The checked-in manifest describes the latest published release, not an
//! unreleased working tree. Updating the release pin is therefore an explicit
//! part of the release process.

use crate::{Cli, config::MobenchConfig};
use anyhow::{Context, Result, ensure};
use clap::{Arg, Command, CommandFactory};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const PUBLISHED_RELEASE_SHA: &str = "d1a3176f9144f35e777e83fd07045116144da257";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteManifest {
    pub schema_version: u32,
    pub release: ReleaseIdentity,
    pub msrv: String,
    pub binaries: Vec<BinaryContract>,
    pub command_tree: CommandContract,
    pub config: Vec<ConfigContract>,
    pub artifacts: Vec<ArtifactContract>,
    pub schemas: Vec<SchemaContract>,
    pub capabilities: Vec<CapabilityContract>,
    pub evidence: Vec<EvidenceContract>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseIdentity {
    pub version: String,
    pub tag: String,
    pub sha: String,
    pub repository: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryContract {
    pub name: String,
    pub invocation: String,
    pub canonical_for_release: bool,
    pub status: CapabilityStatus,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandContract {
    pub name: String,
    pub path: String,
    pub about: Option<String>,
    pub options: Vec<OptionContract>,
    pub subcommands: Vec<CommandContract>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionContract {
    pub id: String,
    pub long: Option<String>,
    pub short: Option<char>,
    pub positional_index: Option<usize>,
    pub required: bool,
    pub global: bool,
    pub hidden: bool,
    pub action: String,
    pub value_names: Vec<String>,
    pub possible_values: Vec<String>,
    pub defaults: Vec<String>,
    pub environment: Option<String>,
    pub help: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigContract {
    pub file: String,
    pub keys: Vec<ConfigKeyContract>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigKeyContract {
    pub path: String,
    pub value_type: String,
    pub required: bool,
    pub default: Option<Value>,
    pub evidence_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactContract {
    pub id: String,
    pub path_pattern: String,
    pub produced_by: Vec<String>,
    pub description: String,
    pub evidence_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaContract {
    pub id: String,
    pub path: String,
    pub checksum_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityStatus {
    Supported,
    Preview,
    Planned,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityContract {
    pub id: String,
    pub status: CapabilityStatus,
    pub summary: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceContract {
    pub id: String,
    pub kind: String,
    pub reference: String,
}

pub fn generate() -> Result<SiteManifest> {
    let mut root = Cli::command();
    root.build();

    let manifest = SiteManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        release: ReleaseIdentity {
            version: env!("CARGO_PKG_VERSION").to_string(),
            tag: format!("v{}", env!("CARGO_PKG_VERSION")),
            sha: PUBLISHED_RELEASE_SHA.to_string(),
            repository: env!("CARGO_PKG_REPOSITORY").to_string(),
        },
        msrv: env!("CARGO_PKG_RUST_VERSION").to_string(),
        binaries: binary_contracts(),
        command_tree: command_contract(&root, ""),
        config: config_contracts()?,
        artifacts: artifact_contracts(),
        schemas: schema_contracts(),
        capabilities: capability_contracts(),
        evidence: evidence_contracts(),
    };
    validate(&manifest)?;
    Ok(manifest)
}

pub fn to_pretty_json(manifest: &SiteManifest) -> Result<String> {
    let mut output = serde_json::to_string_pretty(manifest).context("serialize site manifest")?;
    output.push('\n');
    Ok(output)
}

pub fn validate(manifest: &SiteManifest) -> Result<()> {
    ensure!(
        manifest.schema_version == MANIFEST_SCHEMA_VERSION,
        "unsupported manifest schema version {}",
        manifest.schema_version
    );
    ensure!(
        manifest.release.sha.len() == 40
            && manifest.release.sha.chars().all(|c| c.is_ascii_hexdigit()),
        "release SHA must be 40 hexadecimal characters"
    );
    ensure!(
        manifest.release.tag == format!("v{}", manifest.release.version),
        "release tag and version disagree"
    );
    ensure!(!manifest.msrv.is_empty(), "MSRV is required");

    let evidence_ids = unique_ids(
        manifest.evidence.iter().map(|item| item.id.as_str()),
        "evidence",
    )?;
    unique_ids(
        manifest.capabilities.iter().map(|item| item.id.as_str()),
        "capability",
    )?;
    unique_ids(
        manifest.artifacts.iter().map(|item| item.id.as_str()),
        "artifact",
    )?;
    unique_ids(
        manifest.schemas.iter().map(|item| item.id.as_str()),
        "schema",
    )?;
    validate_command(&manifest.command_tree)?;

    for capability in &manifest.capabilities {
        ensure!(
            !capability.evidence_ids.is_empty(),
            "capability {} has no evidence",
            capability.id
        );
        for evidence_id in &capability.evidence_ids {
            ensure!(
                evidence_ids.contains(evidence_id.as_str()),
                "capability {} references unknown evidence {}",
                capability.id,
                evidence_id
            );
        }
    }
    for artifact in &manifest.artifacts {
        ensure!(
            evidence_ids.contains(artifact.evidence_id.as_str()),
            "artifact {} references unknown evidence {}",
            artifact.id,
            artifact.evidence_id
        );
    }
    for config in &manifest.config {
        for key in &config.keys {
            ensure!(
                evidence_ids.contains(key.evidence_id.as_str()),
                "config key {} references unknown evidence {}",
                key.path,
                key.evidence_id
            );
        }
    }
    Ok(())
}

fn unique_ids<'a>(ids: impl Iterator<Item = &'a str>, label: &str) -> Result<BTreeSet<&'a str>> {
    let mut unique = BTreeSet::new();
    for id in ids {
        ensure!(!id.is_empty(), "{label} ID cannot be empty");
        ensure!(unique.insert(id), "duplicate {label} ID {id}");
    }
    Ok(unique)
}

fn validate_command(command: &CommandContract) -> Result<()> {
    unique_ids(
        command.options.iter().map(|item| item.id.as_str()),
        "option",
    )?;
    unique_ids(
        command.subcommands.iter().map(|item| item.name.as_str()),
        "subcommand",
    )?;
    for subcommand in &command.subcommands {
        validate_command(subcommand)?;
    }
    Ok(())
}

fn command_contract(command: &Command, parent_path: &str) -> CommandContract {
    let name = command.get_name().to_string();
    let path = if parent_path.is_empty() {
        name.clone()
    } else {
        format!("{parent_path} {name}")
    };
    let mut options = command
        .get_arguments()
        .map(option_contract)
        .collect::<Vec<_>>();
    options.sort_by(|left, right| left.id.cmp(&right.id));
    let mut subcommands = command
        .get_subcommands()
        .map(|subcommand| command_contract(subcommand, &path))
        .collect::<Vec<_>>();
    subcommands.sort_by(|left, right| left.name.cmp(&right.name));

    CommandContract {
        name,
        path,
        about: command.get_about().map(ToString::to_string),
        options,
        subcommands,
    }
}

fn option_contract(arg: &Arg) -> OptionContract {
    let mut possible_values: Vec<String> = arg
        .get_value_parser()
        .possible_values()
        .map(|values| values.map(|value| value.get_name().to_string()).collect())
        .unwrap_or_default();
    possible_values.sort();

    OptionContract {
        id: arg.get_id().to_string(),
        long: arg.get_long().map(ToString::to_string),
        short: arg.get_short(),
        positional_index: arg.get_index(),
        required: arg.is_required_set(),
        global: arg.is_global_set(),
        hidden: arg.is_hide_set(),
        action: format!("{:?}", arg.get_action()),
        value_names: arg
            .get_value_names()
            .map(|values| values.iter().map(ToString::to_string).collect())
            .unwrap_or_default(),
        possible_values,
        defaults: arg
            .get_default_values()
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect(),
        environment: arg
            .get_env()
            .map(|value| value.to_string_lossy().into_owned()),
        help: arg.get_help().map(ToString::to_string),
    }
}

fn binary_contracts() -> Vec<BinaryContract> {
    vec![
        BinaryContract {
            name: "mobench".into(),
            invocation: "mobench <command>".into(),
            canonical_for_release: true,
            status: CapabilityStatus::Supported,
            note: "Canonical executable for the pinned v0.1.43 release.".into(),
        },
        BinaryContract {
            name: "cargo-mobench".into(),
            invocation: "cargo mobench <command>".into(),
            canonical_for_release: false,
            status: CapabilityStatus::Unsupported,
            note: "The published v0.1.43 wrapper forwards Cargo's injected `mobench` token. A source fix is integration-tested but requires a later release.".into(),
        },
    ]
}

fn config_contracts() -> Result<Vec<ConfigContract>> {
    let defaults = serde_json::to_value(MobenchConfig::default())
        .context("serialize mobench.toml defaults")?;
    let mut default_values = BTreeMap::new();
    flatten_json("", &defaults, &mut default_values);

    let key_types = [
        ("project.crate", "string|null"),
        ("project.library_name", "string|null"),
        ("project.output_dir", "path|null"),
        ("android.package", "string"),
        ("android.min_sdk", "integer"),
        ("android.target_sdk", "integer"),
        ("android.abis", "string[]|null"),
        ("ios.bundle_id", "string"),
        ("ios.deployment_target", "string"),
        ("ios.team_id", "string|null"),
        ("benchmarks.default_function", "string|null"),
        ("benchmarks.default_iterations", "integer"),
        ("benchmarks.default_warmup", "integer"),
        ("browserstack.ios_completion_timeout_secs", "integer|null"),
    ];
    let mobench_keys = key_types
        .into_iter()
        .map(|(path, value_type)| ConfigKeyContract {
            path: path.into(),
            value_type: value_type.into(),
            required: false,
            default: default_values.get(path).cloned(),
            evidence_id: "source-mobench-config".into(),
        })
        .collect();

    let bench_config_keys = [
        ("target", "android|ios", true),
        ("function", "string", true),
        ("iterations", "integer", true),
        ("warmup", "integer", true),
        ("device_matrix", "path", true),
        ("device_tags", "string[]|null", false),
        ("browserstack.app_automate_username", "string", true),
        ("browserstack.app_automate_access_key", "string", true),
        ("browserstack.project", "string|null", false),
        (
            "browserstack.ios_completion_timeout_secs",
            "integer|null",
            false,
        ),
        (
            "browserstack.android_benchmark_timeout_secs",
            "integer|null",
            false,
        ),
        (
            "browserstack.android_heartbeat_interval_secs",
            "integer|null",
            false,
        ),
        ("ios_xcuitest.app", "path", false),
        ("ios_xcuitest.test_suite", "path", false),
    ]
    .into_iter()
    .map(|(path, value_type, required)| ConfigKeyContract {
        path: path.into(),
        value_type: value_type.into(),
        required,
        default: None,
        evidence_id: "source-bench-config".into(),
    })
    .collect();

    Ok(vec![
        ConfigContract {
            file: "mobench.toml".into(),
            keys: mobench_keys,
        },
        ConfigContract {
            file: "bench-config.toml".into(),
            keys: bench_config_keys,
        },
    ])
}

fn flatten_json(prefix: &str, value: &Value, values: &mut BTreeMap<String, Value>) {
    if let Value::Object(object) = value {
        for (key, child) in object {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            if child.is_object() {
                flatten_json(&path, child, values);
            } else {
                values.insert(path, child.clone());
            }
        }
    }
}

fn artifact_contracts() -> Vec<ArtifactContract> {
    [
        (
            "benchmark-spec",
            "target/mobench/{android|ios}/**/bench_spec.json",
            vec!["mobench run", "mobench build"],
            "Resolved benchmark request embedded in generated runners.",
            "source-run-artifacts",
        ),
        (
            "benchmark-metadata",
            "target/mobench/{android|ios}/**/bench_meta.json",
            vec!["mobench run", "mobench build"],
            "Build metadata embedded beside the benchmark spec.",
            "source-run-artifacts",
        ),
        (
            "android-app",
            "target/mobench/android/app/build/outputs/apk/{debug|release}/*.apk",
            vec!["mobench build", "mobench run"],
            "Generated Android app and instrumentation packages.",
            "source-build-artifacts",
        ),
        (
            "ios-framework",
            "target/mobench/ios/<library>.xcframework",
            vec!["mobench build", "mobench run"],
            "Generated iOS xcframework.",
            "source-build-artifacts",
        ),
        (
            "ios-app",
            "target/mobench/ios/BenchRunner.ipa",
            vec!["mobench package-ipa", "mobench run"],
            "Packaged iOS benchmark application.",
            "source-build-artifacts",
        ),
        (
            "ios-test-suite",
            "target/mobench/ios/BenchRunnerUITests.zip",
            vec!["mobench package-xcuitest", "mobench run"],
            "Packaged XCUITest suite for BrowserStack.",
            "source-build-artifacts",
        ),
        (
            "ci-summary-json",
            "<output-dir>/summary.json",
            vec!["mobench ci run", "mobench ci merge-split-runs"],
            "Machine-readable benchmark summary.",
            "test-ci-artifacts",
        ),
        (
            "ci-summary-markdown",
            "<output-dir>/summary.md",
            vec!["mobench ci run", "mobench ci merge-split-runs"],
            "Human-readable benchmark summary.",
            "test-ci-artifacts",
        ),
        (
            "ci-results-csv",
            "<output-dir>/results.csv",
            vec!["mobench ci run", "mobench ci merge-split-runs"],
            "Tabular benchmark results.",
            "test-ci-artifacts",
        ),
        (
            "profile-manifest",
            "<output-dir>/<run-id>/profile.json",
            vec!["mobench profile run"],
            "Normalized profile manifest, plus a latest convenience copy.",
            "test-profile-artifacts",
        ),
        (
            "profile-summary",
            "<output-dir>/<run-id>/summary.md",
            vec!["mobench profile run"],
            "Profile summary, plus a latest convenience copy.",
            "test-profile-artifacts",
        ),
        (
            "profile-folded-stacks",
            "<output-dir>/<run-id>/artifacts/processed/stacks.folded",
            vec!["mobench profile run"],
            "Processed native stack samples when capture succeeds.",
            "test-profile-artifacts",
        ),
        (
            "profile-native-report",
            "<output-dir>/<run-id>/artifacts/processed/native-report.txt",
            vec!["mobench profile run"],
            "Symbolized native profiler report when available.",
            "test-profile-artifacts",
        ),
        (
            "profile-flamegraphs",
            "<output-dir>/<run-id>/artifacts/processed/flamegraph.{full|focused}.svg",
            vec!["mobench profile run"],
            "Full and benchmark-focused flamegraph SVGs when capture succeeds.",
            "test-profile-artifacts",
        ),
        (
            "profile-viewer",
            "<output-dir>/<run-id>/artifacts/processed/flamegraph.html",
            vec!["mobench profile run"],
            "Standalone profile viewer when capture succeeds.",
            "test-profile-artifacts",
        ),
        (
            "trace-events",
            "<trace-events-output>",
            vec!["mobench profile run"],
            "Machine-readable structured trace-event contract.",
            "test-trace-events",
        ),
    ]
    .into_iter()
    .map(
        |(id, path_pattern, produced_by, description, evidence_id)| ArtifactContract {
            id: id.into(),
            path_pattern: path_pattern.into(),
            produced_by: produced_by.into_iter().map(str::to_string).collect(),
            description: description.into(),
            evidence_id: evidence_id.into(),
        },
    )
    .collect()
}

fn schema_contracts() -> Vec<SchemaContract> {
    [
        (
            "summary-v1",
            "docs/schemas/summary-v1.schema.json",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/schemas/summary-v1.schema.json"
            )),
        ),
        (
            "ci-contract-v1",
            "docs/schemas/ci-contract-v1.schema.json",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/schemas/ci-contract-v1.schema.json"
            )),
        ),
        (
            "trace-events-v1",
            "docs/schemas/trace-events-v1.schema.json",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/schemas/trace-events-v1.schema.json"
            )),
        ),
        (
            "mobench-site-manifest-v1",
            "docs/schemas/mobench-site-manifest-v1.schema.json",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/schemas/mobench-site-manifest-v1.schema.json"
            )),
        ),
    ]
    .into_iter()
    .map(|(id, path, contents)| SchemaContract {
        id: id.into(),
        path: path.into(),
        checksum_sha256: format!("{:x}", Sha256::digest(contents.as_bytes())),
    })
    .collect()
}

fn capability_contracts() -> Vec<CapabilityContract> {
    use CapabilityStatus::{Planned, Supported, Unsupported};
    [
        ("benchmark.build.android", Supported, "Build generated Android benchmark artifacts.", vec!["source-build-artifacts"]),
        ("benchmark.build.ios", Supported, "Build generated iOS benchmark artifacts.", vec!["source-build-artifacts"]),
        ("benchmark.execute.browserstack.android", Supported, "Execute Android benchmarks through BrowserStack App Automate when devices and credentials are provided.", vec!["source-browserstack-run"]),
        ("benchmark.execute.browserstack.ios", Supported, "Execute iOS benchmarks through BrowserStack XCUITest when devices, credentials, and packages are provided.", vec!["source-browserstack-run"]),
        ("benchmark.execute.local-only", Unsupported, "`--local-only` writes preflight/spec outputs but skips the mobile build and does not execute the requested benchmark in v0.1.43.", vec!["source-local-only"]),
        ("benchmark.execute.attached-device", Unsupported, "Ordinary `mobench run` does not execute on an attached phone in v0.1.43.", vec!["source-browserstack-run"]),
        ("benchmark.execute.android-emulator", Unsupported, "Ordinary `mobench run` has no Android emulator execution provider in v0.1.43.", vec!["source-browserstack-run"]),
        ("benchmark.execute.ios-simulator", Unsupported, "Ordinary `mobench run` has no iOS simulator execution provider in v0.1.43.", vec!["source-browserstack-run"]),
        ("benchmark.build-without-devices", Supported, "With no BrowserStack devices, `mobench run` builds artifacts and skips upload/execution.", vec!["source-browserstack-run"]),
        ("profile.local.android-native", Supported, "Attempt local Android simpleperf capture, symbolization, and flamegraph generation when required tools and a device are available.", vec!["source-android-profile", "test-android-profile"]),
        ("profile.local.ios-instruments", Supported, "Attempt local iOS simulator-host sample capture and flamegraph generation when required tools are available.", vec!["source-ios-profile", "test-ios-profile"]),
        ("profile.local.rust-tracing", Planned, "The manifest/trace contract exists, but local rust-tracing capture is not implemented.", vec!["source-rust-tracing"]),
        ("profile.browserstack.android-native", Unsupported, "BrowserStack native Android stack capture is not implemented.", vec!["test-browserstack-profile"]),
        ("profile.browserstack.ios-instruments", Unsupported, "BrowserStack native iOS Instruments capture is not implemented.", vec!["test-browserstack-profile"]),
        ("profile.browserstack.rust-tracing", Unsupported, "BrowserStack rust-tracing capture is not implemented.", vec!["source-rust-tracing"]),
    ]
    .into_iter()
    .map(|(id, status, summary, evidence_ids)| CapabilityContract {
        id: id.into(),
        status,
        summary: summary.into(),
        evidence_ids: evidence_ids.into_iter().map(str::to_string).collect(),
    })
    .collect()
}

fn evidence_contracts() -> Vec<EvidenceContract> {
    [
        ("source-mobench-config", "source", "crates/mobench/src/config.rs#MobenchConfig"),
        ("source-bench-config", "source", "crates/mobench/src/lib.rs#BenchConfig"),
        ("source-run-artifacts", "source", "crates/mobench/src/lib.rs#persist_mobile_spec"),
        ("source-build-artifacts", "source", "crates/mobench/src/lib.rs#cmd_build"),
        ("source-browserstack-run", "source", "crates/mobench/src/lib.rs#run_from"),
        ("source-local-only", "source", "crates/mobench/src/lib.rs#run_from:local_only"),
        ("source-android-profile", "source", "crates/mobench/src/profile.rs#execute_local_android_capture"),
        ("source-ios-profile", "source", "crates/mobench/src/profile.rs#execute_local_ios_capture"),
        ("source-rust-tracing", "source", "crates/mobench/src/profile.rs#execute_capture"),
        ("test-ci-artifacts", "test", "crates/mobench/src/split_runs.rs#merge_split_run_summaries"),
        ("test-profile-artifacts", "test", "crates/mobench/src/profile.rs#profile_run_writes_run_scoped_and_latest_manifest_files"),
        ("test-trace-events", "test", "crates/mobench/tests/profile_cli.rs#profile_run_dry_run_writes_trace_events_output_for_downstream_consumers"),
        ("test-android-profile", "test", "crates/mobench/src/profile.rs#android_backend_builds_capture_plan_with_flamegraph_artifacts"),
        ("test-ios-profile", "test", "crates/mobench/src/profile.rs#ios_backend_allocates_sample_and_flamegraph_artifacts"),
        ("test-browserstack-profile", "test", "crates/mobench/tests/profile_cli.rs#browserstack_profile_run_reports_unsupported_native_capture"),
        ("test-cargo-wrapper", "test", "crates/mobench/tests/invocation_cli.rs#cargo_mobench_invocation_works_through_cargo"),
    ]
    .into_iter()
    .map(|(id, kind, reference)| EvidenceContract {
        id: id.into(),
        kind: kind.into(),
        reference: reference.into(),
    })
    .collect()
}

pub fn write_to_path(path: &Path) -> Result<()> {
    let manifest = generate()?;
    std::fs::write(path, to_pretty_json(&manifest)?)
        .with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
    }

    #[test]
    fn checked_in_manifest_is_deterministic_and_valid() {
        let generated = generate().expect("generate manifest");
        let generated_json = to_pretty_json(&generated).expect("serialize manifest");
        let checked_in_path = workspace_root().join("mobench-site-manifest-v1.json");
        let checked_in =
            std::fs::read_to_string(&checked_in_path).expect("read checked-in manifest");
        assert_eq!(
            checked_in, generated_json,
            "regenerate with `cargo run -p mobench --example generate-site-manifest`"
        );

        let schema: Value = serde_json::from_str(
            &std::fs::read_to_string(
                workspace_root().join("docs/schemas/mobench-site-manifest-v1.schema.json"),
            )
            .expect("read manifest schema"),
        )
        .expect("parse manifest schema");
        let validator = jsonschema::JSONSchema::compile(&schema).expect("compile manifest schema");
        let instance = serde_json::to_value(generated).expect("manifest JSON value");
        if let Err(errors) = validator.validate(&instance) {
            panic!(
                "manifest schema errors:\n{}",
                errors
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }

    #[test]
    fn command_contract_contains_every_runtime_command() {
        let manifest = generate().expect("generate manifest");
        let command_names = manifest
            .command_tree
            .subcommands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<BTreeSet<_>>();
        for expected in ["build", "ci", "devices", "profile", "run"] {
            assert!(command_names.contains(expected), "missing {expected}");
        }
        assert!(
            manifest
                .command_tree
                .subcommands
                .iter()
                .find(|command| command.name == "ci")
                .expect("ci command")
                .subcommands
                .iter()
                .any(|command| command.name == "run")
        );
    }
}
