//! Build execution, provider launch, and mobile-spec embedding.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use mobench_runtime::Distribution;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;
use serde_json::json;

use crate::browserstack::{
    self, BrowserStackArtifacts, BrowserStackAuth, BrowserStackClient, BrowserStackProviderAdapter,
    BrowserStackRunRequest, DEFAULT_BROWSERSTACK_FETCH_TIMEOUT_SECS,
};
use crate::project_layout::*;
use crate::report_binding::RunEnvelopeIdentity;
use crate::reporting::extract_samples;
use crate::workspace_fs::{repo_root, write_file};
use crate::{
    BrowserStackConfig, IosXcuitestArtifacts, MobileTarget, RemoteRun, ResolvedProjectLayout,
    RunSpec,
};

pub(crate) fn run_ios_build(
    layout: &ResolvedProjectLayout,
    release: bool,
    dry_run: bool,
    ios_completion_timeout_secs: Option<u64>,
    ios_deployment_target: Option<&str>,
    ios_runner: Option<&str>,
) -> Result<(PathBuf, PathBuf)> {
    let ios_completion_timeout_secs =
        configured_ios_completion_timeout_secs(layout, ios_completion_timeout_secs);
    let ios_deployment_target = configured_ios_deployment_target(layout, ios_deployment_target)?;
    let ios_runner = configured_ios_runner(layout, &ios_deployment_target, ios_runner)?;
    let builder = ios_builder_for_layout(layout)
        .verbose(true)
        .dry_run(dry_run)
        .deployment_target(ios_deployment_target)
        .runner(Some(ios_runner))
        .benchmark_timeout_secs(ios_completion_timeout_secs)
        .crate_dir(&layout.crate_dir)
        .output_dir(&layout.output_dir);
    let profile = if release {
        mobench_sdk::BuildProfile::Release
    } else {
        mobench_sdk::BuildProfile::Debug
    };
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Ios,
        profile,
        incremental: true,
        android_abis: None,
    };
    if let Some(timeout_secs) = ios_completion_timeout_secs {
        println!("Using iOS benchmark completion timeout: {timeout_secs}s");
    }
    let result = builder.build(&cfg)?;
    let header = layout
        .output_dir
        .join("ios/include")
        .join(format!("{}.h", layout.library_name));
    Ok((result.app_path, header))
}

pub(crate) fn package_ios_xcuitest_artifacts(
    layout: &ResolvedProjectLayout,
    spec: &RunSpec,
    identity: &RunEnvelopeIdentity,
    release: bool,
    ios_completion_timeout_secs: Option<u64>,
    ios_deployment_target: Option<&str>,
    ios_runner: Option<&str>,
) -> Result<IosXcuitestArtifacts> {
    let ios_completion_timeout_secs =
        configured_ios_completion_timeout_secs(layout, ios_completion_timeout_secs);
    let ios_deployment_target = configured_ios_deployment_target(layout, ios_deployment_target)?;
    let ios_runner = configured_ios_runner(layout, &ios_deployment_target, ios_runner)?;
    let builder = ios_builder_for_layout(layout)
        .verbose(true)
        .deployment_target(ios_deployment_target)
        .runner(Some(ios_runner))
        .benchmark_timeout_secs(ios_completion_timeout_secs)
        .crate_dir(&layout.crate_dir)
        .output_dir(&layout.output_dir);
    let profile = if release {
        mobench_sdk::BuildProfile::Release
    } else {
        mobench_sdk::BuildProfile::Debug
    };
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Ios,
        profile,
        incremental: true,
        android_abis: None,
    };
    builder
        .build(&cfg)
        .context("Failed to build iOS xcframework before packaging")?;
    // `build()` refreshes generated XCUITest sources from the crate's detected
    // default. Re-embed after generation so the compiled test suite is bound to
    // the function requested for this run, not a stale scaffolding default.
    embed_spec_into_apps(&layout.output_dir, spec, identity)
        .context("Failed to bind generated iOS artifacts to the current bench spec")?;
    builder
        .regenerate_xcode_project()
        .context("Failed to regenerate iOS Xcode project after embedding bench spec")?;
    let app = builder
        .package_ipa("BenchRunner", mobench_sdk::builders::SigningMethod::AdHoc)
        .context("Failed to package iOS IPA for BrowserStack")?;
    let test_suite = builder
        .package_xcuitest("BenchRunner")
        .context("Failed to package iOS XCUITest runner for BrowserStack")?;
    Ok(IosXcuitestArtifacts { app, test_suite })
}

