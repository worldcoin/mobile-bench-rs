use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::MobileTarget;

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
    #[arg(long, help = "Optional path to config file")]
    pub config: Option<PathBuf>,
    #[arg(long, default_value = "target/mobench/profile")]
    pub output_dir: PathBuf,
    #[arg(long, value_enum, default_value_t = ProfileProvider::Local)]
    pub provider: ProfileProvider,
    #[arg(long, value_enum, default_value_t = ProfileBackend::Auto)]
    pub backend: ProfileBackend,
    #[arg(long, value_enum, default_value_t = ProfileFormat::Both)]
    pub format: ProfileFormat,
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
    pub symbolization: Option<SymbolizationRecord>,
    pub warnings: Vec<String>,
    pub viewer_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolizationRecord {
    pub status: CaptureStatus,
    pub tool: String,
    pub resolved: usize,
    pub unresolved: usize,
    pub notes: Vec<String>,
}

fn default_profile_provider() -> ProfileProvider {
    ProfileProvider::Local
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
    if let Some(symbolization) = &manifest.symbolization {
        let _ = writeln!(markdown);
        let _ = writeln!(markdown, "## Symbolization");
        let _ = writeln!(markdown);
        let _ = writeln!(
            markdown,
            "- Status: `{}`",
            capture_status_label(symbolization.status)
        );
        let _ = writeln!(markdown, "- Tool: `{}`", symbolization.tool);
        let _ = writeln!(markdown, "- Resolved frames: `{}`", symbolization.resolved);
        let _ = writeln!(
            markdown,
            "- Unresolved frames: `{}`",
            symbolization.unresolved
        );
        if !symbolization.notes.is_empty() {
            let _ = writeln!(markdown, "- Notes:");
            for note in &symbolization.notes {
                let _ = writeln!(markdown, "  - {}", note);
            }
        }
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

pub fn cmd_profile_run(args: &ProfileRunArgs) -> Result<()> {
    run_profile_session(args, execute_capture)
}

fn run_profile_session<E>(args: &ProfileRunArgs, execute: E) -> Result<()>
where
    E: FnOnce(&ProfileRunArgs, &Path, &mut ProfileManifest) -> Result<()>,
{
    validate_profile_request(args)?;
    std::fs::create_dir_all(&args.output_dir)?;
    let run_id = allocate_run_id(&args.output_dir, args.target, &args.function)?;
    let run_output_dir = args.output_dir.join(&run_id);
    std::fs::create_dir_all(&run_output_dir)?;

    let mut manifest = build_capture_plan(args, &run_output_dir, &run_id)?;
    if let Err(error) = execute(args, &run_output_dir, &mut manifest) {
        write_profile_artifacts(&run_output_dir, &args.output_dir, &manifest)?;
        return Err(error);
    }
    write_profile_artifacts(&run_output_dir, &args.output_dir, &manifest)?;
    Ok(())
}

fn write_profile_artifacts(
    run_output_dir: &Path,
    latest_output_dir: &Path,
    manifest: &ProfileManifest,
) -> Result<()> {
    let rendered_summary = render_profile_markdown(&manifest);

    let run_profile_path = run_output_dir.join("profile.json");
    let run_summary_path = run_output_dir.join("summary.md");
    write_profile_manifest(&run_profile_path, &manifest)?;
    std::fs::write(&run_summary_path, rendered_summary.as_bytes())?;

    let latest_profile_path = latest_output_dir.join("profile.json");
    let latest_summary_path = latest_output_dir.join("summary.md");
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

fn validate_profile_request(args: &ProfileRunArgs) -> Result<()> {
    let layout = crate::resolve_project_layout(crate::ProjectLayoutOptions {
        start_dir: None,
        project_root: None,
        crate_path: args.crate_path.as_deref(),
        config_path: args.config.as_deref(),
    })
    .context("failed to resolve benchmark layout for profile run")?;
    let benchmarks = crate::discover_benchmarks_for_layout(&layout)
        .context("failed to discover benchmarks for profile run")?;

    if benchmarks.is_empty() {
        bail!(
            "no benchmark functions found in {}; add #[benchmark] functions before profiling",
            layout.crate_dir.display()
        );
    }

    if !benchmarks
        .iter()
        .any(|candidate| candidate == &args.function)
    {
        bail!(
            "benchmark `{}` was not found in {}. Available benchmarks: {}",
            args.function,
            layout.crate_dir.display(),
            benchmarks.join(", ")
        );
    }

    Ok(())
}

fn build_capture_plan(
    args: &ProfileRunArgs,
    output_root: &Path,
    run_id: &str,
) -> Result<ProfileManifest> {
    let backend = resolve_backend(args.target, args.backend);
    validate_profile_capabilities(args.provider, backend)?;
    validate_format_capabilities(backend, args.format)?;

    let raw_root = output_root.join("artifacts/raw");
    let processed_root = output_root.join("artifacts/processed");

    let (raw_artifacts, processed_artifacts) = match backend {
        ProfileBackend::AndroidNative => (
            vec![ArtifactRecord {
                label: "simpleperf".into(),
                path: raw_root.join("sample.perf"),
            }],
            vec![ArtifactRecord {
                label: "flamegraph".into(),
                path: processed_root.join("flamegraph.html"),
            }],
        ),
        ProfileBackend::IosInstruments => (
            vec![ArtifactRecord {
                label: "time-profiler".into(),
                path: raw_root.join("time-profiler.trace"),
            }],
            vec![ArtifactRecord {
                label: "xctrace-export".into(),
                path: processed_root.join("time-profiler.xml"),
            }],
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
    ensure_selected_artifact_roots(&raw_artifacts, &processed_artifacts)?;
    let viewer_hint =
        select_viewer_hint(backend, args.format, &raw_artifacts, &processed_artifacts);

    Ok(ProfileManifest {
        run_id: run_id.to_string(),
        target: args.target,
        function: args.function.clone(),
        provider: args.provider,
        backend,
        format: args.format,
        capture_status: CaptureStatus::Planned,
        raw_artifacts,
        processed_artifacts,
        symbolization: Some(SymbolizationRecord {
            status: CaptureStatus::Planned,
            tool: "llvm-addr2line".into(),
            resolved: 0,
            unresolved: 0,
            notes: vec!["symbolization has not been attempted yet".into()],
        }),
        warnings: vec![
            "capture execution is not implemented yet; this session records the planned artifact contract only"
                .into(),
        ],
        viewer_hint,
    })
}

trait CommandRunner {
    fn output(&mut self, command: &mut Command) -> Result<Output>;
}

struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn output(&mut self, command: &mut Command) -> Result<Output> {
        command.output().context("running external command")
    }
}

fn execute_capture(
    args: &ProfileRunArgs,
    output_root: &Path,
    manifest: &mut ProfileManifest,
) -> Result<()> {
    if args.provider != ProfileProvider::Local || manifest.backend != ProfileBackend::AndroidNative {
        return Ok(());
    }

    let layout = crate::resolve_project_layout(crate::ProjectLayoutOptions {
        start_dir: None,
        project_root: None,
        crate_path: args.crate_path.as_deref(),
        config_path: args.config.as_deref(),
    })
    .context("failed to resolve benchmark layout for android profile execution")?;

    let mut runner = RealCommandRunner;
    execute_local_android_capture_with_runner(
        args,
        output_root,
        manifest,
        &layout,
        |layout| {
            let ndk_home = std::env::var("ANDROID_NDK_HOME")
                .context("ANDROID_NDK_HOME is required for local Android profiling")?;
            crate::run_android_build(layout, &ndk_home, true, false)
                .context("building Android artifacts for local profiling")
        },
        &mut runner,
    )
}

fn execute_local_android_capture_with_runner<R, F>(
    args: &ProfileRunArgs,
    output_root: &Path,
    manifest: &mut ProfileManifest,
    layout: &crate::ResolvedProjectLayout,
    build_fn: F,
    runner: &mut R,
) -> Result<()>
where
    R: CommandRunner,
    F: FnOnce(&crate::ResolvedProjectLayout) -> Result<mobench_sdk::BuildResult>,
{
    let build = build_fn(layout)?;

    let raw_root = output_root.join("artifacts/raw");
    let processed_root = output_root.join("artifacts/processed");
    std::fs::create_dir_all(&raw_root)?;
    std::fs::create_dir_all(&processed_root)?;
    manifest
        .warnings
        .retain(|warning| !warning.contains("capture execution is not implemented yet"));

    let raw_perf_path = manifest
        .raw_artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with("sample.perf"))
        .map(|artifact| artifact.path.clone())
        .context("local Android profile plan did not include sample.perf")?;
    let folded_path = manifest
        .processed_artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with("stacks.folded"))
        .map(|artifact| artifact.path.clone())
        .unwrap_or_else(|| processed_root.join("stacks.folded"));
    let native_report_path = manifest
        .processed_artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with("native-report.txt"))
        .map(|artifact| artifact.path.clone())
        .unwrap_or_else(|| processed_root.join("native-report.txt"));
    let flamegraph_path = manifest
        .processed_artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with("flamegraph.html"))
        .map(|artifact| artifact.path.clone())
        .unwrap_or_else(|| processed_root.join("flamegraph.html"));

    let package_name = "dev.world.bench";
    let activity_name = "dev.world.bench/.MainActivity";

    let install_output = run_adb_command(runner, ["install", "-r"], &build.app_path, None)?;
    if !install_output.status.success() {
        let stderr = String::from_utf8_lossy(&install_output.stderr);
        manifest.capture_status = CaptureStatus::Failed;
        manifest.symbolization = Some(SymbolizationRecord {
            status: CaptureStatus::Failed,
            tool: "llvm-addr2line".into(),
            resolved: 0,
            unresolved: 0,
            notes: vec![format!("adb install failed: {}", stderr.trim())],
        });
        manifest.warnings.push(format!("adb install failed: {}", stderr.trim()));
        bail!("adb install failed for {}: {}", build.app_path.display(), stderr.trim());
    }

    let mut record_command = Command::new("simpleperf");
    record_command
        .arg("record")
        .arg("--app")
        .arg(package_name)
        .arg("--call-graph")
        .arg("fp")
        .arg("--duration")
        .arg("15")
        .arg("--out")
        .arg("/data/local/tmp/sample.perf")
        .arg("--")
        .arg("am")
        .arg("start")
        .arg("-n")
        .arg(activity_name)
        .arg("--es")
        .arg("bench_function")
        .arg(&args.function);

    let record_output = runner.output(&mut record_command)?;
    if !record_output.status.success() {
        let stderr = String::from_utf8_lossy(&record_output.stderr);
        manifest.capture_status = CaptureStatus::Failed;
        manifest.symbolization = Some(SymbolizationRecord {
            status: CaptureStatus::Failed,
            tool: "llvm-addr2line".into(),
            resolved: 0,
            unresolved: 0,
            notes: vec![format!("simpleperf record failed: {}", stderr.trim())],
        });
        manifest
            .warnings
            .push(format!("simpleperf record failed: {}", stderr.trim()));
        bail!("simpleperf capture failed: {}", stderr.trim());
    }

    let mut pull_command = Command::new("adb");
    pull_command.args(["pull", "/data/local/tmp/sample.perf"]);
    pull_command.arg(&raw_perf_path);
    let pull_output = runner.output(&mut pull_command)?;
    if !pull_output.status.success() {
        let stderr = String::from_utf8_lossy(&pull_output.stderr);
        manifest.capture_status = CaptureStatus::Failed;
        manifest.symbolization = Some(SymbolizationRecord {
            status: CaptureStatus::Failed,
            tool: "llvm-addr2line".into(),
            resolved: 0,
            unresolved: 0,
            notes: vec![format!("adb pull failed: {}", stderr.trim())],
        });
        manifest.warnings.push(format!("adb pull failed: {}", stderr.trim()));
        bail!("failed to pull simpleperf capture: {}", stderr.trim());
    }

    let mut report_command = Command::new("simpleperf");
    report_command.arg("report").arg("-i").arg(&raw_perf_path);
    let report_output = runner.output(&mut report_command)?;
    if !report_output.status.success() {
        let stderr = String::from_utf8_lossy(&report_output.stderr);
        manifest.capture_status = CaptureStatus::Failed;
        manifest.symbolization = Some(SymbolizationRecord {
            status: CaptureStatus::Failed,
            tool: "llvm-addr2line".into(),
            resolved: 0,
            unresolved: 0,
            notes: vec![format!("simpleperf report failed: {}", stderr.trim())],
        });
        manifest
            .warnings
            .push(format!("simpleperf report failed: {}", stderr.trim()));
        bail!("simpleperf report failed: {}", stderr.trim());
    }

    let report_text = String::from_utf8_lossy(&report_output.stdout).to_string();
    let addr2line_path = std::env::var("LLVM_ADDR2LINE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("llvm-addr2line"));
    let (symbolized_report, symbolization) = symbolize_android_capture_report(
        &report_text,
        &addr2line_path,
        &build.native_libraries,
    )?;

    std::fs::write(&native_report_path, symbolized_report.as_bytes())?;
    std::fs::write(&folded_path, symbolized_report.as_bytes())?;
    std::fs::write(
        &flamegraph_path,
        render_android_flamegraph_html(&symbolized_report, &manifest.run_id).as_bytes(),
    )?;

    manifest.capture_status = if symbolization.unresolved == 0 {
        CaptureStatus::Captured
    } else {
        CaptureStatus::Partial
    };
    manifest
        .warnings
        .retain(|warning| !warning.contains("capture execution is not implemented yet"));
    manifest.symbolization = Some(SymbolizationRecord {
        status: manifest.capture_status,
        tool: addr2line_path.display().to_string(),
        resolved: symbolization.resolved,
        unresolved: symbolization.unresolved,
        notes: if symbolization.unresolved == 0 {
            vec!["Android native profiling capture completed and symbols resolved".into()]
        } else {
            vec![
                "Android native profiling capture completed with unresolved native frames".into(),
            ]
        },
    });
    if symbolization.unresolved > 0 {
        manifest.warnings.push(format!(
            "Android native profiling completed with {} unresolved native frames",
            symbolization.unresolved
        ));
    }

    Ok(())
}

fn run_adb_command<R, I, S>(
    runner: &mut R,
    args: I,
    artifact: &Path,
    extra: Option<&str>,
) -> Result<Output>
where
    R: CommandRunner,
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut command = Command::new("adb");
    command.args(args);
    if let Some(extra) = extra {
        command.arg(extra);
    }
    command.arg(artifact);
    runner.output(&mut command)
}

