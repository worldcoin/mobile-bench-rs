use anyhow::{Context, Result, anyhow, bail};
use bench_runner::{BenchSpec, run_closure};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use browserstack::{BrowserStackAuth, BrowserStackClient};

mod browserstack;

/// CLI orchestrator for building, packaging, and executing Rust benchmarks on mobile.
#[derive(Parser, Debug)]
#[command(name = "bench-cli", author, version, about = "Mobile Rust benchmarking orchestrator", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run a benchmark against a target platform (mobile integration stub for now).
    Run {
        #[arg(long, value_enum)]
        target: MobileTarget,
        #[arg(long, help = "Fully-qualified Rust function to benchmark")]
        function: String,
        #[arg(long, default_value_t = 100)]
        iterations: u32,
        #[arg(long, default_value_t = 10)]
        warmup: u32,
        #[arg(long, help = "Device identifiers or labels (BrowserStack devices)")]
        devices: Vec<String>,
        #[arg(long, help = "Optional path to config file")]
        config: Option<PathBuf>,
        #[arg(long, help = "Optional output path for JSON report")]
        output: Option<PathBuf>,
        #[arg(long, help = "Skip mobile builds and only run the host harness")]
        local_only: bool,
        #[arg(
            long,
            help = "Path to iOS app bundle (.ipa or zipped .app) for BrowserStack XCUITest"
        )]
        ios_app: Option<PathBuf>,
        #[arg(long, help = "Path to iOS XCUITest test suite package (.zip or .ipa)")]
        ios_test_suite: Option<PathBuf>,
    },
    /// Run a local demo against bundled sample functions to validate the harness.
    Demo {
        #[arg(long, default_value_t = 50)]
        iterations: u32,
        #[arg(long, default_value_t = 5)]
        warmup: u32,
    },
    /// Scaffold a base config file for the CLI.
    Init {
        #[arg(long, default_value = "bench-config.toml")]
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = MobileTarget::Android)]
        target: MobileTarget,
    },
    /// Generate a sample device matrix file.
    Plan {
        #[arg(long, default_value = "device-matrix.yaml")]
        output: PathBuf,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MobileTarget {
    Android,
    Ios,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BrowserStackConfig {
    app_automate_username: String,
    app_automate_access_key: String,
    project: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct IosXcuitestArtifacts {
    app: PathBuf,
    test_suite: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct BenchConfig {
    target: MobileTarget,
    function: String,
    iterations: u32,
    warmup: u32,
    device_matrix: PathBuf,
    browserstack: BrowserStackConfig,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    ios_xcuitest: Option<IosXcuitestArtifacts>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct DeviceEntry {
    name: String,
    os: String,
    os_version: String,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeviceMatrix {
    devices: Vec<DeviceEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct RunSpec {
    target: MobileTarget,
    function: String,
    iterations: u32,
    warmup: u32,
    devices: Vec<String>,
    #[serde(skip_serializing, skip_deserializing, default)]
    browserstack: Option<BrowserStackConfig>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    ios_xcuitest: Option<IosXcuitestArtifacts>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "platform", rename_all = "lowercase")]
enum MobileArtifacts {
    Android {
        apk: PathBuf,
    },
    Ios {
        xcframework: PathBuf,
        header: PathBuf,
        #[serde(skip_serializing_if = "Option::is_none")]
        app: Option<PathBuf>,
        #[serde(skip_serializing_if = "Option::is_none")]
        test_suite: Option<PathBuf>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct RunSummary {
    spec: RunSpec,
    artifacts: Option<MobileArtifacts>,
    local_report: Value,
    remote_run: Option<RemoteRun>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "platform", rename_all = "lowercase")]
enum RemoteRun {
    Android {
        app_url: String,
        build_id: String,
    },
    Ios {
        app_url: String,
        test_suite_url: String,
        build_id: String,
    },
}

fn main() -> Result<()> {
    load_dotenv();
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            target,
            function,
            iterations,
            warmup,
            devices,
            config,
            output,
            local_only,
            ios_app,
            ios_test_suite,
        } => {
            let spec = resolve_run_spec(
                target,
                function,
                iterations,
                warmup,
                devices,
                config.as_deref(),
                ios_app,
                ios_test_suite,
            )?;
            println!(
                "Preparing benchmark run for {:?}: {} (iterations={}, warmup={})",
                spec.target, spec.function, spec.iterations, spec.warmup
            );
            persist_mobile_spec(&spec)?;
            if !spec.devices.is_empty() {
                println!("Devices: {}", spec.devices.join(", "));
            }
            if let Some(path) = &output {
                println!("JSON summary will be written to {:?}", path);
            }

            let local_report = run_local_smoke(&spec)?;
            let mut remote_run = None;
            let artifacts = if local_only {
                println!("Skipping mobile build: --local-only set");
                None
            } else {
                match spec.target {
                    MobileTarget::Android => {
                        let ndk = std::env::var("ANDROID_NDK_HOME")
                            .context("ANDROID_NDK_HOME must be set for Android builds")?;
                        let apk = run_android_build(&ndk)?;
                        println!("Built Android APK at {:?}", apk);
                        if spec.devices.is_empty() {
                            println!("Skipping BrowserStack upload/run: no devices provided");
                            Some(MobileArtifacts::Android { apk })
                        } else {
                            let run = trigger_browserstack_espresso(&spec, &apk)?;
                            remote_run = Some(run);
                            Some(MobileArtifacts::Android { apk })
                        }
                    }
                    MobileTarget::Ios => {
                        let (xcframework, header) = run_ios_build()?;
                        println!("Built iOS xcframework at {:?}", xcframework);
                        let ios_xcuitest = spec.ios_xcuitest.clone();

                        if spec.devices.is_empty() {
                            println!("Skipping BrowserStack upload/run: no devices provided");
                        } else {
                            let xcui = spec.ios_xcuitest.as_ref().context(
                                "iOS XCUITest artifacts required when targeting BrowserStack devices; provide --ios-app and --ios-test-suite or set ios_xcuitest in the config",
                            )?;
                            let run = trigger_browserstack_xcuitest(&spec, xcui)?;
                            remote_run = Some(run);
                        }

                        Some(MobileArtifacts::Ios {
                            xcframework,
                            header,
                            app: ios_xcuitest.as_ref().map(|a| a.app.clone()),
                            test_suite: ios_xcuitest.map(|a| a.test_suite),
                        })
                    }
                }
            };

            let summary = RunSummary {
                spec,
                artifacts,
                local_report,
                remote_run,
            };
            write_summary(&summary, output.as_deref())?;
        }
        Command::Demo { iterations, warmup } => {
            let spec = BenchSpec::new("sample_fns::fibonacci", iterations, warmup)?;
            let report = run_closure(spec, || {
                // This is the shape of the closure that will be invoked on-device;
                // for now we reuse it locally.
                let _ = sample_fns::fibonacci(24);
                Ok(())
            })?;

            let json = serde_json::to_string_pretty(&report)?;
            println!("{json}");
        }
        Command::Init { output, target } => {
            write_config_template(&output, target)?;
            println!("Wrote starter config to {:?}", output);
        }
        Command::Plan { output } => {
            write_device_matrix_template(&output)?;
            println!("Wrote sample device matrix to {:?}", output);
        }
    }

    Ok(())
}

fn write_config_template(path: &Path, target: MobileTarget) -> Result<()> {
    ensure_can_write(path)?;

    let ios_xcuitest = if target == MobileTarget::Ios {
        Some(IosXcuitestArtifacts {
            app: PathBuf::from("target/ios/BenchRunner.ipa"),
            test_suite: PathBuf::from("target/ios/BenchRunnerUITests.zip"),
        })
    } else {
        None
    };

    let cfg = BenchConfig {
        target,
        function: "sample_fns::fibonacci".into(),
        iterations: 100,
        warmup: 10,
        device_matrix: PathBuf::from("device-matrix.yaml"),
        browserstack: BrowserStackConfig {
            app_automate_username: "${BROWSERSTACK_USERNAME}".into(),
            app_automate_access_key: "${BROWSERSTACK_ACCESS_KEY}".into(),
            project: Some("mobile-bench-rs".into()),
        },
        ios_xcuitest,
    };

    let contents = toml::to_string_pretty(&cfg)?;
    write_file(path, contents.as_bytes())
}

fn write_device_matrix_template(path: &Path) -> Result<()> {
    ensure_can_write(path)?;

    let matrix = DeviceMatrix {
        devices: vec![
            DeviceEntry {
                name: "Pixel 7".into(),
                os: "android".into(),
                os_version: "13.0".into(),
                tags: Some(vec!["default".into(), "pixel".into()]),
            },
            DeviceEntry {
                name: "iPhone 14".into(),
                os: "ios".into(),
                os_version: "16".into(),
                tags: Some(vec!["default".into(), "iphone".into()]),
            },
        ],
    };

    let contents = serde_yaml::to_string(&matrix)?;
    write_file(path, contents.as_bytes())
}

fn resolve_run_spec(
    target: MobileTarget,
    function: String,
    iterations: u32,
    warmup: u32,
    devices: Vec<String>,
    config: Option<&Path>,
    ios_app: Option<PathBuf>,
    ios_test_suite: Option<PathBuf>,
) -> Result<RunSpec> {
    if let Some(cfg_path) = config {
        let cfg = load_config(cfg_path)?;
        let matrix = load_device_matrix(&cfg.device_matrix)?;
        let device_names = matrix.devices.into_iter().map(|d| d.name).collect();
        return Ok(RunSpec {
            target: cfg.target,
            function: cfg.function,
            iterations: cfg.iterations,
            warmup: cfg.warmup,
            devices: device_names,
            browserstack: Some(cfg.browserstack),
            ios_xcuitest: cfg.ios_xcuitest,
        });
    }

    if function.trim().is_empty() {
        bail!("function must not be empty");
    }

    let ios_xcuitest = match (ios_app, ios_test_suite) {
        (Some(app), Some(test_suite)) => Some(IosXcuitestArtifacts { app, test_suite }),
        (None, None) => None,
        _ => bail!("both --ios-app and --ios-test-suite must be provided together"),
    };

    if target == MobileTarget::Ios && !devices.is_empty() && ios_xcuitest.is_none() {
        bail!(
            "iOS BrowserStack runs require --ios-app and --ios-test-suite or an ios_xcuitest config block"
        );
    }

    Ok(RunSpec {
        target,
        function,
        iterations,
        warmup,
        devices,
        browserstack: None,
        ios_xcuitest,
    })
}

fn load_config(path: &Path) -> Result<BenchConfig> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading config {:?}", path))?;
    toml::from_str(&contents).with_context(|| format!("parsing config {:?}", path))
}

fn load_device_matrix(path: &Path) -> Result<DeviceMatrix> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading device matrix {:?}", path))?;
    serde_yaml::from_str(&contents).with_context(|| format!("parsing device matrix {:?}", path))
}

fn run_ios_build() -> Result<(PathBuf, PathBuf)> {
    let root = repo_root()?;
    run_cmd(ProcessCommand::new(root.join("scripts/build-ios.sh")).current_dir(&root))?;

    Ok((
        root.join("target/ios/sample_fns.xcframework"),
        root.join("target/ios/include/sample_fns.h"),
    ))
}

#[derive(Debug, Clone)]
struct ResolvedBrowserStack {
    username: String,
    access_key: String,
    project: Option<String>,
}

fn trigger_browserstack_espresso(spec: &RunSpec, apk: &Path) -> Result<RemoteRun> {
    let creds = resolve_browserstack_credentials(spec.browserstack.as_ref())?;
    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: creds.username.clone(),
            access_key: creds.access_key.clone(),
        },
        creds.project.clone(),
    )?;

    // Upload the app-under-test APK.
    let upload = client.upload_espresso_app(apk)?;

    // Upload the Espresso test-suite APK produced by Gradle.
    // We rely on the standard androidTest debug output path.
    let root = repo_root()?;
    let test_apk =
        root.join("android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk");
    let test_upload = client.upload_espresso_test_suite(&test_apk)?;

    // Schedule the Espresso build with both app and testSuite, as required by BrowserStack.
    let run = client.schedule_espresso_run(
        &spec.devices,
        &upload.app_url,
        &test_upload.test_suite_url,
    )?;
    println!(
        "Queued BrowserStack Espresso build {} for devices: {}",
        run.build_id,
        spec.devices.join(", ")
    );

    Ok(RemoteRun::Android {
        app_url: upload.app_url,
        build_id: run.build_id,
    })
}

fn trigger_browserstack_xcuitest(
    spec: &RunSpec,
    artifacts: &IosXcuitestArtifacts,
) -> Result<RemoteRun> {
    let creds = resolve_browserstack_credentials(spec.browserstack.as_ref())?;
    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: creds.username.clone(),
            access_key: creds.access_key.clone(),
        },
        creds.project.clone(),
    )?;

    if !artifacts.app.exists() {
        bail!(
            "iOS app artifact not found at {:?}; provide a .ipa or zipped .app",
            artifacts.app
        );
    }
    if !artifacts.test_suite.exists() {
        bail!(
            "iOS XCUITest test suite artifact not found at {:?}; provide the zipped test runner bundle",
            artifacts.test_suite
        );
    }

    let app_upload = client.upload_xcuitest_app(&artifacts.app)?;
    let test_upload = client.upload_xcuitest_test_suite(&artifacts.test_suite)?;
    let run = client.schedule_xcuitest_run(
        &spec.devices,
        &app_upload.app_url,
        &test_upload.test_suite_url,
    )?;
    println!(
        "Queued BrowserStack XCUITest build {} for devices: {}",
        run.build_id,
        spec.devices.join(", ")
    );

    Ok(RemoteRun::Ios {
        app_url: app_upload.app_url,
        test_suite_url: test_upload.test_suite_url,
        build_id: run.build_id,
    })
}