pub(crate) fn default_ios_xcuitest_artifacts(
    layout: &ResolvedProjectLayout,
) -> IosXcuitestArtifacts {
    IosXcuitestArtifacts {
        app: layout.output_dir.join("ios/BenchRunner.ipa"),
        test_suite: layout.output_dir.join("ios/BenchRunnerUITests.zip"),
    }
}

pub(crate) fn legacy_ios_xcuitest_artifacts(
    layout: &ResolvedProjectLayout,
) -> IosXcuitestArtifacts {
    IosXcuitestArtifacts {
        app: layout.project_root.join("target/ios/BenchRunner.ipa"),
        test_suite: layout
            .project_root
            .join("target/ios/BenchRunnerUITests.zip"),
    }
}

pub(crate) fn resolve_project_relative_path(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

pub(crate) fn uses_managed_ios_xcuitest_artifacts(
    layout: &ResolvedProjectLayout,
    artifacts: &IosXcuitestArtifacts,
) -> bool {
    let app = resolve_project_relative_path(&layout.project_root, &artifacts.app);
    let test_suite = resolve_project_relative_path(&layout.project_root, &artifacts.test_suite);

    [
        default_ios_xcuitest_artifacts(layout),
        legacy_ios_xcuitest_artifacts(layout),
    ]
    .into_iter()
    .any(|managed| app == managed.app && test_suite == managed.test_suite)
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedBrowserStack {
    pub(crate) username: String,
    pub(crate) access_key: String,
    pub(crate) project: Option<String>,
}

/// Represents artifacts validation error details for BrowserStack uploads.
#[derive(Debug)]
struct ArtifactValidationError {
    missing_artifacts: Vec<(String, PathBuf)>,
    target: MobileTarget,
}

impl ArtifactValidationError {
    fn format_error(&self) -> String {
        let mut msg = String::from("Missing required artifacts for BrowserStack run:\n\n");

        for (name, path) in &self.missing_artifacts {
            msg.push_str(&format!("  x {} not found at: {}\n", name, path.display()));
        }

        msg.push('\n');
        msg.push_str("To fix, run:\n");
        match self.target {
            MobileTarget::Android => {
                msg.push_str("  cargo mobench build --target android\n");
            }
            MobileTarget::Ios => {
                msg.push_str("  cargo mobench build --target ios\n");
                msg.push_str("  cargo mobench package-ipa --method adhoc\n");
                msg.push_str("  cargo mobench package-xcuitest\n");
            }
        }

        msg
    }
}

/// Validates that all required artifacts exist before attempting a BrowserStack upload.
///
/// This function checks for the presence of required files early to provide clear
/// error messages before starting any uploads.
///
/// # Arguments
/// * `target` - The target platform (Android or iOS)
/// * `apk` - For Android: path to the app APK
/// * `test_apk` - For Android: path to the test APK
/// * `ios_artifacts` - For iOS: the app and test suite paths
///
/// # Returns
/// * `Ok(())` if all artifacts exist
/// * `Err` with detailed message about missing artifacts and how to fix
pub(crate) fn validate_artifacts_for_browserstack(
    target: MobileTarget,
    apk: Option<&Path>,
    test_apk: Option<&Path>,
    ios_artifacts: Option<&IosXcuitestArtifacts>,
) -> Result<()> {
    let mut missing = Vec::new();

    match target {
        MobileTarget::Android => {
            if let Some(apk_path) = apk
                && !apk_path.exists()
            {
                missing.push(("Android APK".to_string(), apk_path.to_path_buf()));
            }
            if let Some(test_apk_path) = test_apk
                && !test_apk_path.exists()
            {
                missing.push(("Android test APK".to_string(), test_apk_path.to_path_buf()));
            }
        }
        MobileTarget::Ios => {
            if let Some(artifacts) = ios_artifacts {
                if !artifacts.app.exists() {
                    missing.push(("iOS app IPA".to_string(), artifacts.app.clone()));
                }
                if !artifacts.test_suite.exists() {
                    missing.push((
                        "iOS XCUITest runner".to_string(),
                        artifacts.test_suite.clone(),
                    ));
                }
            }
        }
    }

    if !missing.is_empty() {
        let error = ArtifactValidationError {
            missing_artifacts: missing,
            target,
        };
        bail!("{}", error.format_error());
    }

    Ok(())
}

/// Extracted benchmark result for a single device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedBenchmarkResult {
    /// Device name.
    pub device: String,
    /// Benchmark function name.
    pub function: String,
    /// Mean execution time in nanoseconds.
    pub mean_ns: u64,
    /// Number of samples collected.
    pub sample_count: usize,
    /// Standard deviation in nanoseconds (if calculable).
    pub std_dev_ns: Option<u64>,
    /// Minimum sample value in nanoseconds.
    pub min_ns: Option<u64>,
    /// Maximum sample value in nanoseconds.
    pub max_ns: Option<u64>,
}

/// Extract a unified summary from per-device benchmark results.
///
/// This function takes the raw benchmark results from BrowserStack and produces
/// a unified summary that's easier to work with programmatically.
pub fn extract_benchmark_summary(
    results: &HashMap<String, Vec<serde_json::Value>>,
) -> Vec<ExtractedBenchmarkResult> {
    let mut extracted = Vec::new();

    for (device, benchmarks) in results {
        for benchmark in benchmarks {
            let function = benchmark
                .get("function")
                .and_then(|f| f.as_str())
                .unwrap_or("unknown")
                .to_string();

            let producer_mean_ns = benchmark
                .get("mean_ns")
                .and_then(|m| m.as_u64())
                .unwrap_or(0);

            let samples = extract_samples(benchmark);

            let sample_count = samples.len();
            let statistics = Distribution::from_slice(&samples).sdk_v1_summary();
            let mean_ns = if samples.is_empty() {
                producer_mean_ns
            } else {
                statistics.mean_ns as u64
            };
            let min_ns = (!samples.is_empty()).then_some(statistics.min_ns);
            let max_ns = (!samples.is_empty()).then_some(statistics.max_ns);
            let std_dev_ns = (sample_count > 1).then_some(statistics.std_dev_ns as u64);

            extracted.push(ExtractedBenchmarkResult {
                device: device.clone(),
                function,
                mean_ns,
                sample_count,
                std_dev_ns,
                min_ns,
                max_ns,
            });
        }
    }

    extracted
}

pub(crate) fn trigger_browserstack_espresso(
    spec: &RunSpec,
    apk: &Path,
    test_apk: &Path,
) -> Result<RemoteRun> {
    // Validate artifacts exist before attempting upload
    validate_artifacts_for_browserstack(MobileTarget::Android, Some(apk), Some(test_apk), None)?;

    let creds = resolve_browserstack_credentials(spec.browserstack.as_ref())?;
    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: creds.username.clone(),
            access_key: creds.access_key.clone(),
        },
        creds.project.clone(),
    )?;

    let engine = mobench_provider::ProviderEngine::new(BrowserStackProviderAdapter::new(
        client,
        DEFAULT_BROWSERSTACK_FETCH_TIMEOUT_SECS,
        5,
    ));
    let request = BrowserStackRunRequest {
        devices: spec.devices.clone(),
        artifacts: BrowserStackArtifacts::Espresso {
            app: apk.to_path_buf(),
            test_suite: test_apk.to_path_buf(),
        },
    };
    let run = engine
        .start(&request, &mobench_process::global_cancellation_token())
        .map_err(|error| anyhow!("BrowserStack provider failed to start: {error}"))?
        .into_handle();

    // Print dashboard link early so users can monitor progress
    println!();
    println!("BrowserStack build started!");
    println!("  Build ID: {}", run.build_id);
    println!("  Devices:  {}", spec.devices.join(", "));
    println!(
        "  Dashboard: https://app-automate.browserstack.com/dashboard/v2/builds/{}",
        run.build_id
    );
    println!();
    println!("Waiting for results...");

    Ok(RemoteRun::Android {
        app_url: run.app_url,
        build_id: run.build_id,
    })
}

