use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, ValueEnum};
use inferno::collapse::Collapse;
use inferno::{collapse::sample as inferno_sample, flamegraph};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Cursor};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{
    DevicePlatform, MobileTarget, ProjectLayoutOptions, ResolvedMatrixDevice, RunSpec,
    load_dotenv_for_layout, persist_mobile_spec, resolve_devices_for_profile,
    resolve_project_layout, run_android_build, run_ios_build, validate_benchmark_function,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileBackend {
    Auto,
    AndroidNative,
    IosInstruments,
    RustTracing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileFormat {
    Native,
    Processed,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileProvider {
    Local,
    Browserstack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileSummaryFormat {
    Markdown,
    Json,
}

#[derive(Debug, Clone, Args)]
#[command(
    about = "Execute a native profiling session locally, or write a profile contract when execution is unsupported",
    after_help = concat!(
        "Capability matrix:\n",
        "  local + android-native: builds the Android bench app, captures simpleperf, writes folded stacks, and renders flamegraph.html\n",
        "  local + ios-instruments: builds the iOS Simulator bench app, samples the simulator-host process, writes folded stacks, and renders flamegraph.html\n",
        "  local + rust-tracing: planned manifest today; structured trace output is local-only and still not implemented\n",
        "  browserstack + android-native: unsupported for native capture in this release\n",
        "  browserstack + ios-instruments: unsupported for native capture in this release\n",
        "  browserstack + rust-tracing: unsupported; use local provider for trace-events output\n",
        "\n",
        "Local capture requirements:\n",
        "  Android native capture requires one connected adb device or booted emulator plus Android SDK/NDK simpleperf tools.\n",
        "  iOS native capture runs against a local iOS Simulator selected by --device/--os-version when provided.\n",
        "\n",
        "Device selection:\n",
        "  Use --device/--os-version for one explicit device request, or --profile with optional\n",
        "  --device-matrix/--config to reuse the same deterministic resolution model as `mobench devices resolve`.\n"
    )
)]
pub struct ProfileRunArgs {
    #[arg(long, value_enum)]
    pub target: MobileTarget,
    #[arg(long, help = "Fully-qualified Rust function to profile")]
    pub function: String,
    #[arg(
        long,
        help = "Path to the benchmark crate directory containing Cargo.toml"
    )]
    pub crate_path: Option<PathBuf>,
    #[arg(long, default_value_t = 100)]
    pub iterations: u32,
    #[arg(long, default_value_t = 10)]
    pub warmup: u32,
    #[arg(long, help = "Optional path to config file")]
    pub config: Option<PathBuf>,
    #[arg(long, default_value = "target/mobench/profile")]
    pub output_dir: PathBuf,
    #[arg(
        long,
        help = "Explicit BrowserStack device name to resolve for this profiling request",
        requires = "os_version",
        conflicts_with_all = ["profile", "device_matrix"]
    )]
    pub device: Option<String>,
    #[arg(
        long,
        help = "OS version for --device (for example `16` or `14.0`)",
        requires = "device",
        conflicts_with_all = ["profile", "device_matrix"]
    )]
    pub os_version: Option<String>,
    #[arg(long, help = "Device profile/tag to resolve (for example `high-spec`)")]
    pub profile: Option<String>,
    #[arg(
        long,
        help = "Path to device matrix YAML file used with --profile or config-based device tags"
    )]
    pub device_matrix: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = ProfileProvider::Local)]
    pub provider: ProfileProvider,
    #[arg(long, value_enum, default_value_t = ProfileBackend::Auto)]
    pub backend: ProfileBackend,
    #[arg(long, value_enum, default_value_t = ProfileFormat::Both)]
    pub format: ProfileFormat,
    #[arg(long, help = "Build mobile artifacts in release mode")]
    pub release: bool,
    #[arg(
        long,
        default_value_t = 10,
        help = "Capture duration in seconds for native profiling sessions"
    )]
    pub capture_duration_secs: u64,
}

#[derive(Debug, Clone, Args)]
pub struct ProfileSummarizeArgs {
    #[arg(long, default_value = "target/mobench/profile/profile.json")]
    pub profile: PathBuf,
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = ProfileSummaryFormat::Markdown)]
    pub output_format: ProfileSummaryFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureStatus {
    Planned,
    Captured,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub label: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileManifest {
    pub run_id: String,
    pub target: MobileTarget,
    pub function: String,
    #[serde(default = "default_profile_provider")]
    pub provider: ProfileProvider,
    pub backend: ProfileBackend,
    pub format: ProfileFormat,
    pub capture_status: CaptureStatus,
    pub raw_artifacts: Vec<ArtifactRecord>,
    pub processed_artifacts: Vec<ArtifactRecord>,
    pub warnings: Vec<String>,
    pub viewer_hint: Option<String>,
}

fn default_profile_provider() -> ProfileProvider {
    ProfileProvider::Local
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedProfileTarget {
    backend: ProfileBackend,
    device: Option<ResolvedProfileDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedProfileDevice {
    name: String,
    os: String,
    os_version: String,
    identifier: String,
    profile: Option<String>,
    source: String,
}

pub fn render_profile_markdown(manifest: &ProfileManifest) -> String {
    let mut markdown = String::new();
    let _ = writeln!(markdown, "# Profile Summary");
    let _ = writeln!(markdown);
    let _ = writeln!(markdown, "- Run ID: `{}`", manifest.run_id);
    let _ = writeln!(markdown, "- Target: `{}`", manifest.target.as_str());
    let _ = writeln!(markdown, "- Function: `{}`", manifest.function);
    let _ = writeln!(
        markdown,
        "- Provider: `{}`",
        manifest.provider.to_possible_value().unwrap().get_name()
    );
    let _ = writeln!(
        markdown,
        "- Backend: `{}`",
        manifest.backend.to_possible_value().unwrap().get_name()
    );
    let _ = writeln!(
        markdown,
        "- Status: `{}`",
        capture_status_label(manifest.capture_status)
    );
    let _ = writeln!(markdown);
    let _ = writeln!(markdown, "## Raw Artifacts");
    let _ = writeln!(markdown);
    for artifact in &manifest.raw_artifacts {
        let _ = writeln!(
            markdown,
            "- `{}`: `{}`",
            artifact.label,
            artifact.path.display()
        );
    }
    let _ = writeln!(markdown);
    let _ = writeln!(markdown, "## Processed Artifacts");
    let _ = writeln!(markdown);
    for artifact in &manifest.processed_artifacts {
        let _ = writeln!(
            markdown,
            "- `{}`: `{}`",
            artifact.label,
            artifact.path.display()
        );
    }
    if !manifest.warnings.is_empty() {
        let _ = writeln!(markdown);
        let _ = writeln!(markdown, "## Warnings");
        let _ = writeln!(markdown);
        for warning in &manifest.warnings {
            let _ = writeln!(markdown, "- {}", warning);
        }
    }
    if let Some(viewer_hint) = &manifest.viewer_hint {
        let _ = writeln!(markdown);
        let _ = writeln!(markdown, "## Viewer");
        let _ = writeln!(markdown);
        let _ = writeln!(markdown, "{}", viewer_hint);
    }

    markdown
}

pub fn write_profile_manifest(path: &Path, manifest: &ProfileManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(manifest)?;
    std::fs::write(path, body)?;
    Ok(())
}

pub fn cmd_profile_run(args: &ProfileRunArgs, dry_run: bool) -> Result<()> {
    let target = resolve_profile_target(args)?;
    let run_id = build_run_id(args.target, &args.function);
    let run_output_dir = args.output_dir.join(&run_id);
    let mut manifest = build_capture_plan(args, &run_output_dir)?;
    if dry_run {
        manifest.warnings.push(
            "dry-run enabled; capture planning stopped before execution and recorded the planned artifact contract only"
                .into(),
        );
    } else {
        execute_capture(args, &target, &mut manifest)?;
    }

    std::fs::create_dir_all(&args.output_dir)?;
    std::fs::create_dir_all(&run_output_dir)?;
    create_selected_artifact_roots(&manifest.raw_artifacts, &manifest.processed_artifacts)?;
    let rendered_summary = render_profile_markdown(&manifest);

    let run_profile_path = run_output_dir.join("profile.json");
    let run_summary_path = run_output_dir.join("summary.md");
    write_profile_manifest(&run_profile_path, &manifest)?;
    std::fs::write(&run_summary_path, rendered_summary.as_bytes())?;

    let latest_profile_path = args.output_dir.join("profile.json");
    let latest_summary_path = args.output_dir.join("summary.md");
    write_profile_manifest(&latest_profile_path, &manifest)?;
    std::fs::write(&latest_summary_path, rendered_summary.as_bytes())?;

    println!("Profile session written to {}", run_profile_path.display());
    println!("Profile summary written to {}", run_summary_path.display());
    println!(
        "Latest profile manifest refreshed at {}",
        latest_profile_path.display()
    );
    println!(
        "Latest profile summary refreshed at {}",
        latest_summary_path.display()
    );
    Ok(())
}

pub fn cmd_profile_summarize(args: &ProfileSummarizeArgs) -> Result<()> {
    let rendered = cmd_profile_summarize_for_test(args)?;
    if let Some(path) = &args.output {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, rendered.as_bytes())?;
    } else {
        println!("{rendered}");
    }
    Ok(())
}

fn capture_status_label(status: CaptureStatus) -> &'static str {
    match status {
        CaptureStatus::Planned => "planned",
        CaptureStatus::Captured => "captured",
        CaptureStatus::Partial => "partial",
        CaptureStatus::Failed => "failed",
    }
}

fn load_profile_manifest(path: &Path) -> Result<ProfileManifest> {
    let body = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&body)?)
}

