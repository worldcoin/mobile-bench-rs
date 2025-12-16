use anyhow::{Context, Result, bail};
use bench_runner::{BenchSpec, run_closure};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use serde_json::json;

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

#[derive(Debug, Serialize, Deserialize)]
struct BrowserStackConfig {
    app_automate_username: String,
    app_automate_access_key: String,
    project: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BenchConfig {
    target: MobileTarget,
    function: String,
    iterations: u32,
    warmup: u32,
    device_matrix: PathBuf,
    browserstack: BrowserStackConfig,
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

fn main() -> Result<()> {
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
        } => {
            let output_path = output.clone();
            if let Some(config) = config {
                let cfg = load_config(&config)?;
                println!("Loaded config from {:?}:", config);
                println!("  target: {:?}", cfg.target);
                println!("  function: {}", cfg.function);
                println!("  iterations: {}", cfg.iterations);
                println!("  warmup: {}", cfg.warmup);
                println!("  device matrix: {:?}", cfg.device_matrix);
                println!("  BrowserStack project: {:?}", cfg.browserstack.project);
            } else {
                println!("Preparing benchmark run:");
                println!("  target: {:?}", target);
                println!("  function: {}", function);
                println!("  iterations: {}", iterations);
                println!("  warmup: {}", warmup);
                if !devices.is_empty() {
                    println!("  devices: {}", devices.join(", "));
                }
                if let Some(output) = &output_path {
                    println!("  output: {:?}", output);
                }
            }

            println!("Note: mobile build/upload execution not implemented yet.");
            if target == MobileTarget::Android {
                let ndk = std::env::var("ANDROID_NDK_HOME")
                    .context("ANDROID_NDK_HOME must be set for Android builds")?;
                let apk = run_android_build(&ndk)?;

                let stub = json!({
                    "target": "android",
                    "apk": apk,
                    "status": "built",
                    "note": "Execution on device/emulator not implemented yet"
                });

                if let Some(path) = output_path {
                    write_file(&path, stub.to_string().as_bytes())?;
                    println!("Wrote build stub to {:?}", path);
                } else {
                    println!("{stub}");
                }
            }
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
        Command::Init { output } => {
            write_config_template(&output, MobileTarget::Android)?;
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

fn load_config(path: &Path) -> Result<BenchConfig> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading config {:?}", path))?;
    toml::from_str(&contents).with_context(|| format!("parsing config {:?}", path))
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

    Ok(root.join("android/app/build/outputs/apk/debug/app-debug.apk"))
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