pub(crate) fn trigger_browserstack_xcuitest(
    spec: &RunSpec,
    artifacts: &IosXcuitestArtifacts,
) -> Result<RemoteRun> {
    // Validate artifacts exist before attempting upload
    validate_artifacts_for_browserstack(MobileTarget::Ios, None, None, Some(artifacts))?;

    let creds = resolve_browserstack_credentials(spec.browserstack.as_ref())?;
    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: creds.username.clone(),
            access_key: creds.access_key.clone(),
        },
        creds.project.clone(),
    )?;

    let engine = mobench_provider::ProviderEngine::new(BrowserStackProviderAdapter::new(
        client,
        DEFAULT_BROWSERSTACK_FETCH_TIMEOUT_SECS,
        5,
    ));
    let request = BrowserStackRunRequest {
        devices: spec.devices.clone(),
        artifacts: BrowserStackArtifacts::XcuiTest {
            app: artifacts.app.clone(),
            test_suite: artifacts.test_suite.clone(),
        },
    };
    let run = engine
        .start(&request, &mobench_process::global_cancellation_token())
        .map_err(|error| anyhow!("BrowserStack provider failed to start: {error}"))?
        .into_handle();

    // Print dashboard link early so users can monitor progress
    println!();
    println!("BrowserStack build started!");
    println!("  Build ID: {}", run.build_id);
    println!("  Devices:  {}", spec.devices.join(", "));
    println!(
        "  Dashboard: https://app-automate.browserstack.com/dashboard/v2/builds/{}",
        run.build_id
    );
    println!();
    println!("Waiting for results...");

    Ok(RemoteRun::Ios {
        app_url: run.app_url,
        test_suite_url: run
            .test_suite_url
            .context("BrowserStack XCUITest start omitted the test-suite URL")?,
        build_id: run.build_id,
    })
}