pub fn cmd_profile_summarize_for_test(args: &ProfileSummarizeArgs) -> Result<String> {
    let manifest = load_profile_manifest(&args.profile)?;
    match args.output_format {
        ProfileSummaryFormat::Markdown => Ok(render_profile_markdown(&manifest)),
        ProfileSummaryFormat::Json => Ok(serde_json::to_string_pretty(&manifest)?),
    }
}

fn resolve_profile_target(args: &ProfileRunArgs) -> Result<ResolvedProfileTarget> {
    let backend = resolve_backend(args.target, args.backend);
    validate_format_capabilities(backend, args.format)?;

    let device = resolve_profile_device(args)?;
    Ok(ResolvedProfileTarget { backend, device })
}

fn build_capture_plan(args: &ProfileRunArgs, output_root: &Path) -> Result<ProfileManifest> {
    let backend = resolve_backend(args.target, args.backend);
    validate_format_capabilities(backend, args.format)?;

    let raw_root = output_root.join("artifacts/raw");
    let processed_root = output_root.join("artifacts/processed");

    let (raw_artifacts, processed_artifacts) = match backend {
        ProfileBackend::AndroidNative => (
            vec![ArtifactRecord {
                label: "simpleperf".into(),
                path: raw_root.join("sample.perf"),
            }],
            vec![
                ArtifactRecord {
                    label: "collapsed-stacks".into(),
                    path: processed_root.join("stacks.folded"),
                },
                ArtifactRecord {
                    label: "flamegraph".into(),
                    path: processed_root.join("flamegraph.html"),
                },
            ],
        ),
        ProfileBackend::IosInstruments => (
            vec![ArtifactRecord {
                label: "sample".into(),
                path: raw_root.join("sample.txt"),
            }],
            vec![
                ArtifactRecord {
                    label: "collapsed-stacks".into(),
                    path: processed_root.join("stacks.folded"),
                },
                ArtifactRecord {
                    label: "flamegraph".into(),
                    path: processed_root.join("flamegraph.html"),
                },
            ],
        ),
        ProfileBackend::RustTracing => (
            vec![ArtifactRecord {
                label: "trace-events".into(),
                path: raw_root.join("trace-events.json"),
            }],
            Vec::new(),
        ),
        ProfileBackend::Auto => unreachable!("auto backend should resolve before planning"),
    };

    let raw_artifacts = select_artifacts(raw_artifacts, args.format, ArtifactKind::Raw);
    let processed_artifacts =
        select_artifacts(processed_artifacts, args.format, ArtifactKind::Processed);
    let viewer_hint =
        select_viewer_hint(backend, args.format, &raw_artifacts, &processed_artifacts);

    Ok(ProfileManifest {
        run_id: build_run_id(args.target, &args.function),
        target: args.target,
        function: args.function.clone(),
        provider: args.provider,
        backend,
        format: args.format,
        capture_status: CaptureStatus::Planned,
        raw_artifacts,
        processed_artifacts,
        warnings: Vec::new(),
        viewer_hint,
    })
}

fn build_run_id(target: MobileTarget, function: &str) -> String {
    format!("{}-{}", target.as_str(), slugify_function_name(function))
}

fn resolve_backend(target: MobileTarget, backend: ProfileBackend) -> ProfileBackend {
    match backend {
        ProfileBackend::Auto => match target {
            MobileTarget::Android => ProfileBackend::AndroidNative,
            MobileTarget::Ios => ProfileBackend::IosInstruments,
        },
        _ => backend,
    }
}