fn resolve_browserstack_credentials(
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

    if username.as_deref().map(str::is_empty).unwrap_or(true) {
        if let Ok(val) = env::var("BROWSERSTACK_USERNAME") {
            if !val.is_empty() {
                username = Some(val);
            }
        }
    }
    if access_key.as_deref().map(str::is_empty).unwrap_or(true) {
        if let Ok(val) = env::var("BROWSERSTACK_ACCESS_KEY") {
            if !val.is_empty() {
                access_key = Some(val);
            }
        }
    }
    if project.is_none() {
        if let Ok(val) = env::var("BROWSERSTACK_PROJECT") {
            if !val.is_empty() {
                project = Some(val);
            }
        }
    }

    let username = username.filter(|s| !s.is_empty()).ok_or_else(|| {
        anyhow!("BrowserStack username missing; set BROWSERSTACK_USERNAME or provide in config")
    })?;
    let access_key = access_key.filter(|s| !s.is_empty()).ok_or_else(|| {
        anyhow!("BrowserStack access key missing; set BROWSERSTACK_ACCESS_KEY or provide in config")
    })?;

    Ok(ResolvedBrowserStack {
        username,
        access_key,
        project,
    })
}

fn expand_env_var(raw: &str) -> Result<String> {
    if let Some(stripped) = raw.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        let val = env::var(stripped)
            .with_context(|| format!("resolving env var {stripped} for BrowserStack config"))?;
        return Ok(val);
    }
    Ok(raw.to_string())
}