pub(crate) fn resolve_browserstack_credentials(
    config: Option<&BrowserStackConfig>,
) -> Result<ResolvedBrowserStack> {
    let mut username = None;
    let mut access_key = None;
    let mut project = None;

    if let Some(cfg) = config {
        username = Some(expand_env_var(&cfg.app_automate_username)?);
        access_key = Some(expand_env_var(&cfg.app_automate_access_key)?);
        project = cfg
            .project
            .as_ref()
            .map(|p| expand_env_var(p))
            .transpose()?;
    }

    if username.as_deref().map(str::is_empty).unwrap_or(true)
        && let Ok(val) = env::var("BROWSERSTACK_USERNAME")
        && !val.is_empty()
    {
        username = Some(val);
    }
    if access_key.as_deref().map(str::is_empty).unwrap_or(true)
        && let Ok(val) = env::var("BROWSERSTACK_ACCESS_KEY")
        && !val.is_empty()
    {
        access_key = Some(val);
    }
    if project.is_none()
        && let Ok(val) = env::var("BROWSERSTACK_PROJECT")
        && !val.is_empty()
    {
        project = Some(val);
    }

    // Check what's missing and provide helpful error message
    let missing_username = username.as_deref().map(str::is_empty).unwrap_or(true);
    let missing_access_key = access_key.as_deref().map(str::is_empty).unwrap_or(true);

    if missing_username || missing_access_key {
        let error_msg =
            browserstack::format_credentials_error(missing_username, missing_access_key);
        bail!("{}", error_msg);
    }

    Ok(ResolvedBrowserStack {
        username: username.context("BrowserStack username resolved to None")?,
        access_key: access_key.context("BrowserStack access key resolved to None")?,
        project,
    })
}

pub(crate) fn expand_env_var(raw: &str) -> Result<String> {
    if let Some(stripped) = raw.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        let val = env::var(stripped)
            .with_context(|| format!("resolving env var {stripped} for BrowserStack config"))?;
        return Ok(val);
    }
    Ok(raw.to_string())
}

#[cfg(test)]
pub(crate) fn run_local_smoke(spec: &RunSpec) -> Result<Value> {
    println!("Running local smoke test for {}...", spec.function);

    let bench_spec = mobench_sdk::BenchSpec {
        name: spec.function.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
    };

    let report =
        mobench_sdk::run_benchmark(bench_spec).map_err(|e| anyhow!("benchmark failed: {e}"))?;

    serde_json::to_value(&report).context("serializing benchmark report")
}