fn validate_format_capabilities(backend: ProfileBackend, format: ProfileFormat) -> Result<()> {
    if backend == ProfileBackend::RustTracing && format == ProfileFormat::Processed {
        bail!(
            "processed output is unsupported for rust-tracing backend; use --format native or both"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactKind {
    Raw,
    Processed,
}

fn select_artifacts(
    artifacts: Vec<ArtifactRecord>,
    format: ProfileFormat,
    kind: ArtifactKind,
) -> Vec<ArtifactRecord> {
    match format {
        ProfileFormat::Both => artifacts,
        ProfileFormat::Native if kind == ArtifactKind::Raw => artifacts,
        ProfileFormat::Processed if kind == ArtifactKind::Processed => artifacts,
        _ => Vec::new(),
    }
}

fn create_selected_artifact_roots(
    raw_artifacts: &[ArtifactRecord],
    processed_artifacts: &[ArtifactRecord],
) -> Result<()> {
    for artifact in raw_artifacts.iter().chain(processed_artifacts.iter()) {
        if let Some(parent) = artifact.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn resolve_profile_device(args: &ProfileRunArgs) -> Result<Option<ResolvedProfileDevice>> {
    match (args.device.as_deref(), args.os_version.as_deref()) {
        (Some(device), Some(os_version)) => {
            let identifier = format!("{device}-{os_version}");
            Ok(Some(ResolvedProfileDevice {
                name: device.to_string(),
                os: args.target.as_str().to_string(),
                os_version: os_version.to_string(),
                identifier,
                profile: None,
                source: "direct".into(),
            }))
        }
        (None, None) => {
            if args.profile.is_none() && args.device_matrix.is_none() {
                return Ok(None);
            }

            let platform = match args.target {
                MobileTarget::Android => DevicePlatform::Android,
                MobileTarget::Ios => DevicePlatform::Ios,
            };
            let resolved = resolve_devices_for_profile(
                platform,
                args.profile.as_deref(),
                args.config.as_deref(),
                args.device_matrix.as_deref(),
            )?;
            if resolved.devices.len() != 1 {
                bail!(
                    "profile run requires exactly one resolved device, but profile `{}` from {} produced {} devices; use --device/--os-version or a single-device profile",
                    resolved.profile,
                    resolved.source,
                    resolved.devices.len()
                );
            }
            Ok(Some(resolved_profile_device_from_matrix(
                resolved.devices.into_iter().next().expect("single device"),
                resolved.profile,
                resolved.source,
            )))
        }
        _ => unreachable!("clap enforces paired --device/--os-version"),
    }
}

fn resolved_profile_device_from_matrix(
    device: ResolvedMatrixDevice,
    profile: String,
    source: String,
) -> ResolvedProfileDevice {
    ResolvedProfileDevice {
        name: device.name,
        os: device.os,
        os_version: device.os_version,
        identifier: device.identifier,
        profile: Some(profile),
        source,
    }
}

fn execute_capture(
    args: &ProfileRunArgs,
    target: &ResolvedProfileTarget,
    manifest: &mut ProfileManifest,
) -> Result<()> {
    if let Some(device) = &target.device {
        manifest.warnings.push(format!(
            "resolved target device: {} ({}, source: {})",
            device.identifier, device.os, device.source
        ));
    }

    match (args.provider, target.backend) {
        (ProfileProvider::Local, ProfileBackend::AndroidNative) => {
            execute_local_android_native(args, target, manifest)?
        }
        (ProfileProvider::Local, ProfileBackend::IosInstruments) => {
            execute_local_ios_instruments(args, target, manifest)?
        }
        (ProfileProvider::Local, ProfileBackend::RustTracing) => manifest.warnings.push(
            "local rust-tracing capture is not implemented yet; this session records the planned trace-events artifact contract only"
                .into(),
        ),
        (ProfileProvider::Browserstack, ProfileBackend::AndroidNative) => {
            bail!(browserstack_native_capture_unsupported_message(
                "android-native",
                "local Android profiling produces simpleperf captures, folded stacks, and flamegraph.html",
            ));
        }
        (ProfileProvider::Browserstack, ProfileBackend::IosInstruments) => {
            bail!(browserstack_native_capture_unsupported_message(
                "ios-instruments",
                "local iOS profiling on simulator produces sample-based folded stacks and flamegraph.html",
            ));
        }
        (ProfileProvider::Browserstack, ProfileBackend::RustTracing) => {
            bail!(
                "BrowserStack rust-tracing capture is not implemented.\nThis command currently writes a local-first profile contract only.\nUse --provider local for trace-events output, or run a normal BrowserStack benchmark if you only need timing/memory metrics."
            );
        }
        (_, ProfileBackend::Auto) => unreachable!("auto backend should resolve before execution"),
    }
    Ok(())
}

fn browserstack_native_capture_unsupported_message(
    backend_label: &str,
    artifact_guidance: &str,
) -> String {
    format!(
        "BrowserStack native profiling is not implemented for {backend_label}.\nThis command currently writes a local-first profile contract only.\nUse --provider local for planning/local capture, or run a normal BrowserStack benchmark if you only need timing/memory metrics.\n{artifact_guidance}."
    )
}

#[derive(Debug)]
struct PreparedLocalProfileRun {
    layout: crate::ResolvedProjectLayout,
}

#[derive(Debug)]
struct AndroidToolchain {
    adb: PathBuf,
    ndk_home: PathBuf,
    app_profiler: PathBuf,
    stackcollapse: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalIosSimulator {
    name: String,
    udid: String,
    os_version: String,
    state: String,
}

#[derive(Debug, Deserialize)]
struct SimctlList {
    devices: BTreeMap<String, Vec<SimctlDevice>>,
}

#[derive(Debug, Deserialize)]
struct SimctlDevice {
    name: String,
    udid: String,
    state: String,
    #[serde(rename = "isAvailable", default)]
    is_available: bool,
}

fn execute_local_android_native(
    args: &ProfileRunArgs,
    target: &ResolvedProfileTarget,
    manifest: &mut ProfileManifest,
) -> Result<()> {
    if target.device.is_some() {
        manifest.warnings.push(
            "local android-native capture uses the connected adb target; BrowserStack-style device resolution is ignored locally. Set ANDROID_SERIAL if more than one device is attached."
                .into(),
        );
    }

    let prepared = prepare_local_profile_run(args)?;
    mobench_sdk::codegen::ensure_android_project_with_options(
        &prepared.layout.output_dir,
        &prepared.layout.crate_name,
        Some(&prepared.layout.project_root),
        Some(&prepared.layout.crate_dir),
    )
    .map_err(|err| anyhow!("failed to generate Android project scaffolding: {err}"))?;
    ensure_android_profileable_manifest(&prepared.layout.output_dir)?;

    let toolchain = resolve_android_toolchain()?;
    let selected_serial = select_android_serial(&toolchain.adb)?;
    let build = run_android_build(
        &prepared.layout,
        &toolchain.ndk_home.display().to_string(),
        args.release,
        false,
    )?;

    install_android_apk(&toolchain.adb, &selected_serial, &build.app_path)?;

    let output_root = profile_output_root(args, manifest);
    let scratch_root = output_root.join(".capture-tmp/android");
    let perf_path = raw_artifact_path_or_scratch(manifest, &scratch_root, "simpleperf", "sample.perf");
    let workdir = perf_path
        .parent()
        .ok_or_else(|| anyhow!("invalid simpleperf output path {}", perf_path.display()))?
        .to_path_buf();
    fs::create_dir_all(&workdir)?;

    run_android_simpleperf_capture(
        &toolchain,
        &selected_serial,
        &prepared.layout,
        &perf_path,
        args.capture_duration_secs,
    )?;

    if matches!(args.format, ProfileFormat::Processed | ProfileFormat::Both) {
        let folded_path = processed_artifact_path_or_scratch(
            manifest,
            &scratch_root,
            "collapsed-stacks",
            "stacks.folded",
        );
        let flamegraph_path = processed_artifact_path_or_scratch(
            manifest,
            &scratch_root,
            "flamegraph",
            "flamegraph.html",
        );

        match write_android_processed_outputs(
            &toolchain,
            &perf_path,
            &folded_path,
            &flamegraph_path,
            &manifest.function,
        ) {
            Ok(()) => manifest.capture_status = CaptureStatus::Captured,
            Err(err) if matches!(args.format, ProfileFormat::Both) => {
                manifest.capture_status = CaptureStatus::Partial;
                manifest.warnings.push(format!(
                    "native Android capture succeeded, but folded stack or flamegraph generation failed: {err}"
                ));
                return Ok(());
            }
            Err(err) => return Err(err),
        }
    } else {
        manifest.capture_status = CaptureStatus::Captured;
    }

    manifest.warnings.push(format!(
        "captured Android simpleperf profile via adb target `{selected_serial}`"
    ));
    Ok(())
}

fn execute_local_ios_instruments(
    args: &ProfileRunArgs,
    target: &ResolvedProfileTarget,
    manifest: &mut ProfileManifest,
) -> Result<()> {
    let prepared = prepare_local_profile_run(args)?;
    run_ios_build(&prepared.layout, args.release, false)?;

    let simulator = select_local_ios_simulator(target.device.as_ref())?;
    let output_root = profile_output_root(args, manifest);
    let scratch_root = output_root.join(".capture-tmp/ios");
    let derived_data = scratch_root.join("DerivedData");
    fs::create_dir_all(&scratch_root)?;

    boot_ios_simulator(&simulator)?;
    ensure_ios_bench_delay_support(&prepared.layout.output_dir)?;
    let app_path = build_ios_simulator_app(&prepared.layout, args.release, &simulator, &derived_data)?;
    let sample_path = raw_artifact_path_or_scratch(manifest, &scratch_root, "sample", "sample.txt");
    capture_ios_sample_profile(
        &simulator,
        &prepared.layout.crate_name,
        &app_path,
        &sample_path,
        args.capture_duration_secs,
    )?;

    if matches!(args.format, ProfileFormat::Processed | ProfileFormat::Both) {
        let folded_path = processed_artifact_path_or_scratch(
            manifest,
            &scratch_root,
            "collapsed-stacks",
            "stacks.folded",
        );
        let flamegraph_path = processed_artifact_path_or_scratch(
            manifest,
            &scratch_root,
            "flamegraph",
            "flamegraph.html",
        );

        match write_ios_processed_outputs(&sample_path, &folded_path, &flamegraph_path, &manifest.function)
        {
            Ok(()) => manifest.capture_status = CaptureStatus::Captured,
            Err(err) if matches!(args.format, ProfileFormat::Both) => {
                manifest.capture_status = CaptureStatus::Partial;
                manifest.warnings.push(format!(
                    "native iOS capture succeeded, but folded stack or flamegraph generation failed: {err}"
                ));
                return Ok(());
            }
            Err(err) => return Err(err),
        }
    } else {
        manifest.capture_status = CaptureStatus::Captured;
    }

    manifest.warnings.push(format!(
        "captured iOS simulator sample profile for the ios-instruments backend on {} ({})",
        simulator.name, simulator.os_version
    ));
    Ok(())
}

fn prepare_local_profile_run(args: &ProfileRunArgs) -> Result<PreparedLocalProfileRun> {
    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: None,
        crate_path: args.crate_path.as_deref(),
        config_path: args.config.as_deref(),
    })?;
    load_dotenv_for_layout(&layout);
    validate_benchmark_function(&layout, &args.function)?;

    let spec = RunSpec {
        target: args.target,
        function: args.function.clone(),
        iterations: args.iterations,
        warmup: args.warmup,
        devices: Vec::new(),
        browserstack: None,
        ios_xcuitest: None,
    };
    persist_mobile_spec(&layout, &spec, args.release)?;

    Ok(PreparedLocalProfileRun { layout })
}

fn profile_output_root(args: &ProfileRunArgs, manifest: &ProfileManifest) -> PathBuf {
    args.output_dir.join(&manifest.run_id)
}

fn artifact_path(records: &[ArtifactRecord], label: &str) -> Option<PathBuf> {
    records
        .iter()
        .find(|artifact| artifact.label == label)
        .map(|artifact| artifact.path.clone())
}

fn raw_artifact_path_or_scratch(
    manifest: &ProfileManifest,
    scratch_root: &Path,
    label: &str,
    filename: &str,
) -> PathBuf {
    artifact_path(&manifest.raw_artifacts, label)
        .unwrap_or_else(|| scratch_root.join("raw").join(filename))
}

fn processed_artifact_path_or_scratch(
    manifest: &ProfileManifest,
    scratch_root: &Path,
    label: &str,
    filename: &str,
) -> PathBuf {
    artifact_path(&manifest.processed_artifacts, label)
        .unwrap_or_else(|| scratch_root.join("processed").join(filename))
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path {} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)?;
    Ok(())
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    Ok(std::env::current_dir()
        .context("resolving current directory for profile artifact path")?
        .join(path))
}

fn run_command_output(command: &mut Command, description: &str) -> Result<std::process::Output> {
    let output = command
        .output()
        .with_context(|| format!("failed to run {description}"))?;
    if output.status.success() {
        return Ok(output);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!(
        "{description} failed with status {}.\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout.trim(),
        stderr.trim()
    )
}

fn resolve_android_toolchain() -> Result<AndroidToolchain> {
    let sdk_roots = android_sdk_roots();
    let adb = resolve_command_path(
        "adb",
        sdk_roots
            .iter()
            .map(|root| root.join("platform-tools/adb"))
            .collect(),
    )?;

    let ndk_home = if let Ok(explicit) = std::env::var("ANDROID_NDK_HOME") {
        PathBuf::from(explicit)
    } else {
        sdk_roots
            .iter()
            .find_map(|root| newest_directory(root.join("ndk")))
            .ok_or_else(|| {
                anyhow!(
                    "ANDROID_NDK_HOME is not set and no Android NDK was found under the local SDK. Set ANDROID_NDK_HOME before running local Android profiling."
                )
            })?
    };

    let app_profiler = ndk_home.join("simpleperf/app_profiler.py");
    let stackcollapse = ndk_home.join("simpleperf/stackcollapse.py");
    for path in [&app_profiler, &stackcollapse] {
        if !path.is_file() {
            bail!(
                "required simpleperf helper was not found at {}. Install an Android NDK with simpleperf support.",
                path.display()
            );
        }
    }

    Ok(AndroidToolchain {
        adb,
        ndk_home,
        app_profiler,
        stackcollapse,
    })
}

fn android_sdk_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(value) = std::env::var_os(key) {
            roots.push(PathBuf::from(value));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Library/Android/sdk"));
    }
    roots.retain(|path| path.exists());
    roots.sort();
    roots.dedup();
    roots
}

fn newest_directory(root: PathBuf) -> Option<PathBuf> {
    let mut entries = fs::read_dir(root).ok()?.filter_map(|entry| {
        let entry = entry.ok()?;
        entry.file_type().ok()?.is_dir().then_some(entry.path())
    }).collect::<Vec<_>>();
    entries.sort();
    entries.pop()
}

fn resolve_command_path(binary: &str, fallback_paths: Vec<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var)
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    }) {
        return Ok(path);
    }

    if let Some(path) = fallback_paths.into_iter().find(|candidate| candidate.is_file()) {
        return Ok(path);
    }

    bail!("required executable `{binary}` was not found on PATH")
}