fn run_local_smoke(spec: &RunSpec) -> Result<Value> {
    let bench_spec = sample_fns::BenchSpec {
        name: spec.function.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
    };

    let report = sample_fns::run_benchmark(bench_spec)
        .map_err(|e| anyhow!("benchmark failed: {:?}", e))?;

    serde_json::to_value(&report).context("serializing benchmark report")
}

fn persist_mobile_spec(spec: &RunSpec) -> Result<()> {
    let root = repo_root()?;
    let payload = json!({
        "function": spec.function,
        "iterations": spec.iterations,
        "warmup": spec.warmup,
    });
    let contents = serde_json::to_string_pretty(&payload)?;
    let targets = [
        root.join("target/mobile-spec/android/bench_spec.json"),
        root.join("target/mobile-spec/ios/bench_spec.json"),
    ];
    for path in targets {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {:?}", parent))?;
        }
        write_file(&path, contents.as_bytes())?;
    }
    Ok(())
}

fn write_summary(summary: &RunSummary, output: Option<&Path>) -> Result<()> {
    let json = serde_json::to_string_pretty(summary)?;
    if let Some(path) = output {
        write_file(path, json.as_bytes())?;
        println!("Wrote run summary to {:?}", path);
    } else {
        println!("{json}");
    }
    Ok(())
}