/// Validates that the benchmark function exists in the crate source.
///
/// This provides early feedback when a function name is misspelled or doesn't exist.
/// If validation fails, it warns but continues (the final validation happens on device).
pub(crate) fn validate_benchmark_function(
    layout: &ResolvedProjectLayout,
    function_name: &str,
) -> Result<()> {
    let benchmarks = discover_benchmarks_for_layout(layout)?;
    let found_any_benchmarks = !benchmarks.is_empty();
    let simple_name = function_name.split("::").last().unwrap_or(function_name);
    let found_function = benchmarks
        .iter()
        .any(|benchmark| benchmark == function_name)
        || benchmarks
            .iter()
            .any(|benchmark| benchmark.ends_with(&format!("::{}", simple_name)));

    if found_any_benchmarks && !found_function {
        // We found benchmarks but not the one requested - this is likely an error
        println!("=== Warning ===");
        println!(
            "  Benchmark function '{}' was not found in the source code.",
            function_name
        );
        println!("  Available benchmarks:");
        for bench in benchmarks {
            println!("    - {}", bench);
        }
        println!();
        println!("  The run will continue, but the benchmark may fail on the device.");
        println!("  Tip: Use 'cargo mobench list' to see all available benchmarks.");
        println!();
    } else if !found_any_benchmarks {
        // No benchmarks found at all - might be using direct dispatch
        println!("=== Note ===");
        println!(
            "  Could not validate benchmark function '{}' (no #[benchmark] functions found).",
            function_name
        );
        println!("  This is normal for projects using direct FFI dispatch (like sample-fns).");
        println!();
    } else {
        // Function validated successfully
        println!("Benchmark function '{}' validated.", function_name);
    }

    Ok(())
}

pub(crate) fn persist_mobile_spec(
    layout: &ResolvedProjectLayout,
    spec: &RunSpec,
    identity: &RunEnvelopeIdentity,
    release: bool,
) -> Result<()> {
    let root = &layout.project_root;
    let payload = json!({
        "schema_version": mobench_domain::REPORT_SCHEMA_V2,
        "run_id": identity.run_id,
        "nonce": identity.nonce,
        "logical_session_id": identity.logical_session_id,
        "function": spec.function,
        "function_id": spec.function,
        "producer": identity.producer,
        "iterations": spec.iterations,
        "warmup": spec.warmup,
        "android_benchmark_timeout_secs": spec.android_benchmark_timeout_secs,
        "android_heartbeat_interval_secs": spec.android_heartbeat_interval_secs,
    });
    let contents = serde_json::to_string_pretty(&payload)?;

    // Write to legacy mobile-spec locations for backward compatibility
    let legacy_targets = [
        root.join("target/mobile-spec/android/bench_spec.json"),
        root.join("target/mobile-spec/ios/bench_spec.json"),
    ];
    for path in legacy_targets {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {:?}", parent))?;
        }
        write_file(&path, contents.as_bytes())?;
    }

    // IMPORTANT: Also embed the spec directly into the mobile app bundles
    // This ensures the requested benchmark function is always used, even when
    // the app is run via BrowserStack where file paths are different.
    let mobench_output_dir = layout.output_dir.clone();
    let apps_exist =
        mobench_output_dir.join("android").exists() || mobench_output_dir.join("ios").exists();

    if let Err(e) = embed_spec_into_apps(&mobench_output_dir, spec, identity) {
        // Only warn if the apps don't exist yet - they'll be created during build
        if apps_exist {
            println!(
                "Warning: Failed to embed bench spec into app bundles: {}",
                e
            );
        }
    } else if apps_exist {
        println!("Embedded bench_spec.json in mobile app bundles");
    }

    // B3: Embed build metadata (bench_meta.json) for artifact correlation
    let profile = if release { "release" } else { "debug" };
    let target_str = match spec.target {
        MobileTarget::Android => "android",
        MobileTarget::Ios => "ios",
    };

    if let Err(e) = embed_meta_into_apps(&mobench_output_dir, spec, target_str, profile) {
        if apps_exist {
            println!(
                "Warning: Failed to embed bench meta into app bundles: {}",
                e
            );
        }
    } else if apps_exist {
        println!("Embedded bench_meta.json with build metadata");
    }

    Ok(())
}