fn select_android_serial(adb: &Path) -> Result<String> {
    if let Ok(serial) = std::env::var("ANDROID_SERIAL")
        && !serial.trim().is_empty()
    {
        return Ok(serial);
    }

    let output = run_command_output(
        Command::new(adb).arg("devices").arg("-l"),
        "listing connected Android devices",
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let devices = stdout
        .lines()
        .skip(1)
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('*') {
                return None;
            }
            let mut parts = trimmed.split_whitespace();
            let serial = parts.next()?;
            let state = parts.next()?;
            (state == "device").then_some(serial.to_string())
        })
        .collect::<Vec<_>>();

    match devices.as_slice() {
        [serial] => Ok(serial.clone()),
        [] => bail!(
            "no connected Android devices were found for local profiling. Connect a device or boot an emulator, then rerun `mobench profile run --target android ...`"
        ),
        _ => bail!(
            "multiple Android devices are connected for local profiling. Set ANDROID_SERIAL to select one target. Connected devices: {}",
            devices.join(", ")
        ),
    }
}

fn install_android_apk(adb: &Path, serial: &str, apk_path: &Path) -> Result<()> {
    run_command_output(
        Command::new(adb)
            .arg("-s")
            .arg(serial)
            .arg("install")
            .arg("-r")
            .arg(apk_path),
        "installing Android benchmark APK",
    )?;
    Ok(())
}