fn run_android_build(ndk_home: &str) -> Result<PathBuf> {
    let root = repo_root()?;
    run_cmd(
        ProcessCommand::new(root.join("scripts/build-android.sh"))
            .env("ANDROID_NDK_HOME", ndk_home)
            .current_dir(&root),
    )?;
    run_cmd(
        ProcessCommand::new(root.join("scripts/sync-android-libs.sh"))
            .env("ANDROID_NDK_HOME", ndk_home)
            .current_dir(&root),
    )?;
    run_cmd(
        ProcessCommand::new(root.join("android/gradlew"))
            .arg(":app:assembleDebug")
            .current_dir(root.join("android")),
    )?;

    // Also build the androidTest (Espresso) test APK so it can be uploaded as the test suite.
    run_cmd(
        ProcessCommand::new(root.join("android/gradlew"))
            .arg(":app:assembleAndroidTest")
            .current_dir(root.join("android")),
    )?;

    Ok(root.join("android/app/build/outputs/apk/debug/app-debug.apk"))
}

fn load_dotenv() {
    if let Ok(root) = repo_root() {
        let path = root.join(".env.local");
        let _ = dotenvy::from_path(path);
    }
}

fn run_cmd(cmd: &mut ProcessCommand) -> Result<()> {
    let desc = format!("{:?}", cmd);
    let status = cmd.status().with_context(|| format!("running {desc}"))?;
    if !status.success() {
        bail!("command failed: {desc}");
    }
    Ok(())
}

