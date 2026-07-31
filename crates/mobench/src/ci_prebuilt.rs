//! Hashed prebuilt bundles for the BrowserStack credential boundary.
//!
//! `prepare` is intentionally the only half that resolves projects or invokes
//! build tooling. `run-prebuilt` accepts a closed, enumerated bundle and only
//! performs manifest verification, provider execution, collection, and report
//! rendering.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use mobench_report::{render_csv_summary, render_markdown_summary};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::browserstack::{BrowserStackAuth, BrowserStackClient};
use crate::ci::{fetch_browserstack_artifacts, merge_summary_reports};
use crate::cli::{CiPrepareArgs, CiRunPrebuiltArgs, FfiBackendArg, MobileTarget};
use crate::execution::{
    load_dotenv_for_layout, package_ios_xcuitest_artifacts, persist_mobile_spec, run_android_build,
    trigger_browserstack_espresso, trigger_browserstack_xcuitest,
};
use crate::project_layout::{ProjectLayoutOptions, resolve_project_layout};
use crate::report_binding::RunEnvelopeIdentity;
use crate::reporting::build_summary;
use crate::{
    IosXcuitestArtifacts, RemoteRun, RunSpec, RunSummary, SummaryReport,
    resolve_browserstack_credentials, write_file,
};

const PREBUILT_SCHEMA: &str = "mobench.prebuilt.v1";
const PREBUILT_ABI: &str = "mobench-bench-spec-v1";
const PREBUILT_MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const PREBUILT_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const PREBUILT_MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const PREBUILT_MAX_TIMEOUT_SECS: u64 = 6 * 60 * 60;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
enum PrebuiltArtifactKind {
    AndroidApp,
    AndroidTestSuite,
    IosApp,
    IosTestSuite,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrebuiltArtifact {
    kind: PrebuiltArtifactKind,
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrebuiltEntry {
    function: String,
    iterations: u32,
    warmup: u32,
    completion_timeout_secs: Option<u64>,
    artifacts: Vec<PrebuiltArtifact>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrebuiltAbi {
    benchmark: String,
    runner: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrebuiltManifest {
    schema: String,
    source_sha: String,
    platform: MobileTarget,
    build_profile: String,
    mobench_version: String,
    abi: PrebuiltAbi,
    entries: Vec<PrebuiltEntry>,
}

#[derive(Debug)]
struct VerifiedPrebuiltEntry {
    function: String,
    iterations: u32,
    warmup: u32,
    completion_timeout_secs: Option<u64>,
    app: PathBuf,
    test_suite: PathBuf,
}

#[derive(Debug)]
struct ValidatedPrebuiltResults {
    results: HashMap<String, Vec<Value>>,
    device_identities: HashMap<String, String>,
}

fn parse_prebuilt_functions(values: &[String]) -> Result<Vec<String>> {
    let mut functions = values.to_vec();
    if functions.len() == 1 {
        let value = functions[0].trim();
        if value.starts_with('[') {
            functions = serde_json::from_str(value).context("parsing --functions JSON array")?;
        } else if value.contains(',') {
            functions = value
                .split(',')
                .map(str::trim)
                .map(str::to_string)
                .collect();
        }
    }
    functions.retain(|function| !function.trim().is_empty());
    if functions.is_empty() {
        bail!("at least one benchmark function is required");
    }
    if functions.len() > 64 {
        bail!("at most 64 benchmark functions may be prepared");
    }
    for function in &functions {
        if function.len() > 512 || function.chars().any(char::is_control) {
            bail!("benchmark function names must be at most 512 printable characters");
        }
    }
    Ok(functions)
}

fn validate_full_commit_sha(value: &str) -> Result<String> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("source SHA must be a full 40-character hexadecimal commit SHA");
    }
    Ok(value.to_ascii_lowercase())
}

fn sha256_file(path: &Path) -> Result<(u64, String)> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("reading artifact metadata {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!(
            "prebuilt artifact must be a regular file: {}",
            path.display()
        );
    }
    if metadata.len() == 0 || metadata.len() > PREBUILT_MAX_FILE_BYTES {
        bail!(
            "prebuilt artifact size {} is outside 1..={} bytes: {}",
            metadata.len(),
            PREBUILT_MAX_FILE_BYTES,
            path.display()
        );
    }
    let mut file =
        fs::File::open(path).with_context(|| format!("opening artifact {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok((metadata.len(), format!("{:x}", digest.finalize())))
}

fn stage_prebuilt_artifact(
    source: &Path,
    root: &Path,
    relative: &str,
    kind: PrebuiltArtifactKind,
) -> Result<PrebuiltArtifact> {
    let destination = root.join(relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, &destination).with_context(|| {
        format!(
            "copying prebuilt artifact {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    let (size, sha256) = sha256_file(&destination)?;
    Ok(PrebuiltArtifact {
        kind,
        path: relative.to_string(),
        size,
        sha256,
    })
}

pub(crate) fn cmd_ci_prepare(args: CiPrepareArgs, dry_run: bool) -> Result<()> {
    let source_sha = validate_full_commit_sha(&args.source_sha)?;
    let functions = parse_prebuilt_functions(&args.functions)?;
    if args.manifest != args.output_dir.join("manifest.json") {
        bail!("--manifest must be <output-dir>/manifest.json");
    }
    let current_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .context("resolving checked-out commit for prebuilt manifest")?;
    let checked_out_sha = String::from_utf8_lossy(&current_sha.stdout)
        .trim()
        .to_ascii_lowercase();
    if !current_sha.status.success() || checked_out_sha != source_sha {
        bail!("checked-out commit `{checked_out_sha}` does not match --source-sha `{source_sha}`");
    }
    if dry_run {
        println!(
            "Would prepare {} {} prebuilt benchmark artifact set(s) at {}",
            functions.len(),
            args.target.as_str(),
            args.output_dir.display()
        );
        return Ok(());
    }

    if fs::symlink_metadata(&args.output_dir).is_ok() {
        bail!(
            "prebuilt output directory already exists; remove it before preparing: {}",
            args.output_dir.display()
        );
    }
    fs::create_dir_all(&args.output_dir)?;
    let mut layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: None,
        crate_path: args.crate_path.as_deref(),
        config_path: None,
    })?;
    layout.ffi_backend = effective_ci_prepare_ffi_backend(layout.ffi_backend, args.ffi_backend);
    load_dotenv_for_layout(&layout);

    let mut entries = Vec::with_capacity(functions.len());
    for (index, function) in functions.into_iter().enumerate() {
        let spec = RunSpec {
            target: args.target,
            function: function.clone(),
            iterations: args.iterations,
            warmup: args.warmup,
            devices: Vec::new(),
            ios_completion_timeout_secs: layout.ios_completion_timeout_secs,
            ios_deployment_target: Some(layout.ios_deployment_target.clone()),
            ios_runner: layout.ios_runner.clone(),
            android_benchmark_timeout_secs: layout.android_benchmark_timeout_secs,
            android_heartbeat_interval_secs: layout.android_heartbeat_interval_secs,
            browserstack: None,
            ios_xcuitest: None,
        };
        let identity = RunEnvelopeIdentity::generate(args.target)?;
        persist_mobile_spec(&layout, &spec, &identity, args.release)?;
        let prefix = format!("entries/{index:04}");
        let artifacts = match args.target {
            MobileTarget::Android => {
                let ndk = env::var("ANDROID_NDK_HOME")
                    .context("ANDROID_NDK_HOME must be set for Android prepare")?;
                let build = run_android_build(&layout, &ndk, args.release, false)?;
                let test_suite = build
                    .test_suite_path
                    .context("Android prepare did not produce a test-suite APK")?;
                vec![
                    stage_prebuilt_artifact(
                        &build.app_path,
                        &args.output_dir,
                        &format!("{prefix}/app.apk"),
                        PrebuiltArtifactKind::AndroidApp,
                    )?,
                    stage_prebuilt_artifact(
                        &test_suite,
                        &args.output_dir,
                        &format!("{prefix}/test.apk"),
                        PrebuiltArtifactKind::AndroidTestSuite,
                    )?,
                ]
            }
            MobileTarget::Ios => {
                let packaged = package_ios_xcuitest_artifacts(
                    &layout,
                    &spec,
                    &identity,
                    args.release,
                    layout.ios_completion_timeout_secs,
                    Some(&layout.ios_deployment_target),
                    layout.ios_runner.as_deref(),
                )?;
                vec![
                    stage_prebuilt_artifact(
                        &packaged.app,
                        &args.output_dir,
                        &format!("{prefix}/app.ipa"),
                        PrebuiltArtifactKind::IosApp,
                    )?,
                    stage_prebuilt_artifact(
                        &packaged.test_suite,
                        &args.output_dir,
                        &format!("{prefix}/test-suite.zip"),
                        PrebuiltArtifactKind::IosTestSuite,
                    )?,
                ]
            }
        };
        entries.push(PrebuiltEntry {
            function,
            iterations: args.iterations,
            warmup: args.warmup,
            completion_timeout_secs: match args.target {
                MobileTarget::Android => layout.android_benchmark_timeout_secs,
                MobileTarget::Ios => layout.ios_completion_timeout_secs,
            },
            artifacts,
        });
    }

    let manifest = PrebuiltManifest {
        schema: PREBUILT_SCHEMA.to_string(),
        source_sha,
        platform: args.target,
        build_profile: if args.release { "release" } else { "debug" }.to_string(),
        mobench_version: env!("CARGO_PKG_VERSION").to_string(),
        abi: PrebuiltAbi {
            benchmark: PREBUILT_ABI.to_string(),
            runner: match args.target {
                MobileTarget::Android => "browserstack-espresso-v2",
                MobileTarget::Ios => "browserstack-xcuitest-v2",
            }
            .to_string(),
        },
        entries,
    };
    write_file(
        &args.manifest,
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )?;
    println!("Prepared prebuilt manifest at {}", args.manifest.display());
    Ok(())
}

fn effective_ci_prepare_ffi_backend(
    configured: mobench_sdk::FfiBackend,
    explicit: Option<FfiBackendArg>,
) -> mobench_sdk::FfiBackend {
    explicit.map(Into::into).unwrap_or(configured)
}

fn validated_prebuilt_path(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty()
        || relative.contains('\\')
        || relative.chars().any(char::is_control)
        || Path::new(relative)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("invalid prebuilt artifact path `{relative}`");
    }
    Ok(root.join(relative))
}

fn expected_prebuilt_path(
    platform: MobileTarget,
    index: usize,
    kind: PrebuiltArtifactKind,
) -> Option<String> {
    let prefix = format!("entries/{index:04}");
    match (platform, kind) {
        (MobileTarget::Android, PrebuiltArtifactKind::AndroidApp) => {
            Some(format!("{prefix}/app.apk"))
        }
        (MobileTarget::Android, PrebuiltArtifactKind::AndroidTestSuite) => {
            Some(format!("{prefix}/test.apk"))
        }
        (MobileTarget::Ios, PrebuiltArtifactKind::IosApp) => Some(format!("{prefix}/app.ipa")),
        (MobileTarget::Ios, PrebuiltArtifactKind::IosTestSuite) => {
            Some(format!("{prefix}/test-suite.zip"))
        }
        _ => None,
    }
}

fn collect_regular_files(root: &Path, current: &Path, output: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("prebuilt bundle contains symlink {}", path.display());
        }
        if metadata.is_dir() {
            collect_regular_files(root, &path, output)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .context("enumerating prebuilt bundle")?
                .to_str()
                .context("prebuilt paths must be UTF-8")?
                .replace('\\', "/");
            output.insert(relative);
        } else {
            bail!(
                "prebuilt bundle contains non-regular file {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn verify_prebuilt_manifest(
    manifest_path: &Path,
    expected_source_sha: &str,
) -> Result<(PrebuiltManifest, Vec<VerifiedPrebuiltEntry>)> {
    let expected_source_sha = validate_full_commit_sha(expected_source_sha)?;
    let metadata = fs::symlink_metadata(manifest_path)
        .with_context(|| format!("reading manifest {}", manifest_path.display()))?;
    if !metadata.file_type().is_file() || metadata.len() > PREBUILT_MAX_MANIFEST_BYTES {
        bail!("prebuilt manifest must be a regular file no larger than 1 MiB");
    }
    let bytes = fs::read(manifest_path)?;
    let manifest: PrebuiltManifest = serde_json::from_slice(&bytes).context("parsing manifest")?;
    if manifest.schema != PREBUILT_SCHEMA
        || manifest.abi.benchmark != PREBUILT_ABI
        || manifest.source_sha != expected_source_sha
        || manifest.mobench_version != env!("CARGO_PKG_VERSION")
        || !matches!(manifest.build_profile.as_str(), "debug" | "release")
    {
        bail!("prebuilt manifest schema, ABI, producer version, profile, or source SHA mismatch");
    }
    let expected_runner = match manifest.platform {
        MobileTarget::Android => "browserstack-espresso-v2",
        MobileTarget::Ios => "browserstack-xcuitest-v2",
    };
    if manifest.abi.runner != expected_runner
        || manifest.entries.is_empty()
        || manifest.entries.len() > 64
    {
        bail!("prebuilt manifest runner or entry count is invalid");
    }
    let root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let root_metadata = fs::symlink_metadata(root)?;
    if !root_metadata.file_type().is_dir() {
        bail!("prebuilt bundle root must be a real directory");
    }
    if manifest_path.file_name().and_then(|name| name.to_str()) != Some("manifest.json") {
        bail!("prebuilt manifest must be named manifest.json");
    }
    let mut expected_files = BTreeSet::from(["manifest.json".to_string()]);
    let mut verified = Vec::with_capacity(manifest.entries.len());
    let mut functions = BTreeSet::new();
    let mut total_size = 0_u64;
    for (index, entry) in manifest.entries.iter().enumerate() {
        if entry.function.is_empty()
            || entry.function.len() > 512
            || entry.function.chars().any(char::is_control)
            || entry.iterations == 0
            || entry.iterations > 10_000
            || entry.warmup > 10_000
            || entry
                .completion_timeout_secs
                .is_some_and(|timeout| timeout == 0 || timeout > PREBUILT_MAX_TIMEOUT_SECS)
            || !functions.insert(entry.function.clone())
            || entry.artifacts.len() != 2
        {
            bail!("invalid or duplicate prebuilt entry at index {index}");
        }
        let mut app = None;
        let mut test_suite = None;
        let mut kinds = BTreeSet::new();
        for artifact in &entry.artifacts {
            if !kinds.insert(artifact.kind) {
                bail!("duplicate artifact kind in entry {index}");
            }
            let expected_path = expected_prebuilt_path(manifest.platform, index, artifact.kind)
                .ok_or_else(|| anyhow!("artifact kind does not match platform"))?;
            if artifact.path != expected_path
                || artifact.sha256.len() != 64
                || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                || artifact.size == 0
                || artifact.size > PREBUILT_MAX_FILE_BYTES
            {
                bail!("invalid artifact metadata for entry {index}");
            }
            total_size = total_size
                .checked_add(artifact.size)
                .context("prebuilt bundle total size overflow")?;
            if total_size > PREBUILT_MAX_TOTAL_BYTES {
                bail!("prebuilt bundle exceeds total size limit");
            }
            let path = validated_prebuilt_path(root, &artifact.path)?;
            let (actual_size, actual_hash) = sha256_file(&path)?;
            if actual_size != artifact.size || actual_hash != artifact.sha256.to_ascii_lowercase() {
                bail!("artifact size or SHA-256 mismatch for `{}`", artifact.path);
            }
            let mut magic = [0_u8; 4];
            fs::File::open(&path)?.read_exact(&mut magic)?;
            if !matches!(&magic, b"PK\x03\x04" | b"PK\x05\x06" | b"PK\x07\x08") {
                bail!(
                    "artifact `{}` is not a ZIP-based mobile package",
                    artifact.path
                );
            }
            expected_files.insert(artifact.path.clone());
            match artifact.kind {
                PrebuiltArtifactKind::AndroidApp | PrebuiltArtifactKind::IosApp => app = Some(path),
                PrebuiltArtifactKind::AndroidTestSuite | PrebuiltArtifactKind::IosTestSuite => {
                    test_suite = Some(path)
                }
            }
        }
        verified.push(VerifiedPrebuiltEntry {
            function: entry.function.clone(),
            iterations: entry.iterations,
            warmup: entry.warmup,
            completion_timeout_secs: entry.completion_timeout_secs,
            app: app.context("prebuilt entry missing app")?,
            test_suite: test_suite.context("prebuilt entry missing test suite")?,
        });
    }
    let mut actual_files = BTreeSet::new();
    collect_regular_files(root, root, &mut actual_files)?;
    if actual_files != expected_files {
        bail!("prebuilt bundle contains missing or unexpected files");
    }
    Ok((manifest, verified))
}

fn validate_prebuilt_results(
    function: &str,
    iterations: u32,
    warmup: u32,
    requested_devices: &[String],
    results: &HashMap<String, Vec<Value>>,
) -> Result<ValidatedPrebuiltResults> {
    let expected = requested_devices.iter().collect::<BTreeSet<_>>();
    if expected.len() != requested_devices.len() {
        bail!("requested BrowserStack devices must be unique");
    }
    let mut matched = BTreeSet::new();
    let mut canonical_by_observed = BTreeMap::new();
    let mut unexpected = Vec::new();
    for observed in results.keys() {
        let candidate = if let Some(exact) = expected.get(observed) {
            Some(*exact)
        } else {
            let candidates = expected
                .iter()
                .copied()
                .filter(|requested| crate::summarize::device_names_match(requested, observed))
                .collect::<Vec<_>>();
            if candidates.len() > 1 {
                bail!(
                    "ambiguous BrowserStack result device `{observed}` matched multiple requested devices"
                );
            }
            candidates.into_iter().next()
        };
        if let Some(candidate) = candidate {
            if !matched.insert(candidate) {
                bail!(
                    "duplicate BrowserStack result device `{observed}` matched requested device `{candidate}`"
                );
            }
            canonical_by_observed.insert(observed, candidate);
        } else {
            unexpected.push(observed);
        }
    }
    let missing = expected.difference(&matched).copied().collect::<Vec<_>>();
    if !missing.is_empty() || !unexpected.is_empty() {
        bail!(
            "incomplete BrowserStack result matrix for `{function}`; missing devices: {}; unexpected devices: {}",
            missing
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            unexpected
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let mut normalized = HashMap::new();
    let mut identities = HashMap::new();
    for (device, entries) in results {
        if entries.len() != 1 {
            bail!(
                "expected exactly one BrowserStack result shard for `{function}` on `{device}`, found {}",
                entries.len()
            );
        }
        let observed_function = entries[0]
            .get("function")
            .and_then(Value::as_str)
            .context("BrowserStack result omitted benchmark function")?;
        if observed_function != function {
            bail!("BrowserStack result function did not match prepared manifest");
        }
        validate_prebuilt_v2_envelope(&entries[0], function, iterations, warmup)?;
        let canonical = canonical_by_observed
            .get(device)
            .context("validated BrowserStack device was not canonicalized")?;
        normalized.insert((*canonical).clone(), entries.clone());
        identities.insert(device.clone(), (*canonical).clone());
    }
    Ok(ValidatedPrebuiltResults {
        results: normalized,
        device_identities: identities,
    })
}

fn validate_prebuilt_v2_envelope(
    report: &Value,
    function: &str,
    iterations: u32,
    warmup: u32,
) -> Result<()> {
    let envelope: mobench_domain::ReportEnvelopeV2 = serde_json::from_value(report.clone())
        .context("prebuilt BrowserStack result is not a strict mobench.run/v2 envelope")?;
    let identity = envelope.identity();
    let producer = identity.producer().as_str();
    if !matches!(producer, "android-runner" | "ios-runner") {
        bail!("prebuilt BrowserStack result has an unexpected producer");
    }
    let expected = mobench_domain::ExpectedReportIdentity::new(
        mobench_domain::ReportIdentity::new(
            identity.run_id().clone(),
            identity.nonce().clone(),
            identity.logical_session_id().clone(),
            mobench_domain::ReportIdentifier::parse(function.to_string())
                .context("prepared benchmark function is not a valid report identifier")?,
            mobench_domain::ReportIdentifier::parse(producer.to_string())
                .context("prebuilt report producer is invalid")?,
        ),
        mobench_domain::ReportCounts::new(iterations, warmup)
            .context("prepared benchmark counts are invalid")?,
    );
    expected
        .validate(&envelope)
        .context("prebuilt BrowserStack result failed strict v2 identity/count validation")
}

fn trusted_prebuilt_timeout(
    completion_timeout_secs: Option<u64>,
    fetch_timeout_secs: u64,
    max_completion_timeout_secs: u64,
) -> u64 {
    completion_timeout_secs
        .map(|timeout| timeout.saturating_add(60))
        .unwrap_or(0)
        .max(fetch_timeout_secs)
        .min(max_completion_timeout_secs)
}

pub(crate) fn cmd_ci_run_prebuilt(args: CiRunPrebuiltArgs, dry_run: bool) -> Result<()> {
    let (manifest, entries) = verify_prebuilt_manifest(&args.manifest, &args.expected_source_sha)?;
    let expected_functions = parse_prebuilt_functions(&args.expected_functions)?;
    let manifest_functions = manifest
        .entries
        .iter()
        .map(|entry| entry.function.clone())
        .collect::<Vec<_>>();
    if manifest.platform != args.expected_platform
        || manifest_functions != expected_functions
        || manifest.entries.iter().any(|entry| {
            entry.iterations != args.expected_iterations || entry.warmup != args.expected_warmup
        })
    {
        bail!("prebuilt manifest does not match the trusted requested benchmark configuration");
    }
    if args.devices.is_empty()
        || args.devices.iter().any(|device| {
            device.is_empty() || device.len() > 256 || device.chars().any(char::is_control)
        })
        || args.devices.iter().collect::<BTreeSet<_>>().len() != args.devices.len()
    {
        bail!("at least one printable BrowserStack device is required");
    }
    if args.fetch_timeout_secs == 0 || args.fetch_timeout_secs > PREBUILT_MAX_TIMEOUT_SECS {
        bail!("fetch timeout is outside the supported range");
    }
    if args.max_completion_timeout_secs == 0
        || args.max_completion_timeout_secs > PREBUILT_MAX_TIMEOUT_SECS
        || args.fetch_timeout_secs > args.max_completion_timeout_secs
    {
        bail!("maximum completion timeout is outside the supported range");
    }
    if dry_run {
        println!("Verified {} prebuilt entry or entries", entries.len());
        return Ok(());
    }

    fs::create_dir_all(&args.output_dir)?;
    let credentials = resolve_browserstack_credentials(None)?;
    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: credentials.username,
            access_key: credentials.access_key,
        },
        credentials.project,
    )?;
    let mut summaries = Vec::<SummaryReport>::new();
    let mut function_values = BTreeMap::new();
    for entry in entries {
        let timeout_secs = trusted_prebuilt_timeout(
            entry.completion_timeout_secs,
            args.fetch_timeout_secs,
            args.max_completion_timeout_secs,
        );
        let spec = RunSpec {
            target: manifest.platform,
            function: entry.function.clone(),
            iterations: entry.iterations,
            warmup: entry.warmup,
            devices: args.devices.clone(),
            ios_completion_timeout_secs: (manifest.platform == MobileTarget::Ios)
                .then_some(timeout_secs),
            ios_deployment_target: None,
            ios_runner: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
            browserstack: None,
            ios_xcuitest: None,
        };
        let remote = match manifest.platform {
            MobileTarget::Android => {
                trigger_browserstack_espresso(&spec, &entry.app, &entry.test_suite)?
            }
            MobileTarget::Ios => trigger_browserstack_xcuitest(
                &spec,
                &IosXcuitestArtifacts {
                    app: entry.app,
                    test_suite: entry.test_suite,
                },
            )?,
        };
        let build_id = match &remote {
            RemoteRun::Android { build_id, .. } | RemoteRun::Ios { build_id, .. } => build_id,
        };
        let platform = match manifest.platform {
            MobileTarget::Android => "espresso",
            MobileTarget::Ios => "xcuitest",
        };
        let (results, metrics) = client.wait_and_fetch_all_results_with_poll(
            build_id,
            platform,
            Some(timeout_secs),
            Some(args.fetch_poll_interval_secs),
        )?;
        if args.fetch {
            fetch_browserstack_artifacts(
                &client,
                manifest.platform,
                build_id,
                &args.fetch_output_dir.join(build_id),
                false,
                args.fetch_poll_interval_secs,
                args.fetch_timeout_secs,
            )?;
        }
        let validated = validate_prebuilt_results(
            &entry.function,
            entry.iterations,
            entry.warmup,
            &args.devices,
            &results,
        )?;
        let metrics = metrics
            .into_iter()
            .map(|(device, metrics)| {
                let canonical = validated
                    .device_identities
                    .get(&device)
                    .cloned()
                    .unwrap_or(device);
                (canonical, metrics)
            })
            .collect::<BTreeMap<_, _>>();
        let mut run_summary = RunSummary {
            spec,
            artifacts: None,
            local_report: json!({"skipped": true, "reason": "prebuilt BrowserStack run"}),
            remote_run: Some(remote),
            summary: SummaryReport {
                generated_at: String::new(),
                generated_at_unix: 0,
                target: manifest.platform,
                function: entry.function.clone(),
                iterations: entry.iterations,
                warmup: entry.warmup,
                devices: args.devices.clone(),
                device_summaries: Vec::new(),
            },
            benchmark_results: Some(validated.results.into_iter().collect()),
            benchmark_failures: None,
            performance_metrics: Some(metrics),
        };
        run_summary.summary = build_summary(&run_summary)?;
        summaries.push(run_summary.summary.clone());
        function_values.insert(entry.function, serde_json::to_value(&run_summary)?);
    }
    let merged = merge_summary_reports(manifest.platform, &summaries)?;
    let root_value = json!({
        "summary": merged,
        "functions": function_values,
        "ci": {
            "metadata": {
                "request_command": "cargo mobench ci run-prebuilt",
                "source_sha": manifest.source_sha,
                "mobench_version": env!("CARGO_PKG_VERSION")
            },
            "outputs": {
                "summary_json": "summary.json",
                "summary_md": "summary.md",
                "results_csv": "results.csv"
            }
        }
    });
    write_file(
        &args.output_dir.join("summary.json"),
        serde_json::to_string_pretty(&root_value)?.as_bytes(),
    )?;
    write_file(
        &args.output_dir.join("summary.md"),
        render_markdown_summary(&merged).as_bytes(),
    )?;
    write_file(
        &args.output_dir.join("results.csv"),
        render_csv_summary(&merged).as_bytes(),
    )?;
    println!("Prebuilt CI outputs ready at {}", args.output_dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_bundle(root: &Path, mutate: impl FnOnce(&mut PrebuiltManifest)) -> PathBuf {
        let app_path = root.join("entries/0000/app.apk");
        let test_path = root.join("entries/0000/test.apk");
        fs::create_dir_all(app_path.parent().unwrap()).unwrap();
        fs::write(&app_path, b"PK\x03\x04app").unwrap();
        fs::write(&test_path, b"PK\x03\x04test").unwrap();
        let artifact = |kind, path: &Path, relative: &str| {
            let (size, sha256) = sha256_file(path).unwrap();
            PrebuiltArtifact {
                kind,
                path: relative.to_string(),
                size,
                sha256,
            }
        };
        let mut manifest = PrebuiltManifest {
            schema: PREBUILT_SCHEMA.to_string(),
            source_sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
            platform: MobileTarget::Android,
            build_profile: "release".to_string(),
            mobench_version: env!("CARGO_PKG_VERSION").to_string(),
            abi: PrebuiltAbi {
                benchmark: PREBUILT_ABI.to_string(),
                runner: "browserstack-espresso-v2".to_string(),
            },
            entries: vec![PrebuiltEntry {
                function: "sample::bench".to_string(),
                iterations: 2,
                warmup: 1,
                completion_timeout_secs: Some(300),
                artifacts: vec![
                    artifact(
                        PrebuiltArtifactKind::AndroidApp,
                        &app_path,
                        "entries/0000/app.apk",
                    ),
                    artifact(
                        PrebuiltArtifactKind::AndroidTestSuite,
                        &test_path,
                        "entries/0000/test.apk",
                    ),
                ],
            }],
        };
        mutate(&mut manifest);
        let manifest_path = root.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        manifest_path
    }

    #[test]
    fn manifest_accepts_exact_hashed_pair() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = write_test_bundle(temp.path(), |_| {});
        let (_, entries) =
            verify_prebuilt_manifest(&manifest, "0123456789abcdef0123456789abcdef01234567")
                .unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn manifest_rejects_hash_traversal_and_extra_files() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = write_test_bundle(temp.path(), |payload| {
            payload.entries[0].artifacts[0].sha256 = "0".repeat(64);
        });
        assert!(
            verify_prebuilt_manifest(&manifest, "0123456789abcdef0123456789abcdef01234567")
                .is_err()
        );

        let temp = tempfile::tempdir().unwrap();
        let manifest = write_test_bundle(temp.path(), |payload| {
            payload.entries[0].artifacts[0].path = "../app.apk".to_string();
        });
        assert!(
            verify_prebuilt_manifest(&manifest, "0123456789abcdef0123456789abcdef01234567")
                .is_err()
        );

        let temp = tempfile::tempdir().unwrap();
        let manifest = write_test_bundle(temp.path(), |_| {});
        fs::write(temp.path().join("extra"), b"surprise").unwrap();
        assert!(
            verify_prebuilt_manifest(&manifest, "0123456789abcdef0123456789abcdef01234567")
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn manifest_rejects_symlink_artifacts() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let manifest = write_test_bundle(temp.path(), |_| {});
        let app = temp.path().join("entries/0000/app.apk");
        fs::remove_file(&app).unwrap();
        symlink(temp.path().join("entries/0000/test.apk"), app).unwrap();
        assert!(
            verify_prebuilt_manifest(&manifest, "0123456789abcdef0123456789abcdef01234567")
                .is_err()
        );
    }

    #[test]
    fn trusted_timeout_is_bounded() {
        assert_eq!(trusted_prebuilt_timeout(Some(300), 300, 1800), 360);
        assert_eq!(trusted_prebuilt_timeout(Some(21_600), 300, 1800), 1800);
    }

    #[test]
    fn prebuilt_results_require_strict_v2_identity_and_counts() {
        let identifier =
            |value: &str| mobench_domain::ReportIdentifier::parse(value.to_string()).unwrap();
        let report = mobench_domain::ReportEnvelopeV2::new(
            mobench_domain::ReportIdentity::new(
                identifier("run-1"),
                identifier("nonce-1"),
                identifier("logical-1"),
                identifier("sample::bench"),
                identifier("android-runner"),
            ),
            mobench_domain::ReportCounts::new(2, 1).unwrap(),
            mobench_domain::ReportCounts::observed(2, 1).unwrap(),
            vec![10, 20],
            mobench_domain::ReportOutcome::Success,
        )
        .unwrap();
        let value = serde_json::to_value(report).unwrap();

        validate_prebuilt_v2_envelope(&value, "sample::bench", 2, 1).unwrap();
        assert!(validate_prebuilt_v2_envelope(&value, "other::bench", 2, 1).is_err());
        assert!(validate_prebuilt_v2_envelope(&value, "sample::bench", 3, 1).is_err());
        assert!(
            validate_prebuilt_v2_envelope(
                &json!({"function": "sample::bench"}),
                "sample::bench",
                2,
                1
            )
            .is_err()
        );
    }
}