fn run_android_simpleperf_capture(
    toolchain: &AndroidToolchain,
    serial: &str,
    layout: &crate::ResolvedProjectLayout,
    perf_path: &Path,
    capture_duration_secs: u64,
) -> Result<()> {
    ensure_parent_dir(perf_path)?;
    let perf_path = absolute_path(perf_path)?;
    let workdir = perf_path
        .parent()
        .ok_or_else(|| anyhow!("invalid perf output path {}", perf_path.display()))?;
    let package_name = android_package_name(&layout.crate_name);
    let native_lib_dir = layout.output_dir.join("android/app/src/main/jniLibs");
    let record_options = format!(
        "-e task-clock:u -f 1000 -g --duration {}",
        capture_duration_secs.max(1)
    );

    run_command_output(
        Command::new("python3")
            .arg(&toolchain.app_profiler)
            .arg("-p")
            .arg(&package_name)
            .arg("-a")
            .arg(".MainActivity")
            .arg("-r")
            .arg(&record_options)
            .arg("-lib")
            .arg(&native_lib_dir)
            .arg("-o")
            .arg(&perf_path)
            .arg("--ndk_path")
            .arg(&toolchain.ndk_home)
            .env("ANDROID_SERIAL", serial)
            .current_dir(workdir),
        "capturing Android simpleperf profile",
    )?;
    Ok(())
}

fn write_android_processed_outputs(
    toolchain: &AndroidToolchain,
    perf_path: &Path,
    folded_path: &Path,
    flamegraph_path: &Path,
    function: &str,
) -> Result<()> {
    ensure_parent_dir(folded_path)?;
    let perf_path = absolute_path(perf_path)?;
    let workdir = perf_path
        .parent()
        .ok_or_else(|| anyhow!("invalid perf output path {}", perf_path.display()))?;
    let binary_cache = workdir.join("binary_cache");
    if !binary_cache.exists() {
        bail!(
            "simpleperf binary cache was not found at {} after capture",
            binary_cache.display()
        );
    }

    let output = run_command_output(
        Command::new("python3")
            .arg(&toolchain.stackcollapse)
            .arg("--symfs")
            .arg(&binary_cache)
            .arg("-i")
            .arg(&perf_path)
            .current_dir(workdir),
        "collapsing Android simpleperf stacks",
    )?;
    fs::write(folded_path, &output.stdout)?;
    render_flamegraph_html(
        folded_path,
        flamegraph_path,
        &format!("Android Native Flamegraph: {function}"),
    )
}

fn android_package_name(crate_name: &str) -> String {
    format!(
        "dev.world.{}",
        mobench_sdk::codegen::sanitize_bundle_id_component(crate_name)
    )
}

fn ensure_android_profileable_manifest(output_dir: &Path) -> Result<()> {
    let manifest_path = output_dir.join("android/app/src/main/AndroidManifest.xml");
    if !manifest_path.exists() {
        return Ok(());
    }
    let manifest = fs::read_to_string(&manifest_path)?;
    if manifest.contains("<profileable") {
        return Ok(());
    }

    let updated = manifest.replacen(
        "<activity",
        "        <profileable android:shell=\"true\" />\n        <activity",
        1,
    );
    if updated == manifest {
        bail!(
            "failed to update Android manifest at {} with a profileable tag",
            manifest_path.display()
        );
    }
    fs::write(manifest_path, updated)?;
    Ok(())
}

fn select_local_ios_simulator(requested: Option<&ResolvedProfileDevice>) -> Result<LocalIosSimulator> {
    let output = run_command_output(
        Command::new("xcrun")
            .arg("simctl")
            .arg("list")
            .arg("devices")
            .arg("available")
            .arg("--json"),
        "listing available iOS simulators",
    )?;
    let listing: SimctlList = serde_json::from_slice(&output.stdout)
        .context("parsing simctl device listing JSON")?;

    let mut simulators = listing
        .devices
        .into_iter()
        .filter_map(|(runtime, devices)| {
            let os_version = runtime_version_from_simctl_runtime(&runtime)?;
            Some(
                devices
                    .into_iter()
                    .filter(|device| device.is_available && device.name.starts_with("iPhone"))
                    .map(move |device| LocalIosSimulator {
                        name: device.name,
                        udid: device.udid,
                        os_version: os_version.clone(),
                        state: device.state,
                    }),
            )
        })
        .flatten()
        .collect::<Vec<_>>();

    if simulators.is_empty() {
        bail!("no available iOS simulators were found for local profiling");
    }

    simulators.sort_by(|left, right| {
        simulator_sort_key(right).cmp(&simulator_sort_key(left))
    });

    if let Some(requested) = requested {
        if let Some(simulator) = simulators
            .iter()
            .find(|simulator| {
                simulator.name == requested.name
                    && os_version_matches(&simulator.os_version, &requested.os_version)
            })
            .cloned()
        {
            return Ok(simulator);
        }

        let available = simulators
            .iter()
            .map(|simulator| format!("{} ({})", simulator.name, simulator.os_version))
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "no local iOS simulator matched requested device {} (iOS {}). Available simulators: {}",
            requested.name,
            requested.os_version,
            available
        );
    }

    simulators
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no suitable iOS simulator was available"))
}

fn simulator_sort_key(simulator: &LocalIosSimulator) -> (bool, Vec<u32>, &str) {
    (
        simulator.state == "Booted",
        version_parts(&simulator.os_version),
        simulator.name.as_str(),
    )
}

fn version_parts(version: &str) -> Vec<u32> {
    version
        .split('.')
        .filter_map(|segment| segment.parse::<u32>().ok())
        .collect()
}

fn runtime_version_from_simctl_runtime(runtime: &str) -> Option<String> {
    runtime
        .split('.')
        .next_back()
        .and_then(|segment| segment.strip_prefix("iOS-"))
        .map(|version| version.replace('-', "."))
}

fn os_version_matches(candidate: &str, requested: &str) -> bool {
    candidate == requested
        || candidate.starts_with(&format!("{requested}."))
        || requested.starts_with(&format!("{candidate}."))
}

fn boot_ios_simulator(simulator: &LocalIosSimulator) -> Result<()> {
    if simulator.state != "Booted" {
        run_command_output(
            Command::new("xcrun")
                .arg("simctl")
                .arg("boot")
                .arg(&simulator.udid),
            "booting iOS simulator",
        )?;
    }
    run_command_output(
        Command::new("xcrun")
            .arg("simctl")
            .arg("bootstatus")
            .arg(&simulator.udid)
            .arg("-b"),
        "waiting for iOS simulator boot",
    )?;
    Ok(())
}