fn repo_root() -> Result<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .context("resolving repo root")?;
    Ok(path)
}

fn ensure_can_write(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("refusing to overwrite existing file: {:?}", path);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory {:?}", parent))?;
    }
    Ok(())
}

fn write_file(path: &Path, contents: &[u8]) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("writing file {:?}", path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_cli_spec() {
        let spec = resolve_run_spec(
            MobileTarget::Android,
            "sample_fns::fibonacci".into(),
            5,
            1,
            vec!["pixel".into()],
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(spec.function, "sample_fns::fibonacci");
        assert_eq!(spec.iterations, 5);
        assert_eq!(spec.warmup, 1);
        assert_eq!(spec.devices, vec!["pixel".to_string()]);
        assert!(spec.browserstack.is_none());
        assert!(spec.ios_xcuitest.is_none());
    }

    #[test]
    fn local_smoke_produces_samples() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
            iterations: 3,
            warmup: 1,
            devices: vec![],
            browserstack: None,
            ios_xcuitest: None,
        };
        let report = run_local_smoke(&spec).expect("local harness");
        assert!(report["samples"].is_array());
        assert_eq!(report["spec"]["name"], "sample_fns::fibonacci");
    }

    #[test]
    fn ios_requires_artifacts_for_browserstack() {
        let err = resolve_run_spec(
            MobileTarget::Ios,
            "sample_fns::fibonacci".into(),
            1,
            0,
            vec!["iphone".into()],
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("iOS BrowserStack runs require --ios-app and --ios-test-suite")
        );
    }
}