/// Embeds the benchmark spec into Android assets and iOS bundle resources.
pub(crate) fn embed_spec_into_apps(
    output_dir: &Path,
    spec: &RunSpec,
    identity: &RunEnvelopeIdentity,
) -> Result<()> {
    #[derive(Serialize)]
    struct EmbeddedRunSpec {
        schema_version: &'static str,
        run_id: String,
        nonce: String,
        logical_session_id: String,
        function: String,
        function_id: String,
        producer: String,
        iterations: u32,
        warmup: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        android_benchmark_timeout_secs: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        android_heartbeat_interval_secs: Option<u64>,
    }

    let embedded_spec = EmbeddedRunSpec {
        schema_version: mobench_domain::REPORT_SCHEMA_V2,
        run_id: identity.run_id.clone(),
        nonce: identity.nonce.clone(),
        logical_session_id: identity.logical_session_id.clone(),
        function: spec.function.clone(),
        function_id: spec.function.clone(),
        producer: identity.producer.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
        android_benchmark_timeout_secs: spec.android_benchmark_timeout_secs,
        android_heartbeat_interval_secs: spec.android_heartbeat_interval_secs,
    };
    mobench_sdk::builders::embed_bench_spec(output_dir, &embedded_spec)
        .map_err(|e| anyhow!("Failed to embed bench spec: {}", e))
}

/// Embeds build metadata (bench_meta.json) into Android assets and iOS bundle resources.
pub(crate) fn embed_meta_into_apps(
    output_dir: &Path,
    spec: &RunSpec,
    target: &str,
    profile: &str,
) -> Result<()> {
    let embedded_spec = mobench_sdk::builders::EmbeddedBenchSpec {
        function: spec.function.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
    };
    mobench_sdk::builders::embed_bench_meta(output_dir, &embedded_spec, target, profile)
        .map_err(|e| anyhow!("Failed to embed bench meta: {}", e))
}

pub(crate) fn run_android_build(
    layout: &ResolvedProjectLayout,
    _ndk_home: &str,
    release: bool,
    dry_run: bool,
) -> Result<mobench_sdk::BuildResult> {
    ensure_android_home();
    let profile = if release {
        mobench_sdk::BuildProfile::Release
    } else {
        mobench_sdk::BuildProfile::Debug
    };
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Android,
        profile,
        incremental: true,
        android_abis: layout.android_abis.clone(),
    };
    let builder = android_builder_for_layout(layout)
        .verbose(true)
        .dry_run(dry_run)
        .crate_dir(&layout.crate_dir)
        .output_dir(&layout.output_dir);
    let result = builder.build(&cfg)?;
    Ok(result)
}

/// Ensure ANDROID_HOME is set, inferring it from ANDROID_NDK_HOME if necessary.
///
/// Gradle requires ANDROID_HOME to locate the SDK. Many developers only set
/// ANDROID_NDK_HOME (which is `$ANDROID_HOME/ndk/<version>`). This function
/// strips the `/ndk/<version>` suffix to derive ANDROID_HOME when it is missing.
pub(crate) fn ensure_android_home() {
    if std::env::var("ANDROID_HOME").is_ok() {
        return;
    }
    if let Ok(ndk_home) = std::env::var("ANDROID_NDK_HOME") {
        // ANDROID_NDK_HOME is typically $ANDROID_HOME/ndk/<version>
        let ndk_path = std::path::Path::new(&ndk_home);
        if let Some(ndk_dir) = ndk_path.parent()
            && ndk_dir.file_name().is_some_and(|n| n == "ndk")
            && let Some(sdk_root) = ndk_dir.parent()
        {
            eprintln!(
                "Inferred ANDROID_HOME={} from ANDROID_NDK_HOME",
                sdk_root.display()
            );
            // SAFETY: called early in single-threaded CLI init, before
            // any threads are spawned.
            unsafe { std::env::set_var("ANDROID_HOME", sdk_root) };
        }
    }
}

/// Load .env/.env.local from the repo root (best-effort, for commands that don't resolve a layout).
pub(crate) fn load_dotenv_global() {
    if let Ok(root) = repo_root() {
        let _ = dotenvy::from_path(root.join(".env"));
        let _ = dotenvy::from_path(root.join(".env.local"));
    }
}

pub(crate) fn load_dotenv_for_layout(layout: &ResolvedProjectLayout) {
    let mut directories = vec![layout.project_root.clone()];
    if let Some(config_path) = &layout.config_path
        && let Some(config_dir) = config_path.parent()
        && config_dir != layout.project_root
    {
        directories.push(config_dir.to_path_buf());
    }

    for dir in directories {
        let _ = dotenvy::from_path(dir.join(".env"));
        let _ = dotenvy::from_path(dir.join(".env.local"));
    }
}