fn build_ios_simulator_app(
    layout: &crate::ResolvedProjectLayout,
    release: bool,
    _simulator: &LocalIosSimulator,
    derived_data: &Path,
) -> Result<PathBuf> {
    let derived_data = if derived_data.is_absolute() {
        derived_data.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolving absolute iOS simulator build output path")?
            .join(derived_data)
    };
    let configuration = if release { "Release" } else { "Debug" };
    let project_path = layout.output_dir.join("ios/BenchRunner/BenchRunner.xcodeproj");
    let config_build_dir = derived_data.join(format!("{configuration}-iphonesimulator"));
    run_command_output(
        Command::new("xcodebuild")
            .arg("-project")
            .arg(&project_path)
            .arg("-target")
            .arg("BenchRunner")
            .arg("-configuration")
            .arg(configuration)
            .arg("build")
            .arg("SDKROOT=iphonesimulator")
            .arg("SUPPORTED_PLATFORMS=iphonesimulator iphoneos")
            .arg("CODE_SIGNING_ALLOWED=NO")
            .arg("CODE_SIGNING_REQUIRED=NO")
            .arg(format!("CONFIGURATION_BUILD_DIR={}", config_build_dir.display()))
            .arg(format!("OBJROOT={}", derived_data.join("Intermediates").display()))
            .arg(format!("SYMROOT={}", derived_data.join("Products").display())),
        "building iOS simulator benchmark app",
    )?;

    let app_path = config_build_dir.join("BenchRunner.app");
    if !app_path.exists() {
        bail!(
            "expected iOS simulator app at {}, but the build output was missing",
            app_path.display()
        );
    }
    Ok(app_path)
}

fn ensure_ios_bench_delay_support(output_dir: &Path) -> Result<()> {
    let content_view_path = output_dir.join("ios/BenchRunner/BenchRunner/ContentView.swift");
    if !content_view_path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(&content_view_path)?;
    if content.contains("MOBENCH_BENCH_DELAY_MS") {
        return Ok(());
    }
    let updated = content.replacen(
        "            Task {\n                let result = await BenchRunnerFFI.runCurrentBenchmark()\n",
        "            Task {\n                if let delay = ProcessInfo.processInfo.environment[\"MOBENCH_BENCH_DELAY_MS\"],\n                   let delayMs = UInt64(delay) {\n                    try? await Task.sleep(nanoseconds: delayMs * 1_000_000)\n                }\n                let result = await BenchRunnerFFI.runCurrentBenchmark()\n",
        1,
    );
    if updated == content {
        bail!(
            "failed to inject benchmark start delay support into {}",
            content_view_path.display()
        );
    }
    fs::write(content_view_path, updated)?;
    Ok(())
}

fn capture_ios_sample_profile(
    simulator: &LocalIosSimulator,
    crate_name: &str,
    app_path: &Path,
    sample_path: &Path,
    capture_duration_secs: u64,
) -> Result<()> {
    ensure_parent_dir(sample_path)?;
    install_ios_simulator_app(simulator, app_path)?;
    launch_ios_app_with_delay(simulator, crate_name)?;
    let host_pid = wait_for_ios_host_process_pid()?;
    run_command_output(
        Command::new("sample")
            .arg(host_pid.to_string())
            .arg(capture_duration_secs.max(1).to_string())
            .arg("-file")
            .arg(sample_path),
        "capturing iOS simulator sample profile",
    )?;
    Ok(())
}

fn install_ios_simulator_app(simulator: &LocalIosSimulator, app_path: &Path) -> Result<()> {
    run_command_output(
        Command::new("xcrun")
            .arg("simctl")
            .arg("install")
            .arg(&simulator.udid)
            .arg(app_path),
        "installing iOS simulator benchmark app",
    )?;
    Ok(())
}

fn launch_ios_app_with_delay(simulator: &LocalIosSimulator, crate_name: &str) -> Result<()> {
    let bundle_id = ios_bundle_identifier(crate_name);
    let _ = run_command_output(
        Command::new("xcrun")
            .arg("simctl")
            .arg("terminate")
            .arg(&simulator.udid)
            .arg(&bundle_id),
        "terminating previous iOS simulator benchmark app",
    );
    run_command_output(
        Command::new("xcrun")
            .arg("simctl")
            .arg("launch")
            .arg("--terminate-running-process")
            .arg(&simulator.udid)
            .arg(&bundle_id)
            .env("SIMCTL_CHILD_MOBENCH_BENCH_DELAY_MS", "4000"),
        "launching iOS simulator benchmark app",
    )?;
    Ok(())
}