fn symbolize_android_capture_report(
    report_text: &str,
    addr2line_path: &Path,
    libraries: &[mobench_sdk::NativeLibraryArtifact],
) -> Result<(String, AndroidSymbolizationTotals)> {
    let mut output = String::new();
    let mut totals = AndroidSymbolizationTotals::default();

    for line in report_text.lines() {
        let (symbolized_line, stats) =
            mobench_sdk::builders::android::symbolize_android_native_stack_line(
                line,
                addr2line_path,
                libraries,
            )?;
        totals.resolved += stats.resolved;
        totals.unresolved += stats.unresolved;
        output.push_str(&symbolized_line);
        output.push('\n');
    }

    Ok((output, totals))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct AndroidSymbolizationTotals {
    resolved: usize,
    unresolved: usize,
}

fn render_android_flamegraph_html(report_text: &str, run_id: &str) -> String {
    let escaped = report_text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Android profile {run_id}</title><style>body{{font-family:ui-monospace,monospace;white-space:pre-wrap;padding:24px;}}pre{{background:#111827;color:#f9fafb;padding:16px;border-radius:12px;overflow:auto;}}</style></head><body><h1>Android native profile {run_id}</h1><p>Symbolized native report</p><pre>{escaped}</pre></body></html>"
    )
}

fn allocate_run_id(output_dir: &Path, target: MobileTarget, function: &str) -> Result<String> {
    let prefix = build_run_id(target, function);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    for suffix in 0..1000 {
        let candidate = if suffix == 0 {
            format!("{prefix}-{timestamp}")
        } else {
            format!("{prefix}-{timestamp}-{suffix}")
        };

        if !output_dir.join(&candidate).exists() {
            return Ok(candidate);
        }
    }

    bail!(
        "failed to allocate a unique profile run id for `{}` after 1000 attempts",
        function
    )
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

fn validate_profile_capabilities(provider: ProfileProvider, backend: ProfileBackend) -> Result<()> {
    if provider == ProfileProvider::Browserstack
        && matches!(
            backend,
            ProfileBackend::AndroidNative | ProfileBackend::IosInstruments
        )
    {
        bail!(
            "BrowserStack native profile capture is unsupported for the MVP; use local provider for {:?}",
            backend
        );
    }
    Ok(())
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

fn ensure_selected_artifact_roots(
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

fn select_viewer_hint(
    backend: ProfileBackend,
    format: ProfileFormat,
    raw_artifacts: &[ArtifactRecord],
    processed_artifacts: &[ArtifactRecord],
) -> Option<String> {
    match backend {
        ProfileBackend::AndroidNative => {
            if format != ProfileFormat::Native && !processed_artifacts.is_empty() {
                Some("Open artifacts/processed/flamegraph.html in a browser".into())
            } else if !raw_artifacts.is_empty() {
                Some(
                    "Inspect artifacts/raw/sample.perf with the Android profiling toolchain".into(),
                )
            } else {
                None
            }
        }
        ProfileBackend::IosInstruments => {
            if !raw_artifacts.is_empty() {
                Some("Open artifacts/raw/time-profiler.trace in Instruments".into())
            } else if !processed_artifacts.is_empty() {
                Some(
                    "Inspect artifacts/processed/time-profiler.xml or rerun with --format both to keep the .trace bundle"
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
    use std::process::{Command, Output};

    #[test]
    fn profile_manifest_serializes_partial_failure_state() {
        let manifest = sample_manifest();

        let json = serde_json::to_value(&manifest).expect("serialize manifest");
        assert_eq!(json["warnings"][0], "missing symbols");
        assert_eq!(json["capture_status"], "partial");
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
    fn android_native_offsets_are_symbolized_into_rust_frames() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let addr2line_shim = temp_dir.path().join("llvm-addr2line");
        std::fs::write(
            &addr2line_shim,
            b"#!/bin/sh\nprintf 'sample_fns::fibonacci\\n/opt/sample_fns.rs:123\\n'\n",
        )
        .expect("write addr2line shim");
        make_executable(&addr2line_shim);

        let native_lib = temp_dir.path().join("libsample_fns.so");
        std::fs::write(&native_lib, b"so").expect("write library");
        let artifact = mobench_sdk::NativeLibraryArtifact {
            abi: "arm64-v8a".into(),
            packaged_path: native_lib.clone(),
            unstripped_path: native_lib,
        };

        let (rendered, totals) = symbolize_android_capture_report(
            "libsample_fns.so[+0x1a2b]\n",
            &addr2line_shim,
            &[artifact],
        )
        .expect("symbolize capture report");

        assert!(rendered.contains("sample_fns::fibonacci"));
        assert_eq!(totals.resolved, 1);
        assert_eq!(totals.unresolved, 0);
    }

    #[test]
    fn local_android_capture_attempts_symbolization_and_updates_manifest_status() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let output_root = temp_dir.path().join("profile");
        let run_id = "android-ffi_benchmark--bench_fibonacci-123";
        let args = ProfileRunArgs {
            target: MobileTarget::Android,
            function: "ffi_benchmark::bench_fibonacci".into(),
            crate_path: Some(workspace_root().join("examples/ffi-benchmark")),
            config: None,
            output_dir: output_root.clone(),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::AndroidNative,
            format: ProfileFormat::Both,
        };
        let mut manifest = build_capture_plan(
            &args,
            &output_root.join(run_id),
            run_id,
        )
        .expect("build capture plan");

        let layout = crate::resolve_project_layout(crate::ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: args.crate_path.as_deref(),
            config_path: args.config.as_deref(),
        })
        .expect("resolve layout");

        let app_path = temp_dir.path().join("app.apk");
        std::fs::write(&app_path, b"apk").expect("write app artifact");
        let packaged_lib = temp_dir.path().join("libsample_fns.so");
        let unstripped_lib = temp_dir.path().join("libsample_fns.so.unstripped");
        std::fs::write(&packaged_lib, b"so").expect("write packaged lib");
        std::fs::write(&unstripped_lib, b"so").expect("write unstripped lib");
        let addr2line_shim = temp_dir.path().join("llvm-addr2line");
        std::fs::write(
            &addr2line_shim,
            b"#!/bin/sh\nprintf 'sample_fns::fibonacci\\n/opt/sample_fns.rs:123\\n'\n",
        )
        .expect("write addr2line shim");
        make_executable(&addr2line_shim);

        unsafe {
            std::env::set_var("LLVM_ADDR2LINE", &addr2line_shim);
        }

        let build_result = mobench_sdk::BuildResult {
            platform: mobench_sdk::Target::Android,
            app_path: app_path.clone(),
            test_suite_path: Some(temp_dir.path().join("androidTest.apk")),
            native_libraries: vec![mobench_sdk::NativeLibraryArtifact {
                abi: "arm64-v8a".into(),
                packaged_path: packaged_lib.clone(),
                unstripped_path: unstripped_lib.clone(),
            }],
        };

        let mut runner = RecordingRunner {
            outputs: vec![
                success_output(""),
                success_output(""),
                success_output(""),
                success_output("libsample_fns.so[+0x1a2b]\n"),
            ],
            commands: Vec::new(),
        };

        execute_local_android_capture_with_runner(
            &args,
            &output_root.join(run_id),
            &mut manifest,
            &layout,
            |_| Ok(build_result.clone()),
            &mut runner,
        )
        .expect("android capture attempt");

        assert_eq!(manifest.capture_status, CaptureStatus::Captured);
        let symbolization = manifest.symbolization.as_ref().expect("symbolization record");
        assert_eq!(symbolization.status, CaptureStatus::Captured);
        assert_eq!(symbolization.resolved, 1);
        assert_eq!(symbolization.unresolved, 0);
        assert!(
            manifest
                .warnings
                .iter()
                .all(|warning| !warning.contains("capture execution is not implemented yet"))
        );
        assert!(output_root.join(run_id).join("artifacts/raw/sample.perf").exists());
        assert!(output_root
            .join(run_id)
            .join("artifacts/processed/native-report.txt")
            .exists());
        assert!(output_root
            .join(run_id)
            .join("artifacts/processed/stacks.folded")
            .exists());
        assert!(output_root
            .join(run_id)
            .join("artifacts/processed/flamegraph.html")
            .exists());
        assert!(runner
            .commands
            .iter()
            .any(|command| command.contains("simpleperf record")));
        assert!(runner
            .commands
            .iter()
            .any(|command| command.contains("simpleperf report")));

        unsafe {
            std::env::remove_var("LLVM_ADDR2LINE");
        }
    }

    #[test]
    fn profile_session_writes_failed_android_manifest_after_attempted_execution() {
        let dir = tempfile::tempdir().expect("temp dir");
        let args = ProfileRunArgs {
            target: MobileTarget::Android,
            function: "ffi_benchmark::bench_fibonacci".into(),
            crate_path: Some(workspace_root().join("examples/ffi-benchmark")),
            config: None,
            output_dir: dir.path().to_path_buf(),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::AndroidNative,
            format: ProfileFormat::Both,
        };

        let error = run_profile_session(&args, |_, _, manifest| {
            manifest.capture_status = CaptureStatus::Failed;
            manifest.symbolization = Some(SymbolizationRecord {
                status: CaptureStatus::Failed,
                tool: "llvm-addr2line".into(),
                resolved: 0,
                unresolved: 0,
                notes: vec!["simulated Android capture failure".into()],
            });
            anyhow::bail!("simulated Android capture failure");
        })
        .expect_err("simulated execution failure should bubble up");

        assert!(error
            .to_string()
            .contains("simulated Android capture failure"));

        let run_dir = find_single_run_dir(
            &dir.path().to_path_buf(),
            "android-ffi_benchmark--bench_fibonacci",
        );
        let manifest = load_profile_manifest(&run_dir.join("profile.json"))
            .expect("load failed profile manifest");
        assert_eq!(manifest.capture_status, CaptureStatus::Failed);
        assert_eq!(
            manifest
                .symbolization
                .as_ref()
                .expect("symbolization record")
                .status,
            CaptureStatus::Failed
        );
        assert!(run_dir.join("summary.md").exists());
        assert!(dir.path().join("profile.json").exists());
        assert!(dir.path().join("summary.md").exists());
    }

    #[test]
    fn android_backend_builds_capture_plan_with_flamegraph_artifacts() {
        let plan = build_capture_plan(
            &ProfileRunArgs {
                target: MobileTarget::Android,
                function: "sample_fns::fibonacci".into(),
                crate_path: None,
                config: None,
                output_dir: PathBuf::from("target/mobench/profile"),
                provider: ProfileProvider::Local,
                backend: ProfileBackend::AndroidNative,
                format: ProfileFormat::Both,
            },
            &PathBuf::from("target/mobench/profile"),
            "android-sample_fns--fibonacci",
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
                .any(|p| p.path.ends_with("flamegraph.html"))
        );
    }

    #[test]
    fn profile_native_format_excludes_processed_artifacts_from_plan() {
        let plan = build_capture_plan(
            &ProfileRunArgs {
                target: MobileTarget::Android,
                function: "sample_fns::fibonacci".into(),
                crate_path: None,
                config: None,
                output_dir: PathBuf::from("target/mobench/profile"),
                provider: ProfileProvider::Local,
                backend: ProfileBackend::AndroidNative,
                format: ProfileFormat::Native,
            },
            &PathBuf::from("target/mobench/profile"),
            "android-sample_fns--fibonacci",
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
    fn ios_backend_allocates_trace_bundle_and_export_paths() {
        let plan = build_capture_plan(
            &ProfileRunArgs {
                target: MobileTarget::Ios,
                function: "sample_fns::fibonacci".into(),
                crate_path: None,
                config: None,
                output_dir: PathBuf::from("target/mobench/profile"),
                provider: ProfileProvider::Local,
                backend: ProfileBackend::IosInstruments,
                format: ProfileFormat::Both,
            },
            &PathBuf::from("target/mobench/profile"),
            "ios-sample_fns--fibonacci",
        )
        .expect("ios capture plan");

        assert!(
            plan.raw_artifacts
                .iter()
                .any(|p| p.path.ends_with("time-profiler.trace"))
        );
        assert!(
            plan.processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("time-profiler.xml"))
        );
    }

    #[test]
    fn browserstack_profile_run_reports_unsupported_native_capture() {
        let error = build_capture_plan(
            &ProfileRunArgs {
                target: MobileTarget::Android,
                function: "sample_fns::fibonacci".into(),
                crate_path: None,
                config: None,
                output_dir: PathBuf::from("target/mobench/profile"),
                provider: ProfileProvider::Browserstack,
                backend: ProfileBackend::AndroidNative,
                format: ProfileFormat::Both,
            },
            &PathBuf::from("target/mobench/profile"),
            "android-sample_fns--fibonacci",
        )
        .unwrap_err();

        assert!(error.to_string().contains("BrowserStack"));
        assert!(error.to_string().contains("unsupported"));
    }

    #[test]
    fn profile_rust_tracing_processed_only_is_rejected() {
        let error = build_capture_plan(
            &ProfileRunArgs {
                target: MobileTarget::Android,
                function: "sample_fns::fibonacci".into(),
                crate_path: None,
                config: None,
                output_dir: PathBuf::from("target/mobench/profile"),
                provider: ProfileProvider::Local,
                backend: ProfileBackend::RustTracing,
                format: ProfileFormat::Processed,
            },
            &PathBuf::from("target/mobench/profile"),
            "android-sample_fns--fibonacci",
        )
        .unwrap_err();

        assert!(error.to_string().contains("processed"));
        assert!(error.to_string().contains("rust-tracing"));
    }

    #[test]
    fn profile_run_writes_run_scoped_and_latest_manifest_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let crate_path = workspace_root().join("examples/ffi-benchmark");
        let android_args = ProfileRunArgs {
            target: MobileTarget::Android,
            function: "ffi_benchmark::bench_fibonacci".into(),
            crate_path: Some(crate_path.clone()),
            config: None,
            output_dir: dir.path().to_path_buf(),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::RustTracing,
            format: ProfileFormat::Both,
        };
        let ios_args = ProfileRunArgs {
            target: MobileTarget::Ios,
            function: "ffi_benchmark::bench_checksum".into(),
            crate_path: Some(crate_path),
            config: None,
            output_dir: dir.path().to_path_buf(),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::IosInstruments,
            format: ProfileFormat::Both,
        };

        cmd_profile_run(&android_args).expect("write first planned profile session");
        cmd_profile_run(&ios_args).expect("write second planned profile session");

        let android_run_dir = find_single_run_dir(
            &dir.path().to_path_buf(),
            "android-ffi_benchmark--bench_fibonacci",
        );
        let ios_run_dir = find_single_run_dir(
            &dir.path().to_path_buf(),
            "ios-ffi_benchmark--bench_checksum",
        );

        assert!(android_run_dir.join("profile.json").exists());
        assert!(android_run_dir.join("summary.md").exists());
        assert!(ios_run_dir.join("profile.json").exists());
        assert!(ios_run_dir.join("summary.md").exists());
        assert!(dir.path().join("profile.json").exists());
        assert!(dir.path().join("summary.md").exists());

        let latest_manifest =
            load_profile_manifest(&dir.path().join("profile.json")).expect("load latest manifest");
        assert_eq!(latest_manifest.target, MobileTarget::Ios);
        assert_eq!(latest_manifest.function, "ffi_benchmark::bench_checksum");
        assert_eq!(
            latest_manifest.run_id,
            ios_run_dir
                .file_name()
                .expect("ios run dir name")
                .to_string_lossy()
        );
    }

    #[test]
    fn profile_run_rejects_unknown_benchmark_function() {
        let dir = tempfile::tempdir().expect("temp dir");
        let error = cmd_profile_run(&ProfileRunArgs {
            target: MobileTarget::Android,
            function: "does_not_exist::bench".into(),
            crate_path: Some(workspace_root().join("examples/ffi-benchmark")),
            config: None,
            output_dir: dir.path().join("profile"),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::AndroidNative,
            format: ProfileFormat::Both,
        })
        .expect_err("invalid benchmark selector should fail");

        assert!(error.to_string().contains("does_not_exist::bench"));
        assert!(error.to_string().contains("Available benchmarks"));
        assert!(!dir.path().join("profile").exists());
    }

    #[test]
    fn profile_run_preserves_history_for_repeated_runs() {
        let dir = tempfile::tempdir().expect("temp dir");
        let args = ProfileRunArgs {
            target: MobileTarget::Android,
            function: "ffi_benchmark::bench_fibonacci".into(),
            crate_path: Some(workspace_root().join("examples/ffi-benchmark")),
            config: None,
            output_dir: dir.path().to_path_buf(),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::RustTracing,
            format: ProfileFormat::Both,
        };

        cmd_profile_run(&args).expect("write first profile session");
        cmd_profile_run(&args).expect("write second profile session");

        let run_dirs = collect_run_dirs(
            &dir.path().to_path_buf(),
            "android-ffi_benchmark--bench_fibonacci",
        );
        assert_eq!(run_dirs.len(), 2);
        assert_ne!(run_dirs[0], run_dirs[1]);
        for run_dir in &run_dirs {
            assert!(run_dir.join("profile.json").exists());
            assert!(run_dir.join("summary.md").exists());
        }

        let latest_manifest =
            load_profile_manifest(&dir.path().join("profile.json")).expect("load latest manifest");
        let latest_run_dir = run_dirs
            .iter()
            .find(|dir| {
                dir.file_name()
                    .is_some_and(|name| name == latest_manifest.run_id.as_str())
            })
            .expect("latest manifest should point at a run-scoped directory");
        assert!(latest_run_dir.join("profile.json").exists());
    }

    #[test]
    fn profile_manifest_serializes_provider() {
        let manifest = build_capture_plan(
            &ProfileRunArgs {
                target: MobileTarget::Android,
                function: "sample_fns::fibonacci".into(),
                crate_path: None,
                config: None,
                output_dir: PathBuf::from("target/mobench/profile"),
                provider: ProfileProvider::Browserstack,
                backend: ProfileBackend::RustTracing,
                format: ProfileFormat::Both,
            },
            &PathBuf::from("target/mobench/profile"),
            "android-sample_fns--fibonacci",
        )
        .expect("build manifest");

        let json = serde_json::to_value(&manifest).expect("serialize manifest");
        assert_eq!(json["provider"], "browserstack");
    }

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn collect_run_dirs(root: &PathBuf, prefix: &str) -> Vec<PathBuf> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(root)
            .expect("read run dir")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_dir())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(prefix))
            })
            .collect();
        entries.sort();
        entries
    }

    fn find_single_run_dir(root: &PathBuf, prefix: &str) -> PathBuf {
        let matches = collect_run_dirs(root, prefix);
        assert_eq!(matches.len(), 1, "expected one run dir for prefix {prefix}");
        matches.into_iter().next().expect("single run dir")
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
            symbolization: Some(SymbolizationRecord {
                status: CaptureStatus::Partial,
                tool: "llvm-addr2line".into(),
                resolved: 1,
                unresolved: 1,
                notes: vec!["missing symbols".into()],
            }),
            warnings: vec!["missing symbols".into()],
            viewer_hint: Some("Open flamegraph.html in a browser".into()),
        }
    }

    fn success_output(stdout: &str) -> Output {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("true")
            .status()
            .expect("create success status");
        Output {
            status,
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path)
                .expect("metadata")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).expect("set perms");
        }
    }

    struct RecordingRunner {
        outputs: Vec<Output>,
        commands: Vec<String>,
    }

    impl CommandRunner for RecordingRunner {
        fn output(&mut self, command: &mut Command) -> Result<Output> {
            let program = command.get_program().to_string_lossy().to_string();
            let args: Vec<String> = command
                .get_args()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect();
            let mut rendered = command.get_program().to_string_lossy().to_string();
            for arg in &args {
                rendered.push(' ');
                rendered.push_str(arg);
            }
            self.commands.push(rendered);

            if program == "adb" && args.first().is_some_and(|arg| arg == "pull") {
                if let Some(destination) = args.last() {
                    std::fs::write(destination, b"simpleperf")
                        .expect("create pulled raw capture artifact");
                }
            }

            Ok(self.outputs.remove(0))
        }
    }
}