fn wait_for_ios_host_process_pid() -> Result<u32> {
    for _ in 0..20 {
        let output = run_command_output(
            Command::new("pgrep")
                .arg("-f")
                .arg("BenchRunner.app/BenchRunner"),
            "locating iOS simulator benchmark host process",
        );
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(pid) = stdout.lines().find_map(|line| line.trim().parse::<u32>().ok()) {
                return Ok(pid);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    bail!("timed out waiting for the iOS simulator benchmark process to appear")
}

fn ios_bundle_identifier(crate_name: &str) -> String {
    format!(
        "dev.world.{}.BenchRunner",
        mobench_sdk::codegen::sanitize_bundle_id_component(crate_name)
    )
}

fn write_ios_processed_outputs(
    sample_path: &Path,
    folded_path: &Path,
    flamegraph_path: &Path,
    function: &str,
) -> Result<()> {
    ensure_parent_dir(folded_path)?;
    let input = BufReader::new(File::open(sample_path)?);
    let output = BufWriter::new(File::create(folded_path)?);
    inferno_sample::Folder::default()
        .collapse(input, output)
        .context("collapsing iOS sample output to folded stacks")?;

    render_flamegraph_html(
        folded_path,
        flamegraph_path,
        &format!("iOS Simulator Flamegraph: {function}"),
    )
}

fn render_flamegraph_html(folded_path: &Path, html_path: &Path, title: &str) -> Result<()> {
    ensure_parent_dir(html_path)?;

    let folded = fs::read(folded_path)?;
    if folded.is_empty() {
        bail!("folded stack file {} was empty", folded_path.display());
    }

    let mut options = flamegraph::Options::default();
    options.title = title.to_string();
    options.count_name = "samples".into();
    options.notes = "Generated by mobench profile run".into();

    let mut svg = Vec::new();
    flamegraph::from_reader(&mut options, Cursor::new(folded), &mut svg)
        .context("rendering flamegraph SVG")?;
    let svg = String::from_utf8(svg).context("decoding rendered flamegraph SVG")?;
    let html = wrap_svg_document(title, &svg);
    fs::write(html_path, html.as_bytes())?;
    Ok(())
}

fn wrap_svg_document(title: &str, svg: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title><style>body{{margin:0;background:#fff;font-family:ui-monospace,SFMono-Regular,Menlo,monospace}}svg{{display:block;width:100%;height:auto}}</style></head><body>{}</body></html>",
        escape_html(title),
        svg
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
}

fn select_viewer_hint(
    backend: ProfileBackend,
    format: ProfileFormat,
    raw_artifacts: &[ArtifactRecord],
    processed_artifacts: &[ArtifactRecord],
) -> Option<String> {
    match backend {
        ProfileBackend::AndroidNative => {
            if format != ProfileFormat::Native && !processed_artifacts.is_empty() {
                Some(
                    "Open artifacts/processed/flamegraph.html in a browser, or inspect artifacts/processed/stacks.folded for folded stacks"
                        .into(),
                )
            } else if !raw_artifacts.is_empty() {
                Some(
                    "Inspect artifacts/raw/sample.perf with the Android profiling toolchain".into(),
                )
            } else {
                None
            }
        }
        ProfileBackend::IosInstruments => {
            if format != ProfileFormat::Native && !processed_artifacts.is_empty() {
                Some(
                    "Open artifacts/processed/flamegraph.html in a browser, or inspect artifacts/processed/stacks.folded"
                        .into(),
                )
            } else if !raw_artifacts.is_empty() {
                Some(
                    "Inspect artifacts/raw/sample.txt or rerun with --format both to keep the folded stacks and flamegraph"
                        .into(),
                )
            } else {
                None
            }
        }
        ProfileBackend::RustTracing => {
            if !raw_artifacts.is_empty() {
                Some("Open artifacts/raw/trace-events.json in a trace viewer".into())
            } else {
                None
            }
        }
        ProfileBackend::Auto => None,
    }
}

fn slugify_function_name(function: &str) -> String {
    let mut slug = String::new();
    for ch in function.chars() {
        match ch {
            ':' | '/' | ' ' => slug.push('-'),
            '_' | '-' => slug.push(ch),
            ch if ch.is_ascii_alphanumeric() => slug.push(ch),
            _ => slug.push('_'),
        }
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_run_args(
        target: MobileTarget,
        provider: ProfileProvider,
        backend: ProfileBackend,
        format: ProfileFormat,
    ) -> ProfileRunArgs {
        ProfileRunArgs {
            target,
            function: "sample_fns::fibonacci".into(),
            crate_path: None,
            iterations: 100,
            warmup: 10,
            config: None,
            output_dir: PathBuf::from("target/mobench/profile"),
            device: None,
            os_version: None,
            profile: None,
            device_matrix: None,
            provider,
            backend,
            format,
            release: false,
            capture_duration_secs: 10,
        }
    }

    #[test]
    fn profile_manifest_serializes_partial_failure_state() {
        let manifest = sample_manifest();

        let json = serde_json::to_value(&manifest).expect("serialize manifest");
        assert_eq!(json["warnings"][0], "missing symbols");
        assert_eq!(json["capture_status"], "partial");
    }

    #[test]
    fn profile_manifest_serializes_native_capture_sections() {
        let manifest = sample_manifest();

        let json = serde_json::to_value(&manifest).expect("serialize manifest");
        assert!(
            json.get("native_capture").is_some(),
            "expected native capture metadata to be nested under native_capture, got: {json}"
        );
        assert!(
            json["native_capture"].get("symbolization").is_some(),
            "expected native capture metadata to include symbolization state, got: {json}"
        );
    }

    #[test]
    fn profile_manifest_serializes_semantic_profile_sections() {
        let manifest = sample_manifest();

        let json = serde_json::to_value(&manifest).expect("serialize manifest");
        assert!(
            json.get("semantic_profile").is_some(),
            "expected semantic profiling metadata to be nested under semantic_profile, got: {json}"
        );
        assert!(
            json["semantic_profile"].get("phases").is_some(),
            "expected semantic profiling metadata to expose phase data, got: {json}"
        );
    }

    #[test]
    fn render_profile_summary_mentions_backend_and_artifacts() {
        let manifest = sample_manifest();
        let markdown = render_profile_markdown(&manifest);

        assert!(markdown.contains("android-native"));
        assert!(markdown.contains("artifacts/raw/sample.perf"));
        assert!(markdown.contains("missing symbols"));
    }

    #[test]
    fn render_profile_summary_separates_native_and_semantic_outputs() {
        let manifest = sample_manifest();
        let markdown = render_profile_markdown(&manifest);

        assert!(
            markdown.contains("Native capture") || markdown.contains("native capture"),
            "expected a native capture section, got:\n{markdown}"
        );
        assert!(
            markdown.contains("Semantic phases"),
            "expected a semantic phases section, got:\n{markdown}"
        );
        assert!(
            markdown.contains("flamegraph.html") || markdown.contains("sample.perf"),
            "expected native artifact references to remain visible, got:\n{markdown}"
        );
    }

    #[test]
    fn summarize_command_reads_manifest_and_renders_markdown() {
        let dir = tempfile::tempdir().expect("temp dir");
        let manifest_path = dir.path().join("profile.json");
        write_profile_manifest(&manifest_path, &sample_manifest()).expect("write manifest");

        let rendered = cmd_profile_summarize_for_test(&ProfileSummarizeArgs {
            profile: manifest_path,
            output: None,
            output_format: ProfileSummaryFormat::Markdown,
        })
        .expect("summarize profile");

        assert!(rendered.contains("sample_fns::fibonacci"));
        assert!(rendered.contains("Profile Summary"));
    }

    #[test]
    fn android_backend_builds_capture_plan_with_flamegraph_artifacts() {
        let plan = build_capture_plan(
            &sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::AndroidNative,
                ProfileFormat::Both,
            ),
            &PathBuf::from("target/mobench/profile"),
        )
        .expect("android capture plan");

        assert!(
            plan.raw_artifacts
                .iter()
                .any(|p| p.path.ends_with("sample.perf"))
        );
        assert!(
            plan.processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("stacks.folded"))
        );
        assert!(
            plan.processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("flamegraph.html"))
        );
    }

    #[test]
    fn profile_native_format_excludes_processed_artifacts_from_plan() {
        let plan = build_capture_plan(
            &sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::AndroidNative,
                ProfileFormat::Native,
            ),
            &PathBuf::from("target/mobench/profile"),
        )
        .expect("native-only capture plan");

        assert_eq!(plan.raw_artifacts.len(), 1);
        assert!(plan.processed_artifacts.is_empty());
        assert_eq!(
            plan.viewer_hint.as_deref(),
            Some("Inspect artifacts/raw/sample.perf with the Android profiling toolchain")
        );
    }

    #[test]
    fn ios_backend_builds_capture_plan_with_sample_artifacts() {
        let plan = build_capture_plan(
            &sample_run_args(
                MobileTarget::Ios,
                ProfileProvider::Local,
                ProfileBackend::IosInstruments,
                ProfileFormat::Both,
            ),
            &PathBuf::from("target/mobench/profile"),
        )
        .expect("ios capture plan");

        assert!(
            plan.raw_artifacts
                .iter()
                .any(|p| p.path.ends_with("sample.txt"))
        );
        assert!(
            plan.processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("stacks.folded"))
        );
        assert!(
            plan.processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("flamegraph.html"))
        );
    }

    #[test]
    fn browserstack_profile_run_reports_unsupported_native_capture() {
        let args = sample_run_args(
            MobileTarget::Android,
            ProfileProvider::Browserstack,
            ProfileBackend::AndroidNative,
            ProfileFormat::Both,
        );
        let target = resolve_profile_target(&args).expect("resolve target");
        let mut manifest =
            build_capture_plan(&args, &PathBuf::from("target/mobench/profile")).expect("plan");
        let error = execute_capture(&args, &target, &mut manifest).unwrap_err();

        assert!(error.to_string().contains("BrowserStack"));
        assert!(
            error.to_string().contains("unsupported")
                || error.to_string().contains("not implemented")
        );
    }

    #[test]
    fn browserstack_native_profile_error_is_actionable() {
        let args = sample_run_args(
            MobileTarget::Ios,
            ProfileProvider::Browserstack,
            ProfileBackend::IosInstruments,
            ProfileFormat::Both,
        );
        let target = resolve_profile_target(&args).expect("resolve target");
        let mut manifest =
            build_capture_plan(&args, &PathBuf::from("target/mobench/profile")).expect("plan");
        let error = execute_capture(&args, &target, &mut manifest).unwrap_err();

        let message = error.to_string();
        assert!(
            message.contains("BrowserStack native profiling is not implemented"),
            "expected an explicit unsupported message, got: {message}"
        );
        assert!(
            message.contains("local-first profile contract")
                || message.contains("planned artifact contract only"),
            "expected the error to explain that profile run only records planned artifacts today, got: {message}"
        );
        assert!(
            message.contains("Use --provider local"),
            "expected the error to tell the user what to do instead, got: {message}"
        );
        assert!(
            message.contains("Instruments")
                || message.contains("sample.txt")
                || message.contains("flamegraph"),
            "expected the error to clarify the iOS artifact story, got: {message}"
        );
    }

    #[test]
    fn profile_summary_renders_semantic_phases_separately_from_flamegraph_artifacts() {
        let markdown = render_profile_markdown(&sample_manifest());

        assert!(
            markdown.contains("Semantic phases"),
            "expected semantic phases to be rendered separately, got:\n{markdown}"
        );
        assert!(
            markdown.contains("prove"),
            "expected semantic phase names to be visible, got:\n{markdown}"
        );
        assert!(
            markdown.contains("serialize"),
            "expected semantic phase names to be visible, got:\n{markdown}"
        );
    }

    #[test]
    fn profile_rust_tracing_processed_only_is_rejected() {
        let error = build_capture_plan(
            &sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::RustTracing,
                ProfileFormat::Processed,
            ),
            &PathBuf::from("target/mobench/profile"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("processed"));
        assert!(error.to_string().contains("rust-tracing"));
    }

    #[test]
    fn profile_run_writes_run_scoped_and_latest_manifest_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut android_args = sample_run_args(
            MobileTarget::Android,
            ProfileProvider::Local,
            ProfileBackend::AndroidNative,
            ProfileFormat::Both,
        );
        android_args.output_dir = dir.path().to_path_buf();
        let mut ios_args = sample_run_args(
            MobileTarget::Ios,
            ProfileProvider::Local,
            ProfileBackend::IosInstruments,
            ProfileFormat::Both,
        );
        ios_args.function = "sample_fns::checksum".into();
        ios_args.output_dir = dir.path().to_path_buf();

        cmd_profile_run(&android_args, true).expect("write first planned profile session");
        cmd_profile_run(&ios_args, true).expect("write second planned profile session");

        let android_run_dir = dir.path().join("android-sample_fns--fibonacci");
        let ios_run_dir = dir.path().join("ios-sample_fns--checksum");

        assert!(android_run_dir.join("profile.json").exists());
        assert!(android_run_dir.join("summary.md").exists());
        assert!(ios_run_dir.join("profile.json").exists());
        assert!(ios_run_dir.join("summary.md").exists());
        assert!(dir.path().join("profile.json").exists());
        assert!(dir.path().join("summary.md").exists());

        let latest_manifest =
            load_profile_manifest(&dir.path().join("profile.json")).expect("load latest manifest");
        assert_eq!(latest_manifest.target, MobileTarget::Ios);
        assert_eq!(latest_manifest.function, "sample_fns::checksum");
    }

    #[test]
    fn profile_manifest_serializes_provider() {
        let manifest = build_capture_plan(
            &sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Browserstack,
                ProfileBackend::RustTracing,
                ProfileFormat::Both,
            ),
            &PathBuf::from("target/mobench/profile"),
        )
        .expect("build manifest");

        let json = serde_json::to_value(&manifest).expect("serialize manifest");
        assert_eq!(json["provider"], "browserstack");
    }

    #[test]
    fn resolve_profile_target_accepts_direct_ios_browserstack_device_request() {
        let mut args = sample_run_args(
            MobileTarget::Ios,
            ProfileProvider::Browserstack,
            ProfileBackend::IosInstruments,
            ProfileFormat::Both,
        );
        args.device = Some("iPhone 14".into());
        args.os_version = Some("16".into());

        let target = resolve_profile_target(&args).expect("resolve direct device");
        let device = target.device.expect("device");

        assert_eq!(device.identifier, "iPhone 14-16");
        assert_eq!(device.name, "iPhone 14");
        assert_eq!(device.os_version, "16");
        assert_eq!(device.source, "direct");
    }

    #[test]
    fn profile_dry_run_always_stays_planned() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut args = sample_run_args(
            MobileTarget::Ios,
            ProfileProvider::Browserstack,
            ProfileBackend::IosInstruments,
            ProfileFormat::Both,
        );
        args.output_dir = dir.path().to_path_buf();
        args.device = Some("iPhone 14".into());
        args.os_version = Some("16".into());

        cmd_profile_run(&args, true).expect("dry-run should stop after planning");

        let manifest = load_profile_manifest(
            &dir.path()
                .join("ios-sample_fns--fibonacci")
                .join("profile.json"),
        )
        .expect("load planned manifest");
        assert_eq!(manifest.capture_status, CaptureStatus::Planned);
        assert!(
            manifest
                .warnings
                .iter()
                .any(|warning| warning.contains("dry-run enabled")),
            "expected dry-run warning in manifest: {:?}",
            manifest.warnings
        );
    }

    #[test]
    fn unsupported_browserstack_capture_fails_before_writing_fake_artifacts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut args = sample_run_args(
            MobileTarget::Ios,
            ProfileProvider::Browserstack,
            ProfileBackend::IosInstruments,
            ProfileFormat::Both,
        );
        args.output_dir = dir.path().to_path_buf();
        args.device = Some("iPhone 14".into());
        args.os_version = Some("16".into());

        let error = cmd_profile_run(&args, false).unwrap_err();

        assert!(error.to_string().contains("BrowserStack native profiling"));
        assert!(
            !dir.path()
                .join("ios-sample_fns--fibonacci")
                .join("profile.json")
                .exists(),
            "unsupported execution should not write fake captured artifacts"
        );
    }

    fn sample_manifest() -> ProfileManifest {
        ProfileManifest {
            run_id: "run-123".into(),
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::AndroidNative,
            format: ProfileFormat::Both,
            capture_status: CaptureStatus::Partial,
            raw_artifacts: vec![ArtifactRecord {
                label: "simpleperf".into(),
                path: PathBuf::from("artifacts/raw/sample.perf"),
            }],
            processed_artifacts: vec![ArtifactRecord {
                label: "flamegraph".into(),
                path: PathBuf::from("artifacts/processed/flamegraph.html"),
            }],
            warnings: vec!["missing symbols".into()],
            viewer_hint: Some("Open flamegraph.html in a browser".into()),
        }
    }
}
