use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{
    DevicePlatform, MobileTarget, ProjectLayoutOptions, ResolvedMatrixDevice, RunSpec,
    flamegraph_viewer::{
        ArtifactLink as ViewerArtifactLink, FlamegraphMode, FlamegraphViewerDoc,
        count_folded_stack_lines, derive_benchmark_focused_folded_stacks,
        render_flamegraph_viewer_html, render_standalone_flamegraph_svg,
        summarize_folded_stacks,
    },
    load_dotenv_for_layout, persist_mobile_spec, resolve_devices_for_profile,
    resolve_project_layout, run_android_build, run_ios_build, validate_benchmark_function,
};
use mobench_sdk::types::NativeLibraryArtifact;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureWarmupMode {
    Cold,
    Warm,
}

impl CaptureWarmupMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
        }
    }
}

#[derive(Debug, Clone, Args)]
#[command(
    about = "Plan or execute a native profiling session; local android-native and ios-instruments now attempt real native capture",
    after_help = concat!(
        "Capability matrix:\n",
        "  local + android-native: attempts real simpleperf capture and symbolization\n",
        "  local + ios-instruments: attempts real simulator-host sample capture and flamegraph generation\n",
        "  local + rust-tracing: planned manifest today; structured trace output is local-only\n",
        "  browserstack + android-native: unsupported for native capture in this release\n",
        "  browserstack + ios-instruments: unsupported for native capture in this release\n",
        "  browserstack + rust-tracing: unsupported; use local provider for trace-events output\n",
        "\n",
        "Device selection:\n",
        "  Use --device/--os-version for one explicit BrowserStack device, or --profile with\n",
        "  optional --device-matrix/--config to reuse the same deterministic resolution model as\n",
        "  `mobench devices resolve`.\n"
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
    #[arg(
        long,
        value_enum,
        help = "Warm or cold capture mode for local native profiling (defaults to warm for local Android/iOS native backends)"
    )]
    pub warmup_mode: Option<CaptureWarmupMode>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SemanticCaptureStatus {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolizationRecord {
    pub status: CaptureStatus,
    pub tool: Option<String>,
    pub resolved_frames: u64,
    pub unresolved_frames: u64,
    pub notes: Vec<String>,
}

impl Default for SymbolizationRecord {
    fn default() -> Self {
        Self {
            status: CaptureStatus::Planned,
            tool: None,
            resolved_frames: 0,
            unresolved_frames: 0,
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeCaptureRecord {
    pub status: CaptureStatus,
    pub raw_artifacts: Vec<ArtifactRecord>,
    pub processed_artifacts: Vec<ArtifactRecord>,
    pub symbolization: SymbolizationRecord,
    pub viewer_hint: Option<String>,
}

impl Default for NativeCaptureRecord {
    fn default() -> Self {
        Self {
            status: CaptureStatus::Planned,
            raw_artifacts: Vec::new(),
            processed_artifacts: Vec::new(),
            symbolization: SymbolizationRecord::default(),
            viewer_hint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticPhaseRecord {
    pub name: String,
    pub duration_ns: Option<u64>,
    pub percent_total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticProfileRecord {
    pub status: SemanticCaptureStatus,
    pub phases: Vec<SemanticPhaseRecord>,
    pub spans_path: Option<PathBuf>,
}

impl Default for SemanticProfileRecord {
    fn default() -> Self {
        Self {
            status: SemanticCaptureStatus::Planned,
            phases: Vec::new(),
            spans_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CaptureMetadataRecord {
    pub device: Option<String>,
    pub sample_duration_secs: Option<u64>,
    pub warmup_mode: Option<CaptureWarmupMode>,
    pub capture_method: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileManifest {
    pub run_id: String,
    pub target: MobileTarget,
    pub function: String,
    #[serde(default = "default_profile_provider")]
    pub provider: ProfileProvider,
    pub backend: ProfileBackend,
    pub format: ProfileFormat,
    pub native_capture: NativeCaptureRecord,
    pub semantic_profile: SemanticProfileRecord,
    pub capture_metadata: CaptureMetadataRecord,
}

#[derive(Debug, Clone, Deserialize)]
struct ProfileManifestSerde {
    run_id: String,
    target: MobileTarget,
    function: String,
    #[serde(default = "default_profile_provider")]
    provider: ProfileProvider,
    backend: ProfileBackend,
    format: ProfileFormat,
    #[serde(default)]
    native_capture: NativeCaptureRecord,
    #[serde(default)]
    semantic_profile: SemanticProfileRecord,
    #[serde(default)]
    capture_metadata: CaptureMetadataRecord,
    #[serde(default)]
    capture_status: Option<CaptureStatus>,
    #[serde(default)]
    raw_artifacts: Vec<ArtifactRecord>,
    #[serde(default)]
    processed_artifacts: Vec<ArtifactRecord>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    viewer_hint: Option<String>,
}

impl<'de> Deserialize<'de> for ProfileManifest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let helper = ProfileManifestSerde::deserialize(deserializer)?;
        Ok(Self::from(helper))
    }
}

impl From<ProfileManifestSerde> for ProfileManifest {
    fn from(mut helper: ProfileManifestSerde) -> Self {
        let has_legacy_native_fields = helper.capture_status.is_some()
            || !helper.raw_artifacts.is_empty()
            || !helper.processed_artifacts.is_empty()
            || helper.viewer_hint.is_some();
        if has_legacy_native_fields && helper.native_capture == NativeCaptureRecord::default() {
            helper.native_capture = NativeCaptureRecord {
                status: helper.capture_status.unwrap_or(CaptureStatus::Planned),
                raw_artifacts: helper.raw_artifacts,
                processed_artifacts: helper.processed_artifacts,
                symbolization: SymbolizationRecord::default(),
                viewer_hint: helper.viewer_hint,
            };
        }
        if !helper.warnings.is_empty() && helper.capture_metadata.warnings.is_empty() {
            helper.capture_metadata.warnings = helper.warnings;
        }

        Self {
            run_id: helper.run_id,
            target: helper.target,
            function: helper.function,
            provider: helper.provider,
            backend: helper.backend,
            format: helper.format,
            native_capture: helper.native_capture,
            semantic_profile: helper.semantic_profile,
            capture_metadata: helper.capture_metadata,
        }
    }
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
    let _ = writeln!(markdown);
    let _ = writeln!(markdown, "## Native capture");
    let _ = writeln!(markdown);
    let _ = writeln!(
        markdown,
        "- Status: `{}`",
        capture_status_label(manifest.native_capture.status)
    );
    let _ = writeln!(markdown, "- Raw artifacts:");
    for artifact in &manifest.native_capture.raw_artifacts {
        let _ = writeln!(
            markdown,
            "  - `{}`: `{}`",
            artifact.label,
            artifact.path.display()
        );
    }
    let _ = writeln!(markdown, "- Processed artifacts:");
    for artifact in &manifest.native_capture.processed_artifacts {
        let _ = writeln!(
            markdown,
            "  - `{}`: `{}`",
            artifact.label,
            artifact.path.display()
        );
    }
    let _ = writeln!(markdown, "- Symbolization:");
    let _ = writeln!(
        markdown,
        "  - Status: `{}`",
        capture_status_label(manifest.native_capture.symbolization.status)
    );
    if let Some(tool) = &manifest.native_capture.symbolization.tool {
        let _ = writeln!(markdown, "  - Tool: `{tool}`");
    }
    let _ = writeln!(
        markdown,
        "  - Resolved frames: `{}`",
        manifest.native_capture.symbolization.resolved_frames
    );
    let _ = writeln!(
        markdown,
        "  - Unresolved frames: `{}`",
        manifest.native_capture.symbolization.unresolved_frames
    );
    for note in &manifest.native_capture.symbolization.notes {
        let _ = writeln!(markdown, "  - {}", note);
    }
    if let Some(viewer_hint) = &manifest.native_capture.viewer_hint {
        let _ = writeln!(markdown);
        let _ = writeln!(markdown, "## Viewer");
        let _ = writeln!(markdown);
        let _ = writeln!(markdown, "{}", viewer_hint);
    }
    let _ = writeln!(markdown);
    let _ = writeln!(markdown, "## Semantic phases");
    let _ = writeln!(markdown);
    let _ = writeln!(
        markdown,
        "- Status: `{}`",
        semantic_capture_status_label(manifest.semantic_profile.status)
    );
    match &manifest.semantic_profile.spans_path {
        Some(path) => {
            let _ = writeln!(markdown, "- Spans path: `{}`", path.display());
        }
        None => {
            let _ = writeln!(markdown, "- Spans path: `not recorded`");
        }
    }
    if manifest.semantic_profile.phases.is_empty() {
        let _ = writeln!(markdown, "- No semantic phases recorded");
    } else {
        let _ = writeln!(markdown, "- Phases:");
        for phase in &manifest.semantic_profile.phases {
            let _ = writeln!(markdown, "  - `{}`", phase.name);
            if let Some(duration_ns) = phase.duration_ns {
                let _ = writeln!(markdown, "    - Duration: `{duration_ns}` ns");
            }
            if let Some(percent_total) = phase.percent_total {
                let _ = writeln!(markdown, "    - Share of total: `{percent_total}`");
            }
        }
    }
    let _ = writeln!(markdown);
    let _ = writeln!(markdown, "## Capture metadata");
    let _ = writeln!(markdown);
    match &manifest.capture_metadata.device {
        Some(device) => {
            let _ = writeln!(markdown, "- Device: `{device}`");
        }
        None => {
            let _ = writeln!(markdown, "- Device: `not recorded`");
        }
    }
    match manifest.capture_metadata.sample_duration_secs {
        Some(sample_duration_secs) => {
            let _ = writeln!(markdown, "- Sample duration: `{sample_duration_secs}` s");
        }
        None => {
            let _ = writeln!(markdown, "- Sample duration: `not recorded`");
        }
    }
    match &manifest.capture_metadata.warmup_mode {
        Some(warmup_mode) => {
            let _ = writeln!(markdown, "- Warmup mode: `{}`", warmup_mode.as_str());
        }
        None => {
            let _ = writeln!(markdown, "- Warmup mode: `not recorded`");
        }
    }
    match &manifest.capture_metadata.capture_method {
        Some(capture_method) => {
            let _ = writeln!(markdown, "- Capture method: `{capture_method}`");
        }
        None => {
            let _ = writeln!(markdown, "- Capture method: `not recorded`");
        }
    }
    if !manifest.capture_metadata.warnings.is_empty() {
        let _ = writeln!(markdown);
        let _ = writeln!(markdown, "### Warnings");
        let _ = writeln!(markdown);
        for warning in &manifest.capture_metadata.warnings {
            let _ = writeln!(markdown, "- {}", warning);
        }
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
    run_profile_session_with_executor(args, dry_run, execute_capture)
}

fn run_profile_session_with_executor<E>(
    args: &ProfileRunArgs,
    dry_run: bool,
    execute: E,
) -> Result<()>
where
    E: FnOnce(&ProfileRunArgs, &ResolvedProfileTarget, &mut ProfileManifest) -> Result<()>,
{
    let target = resolve_profile_target(args)?;
    let run_id = build_run_id(args.target, &args.function);
    let run_output_dir = args.output_dir.join(&run_id);
    let mut manifest = build_capture_plan(args, &target, &run_output_dir)?;
    let execution_result = if dry_run {
        manifest.capture_metadata.warnings.push(
            "dry-run enabled; capture planning stopped before execution and recorded the planned artifact contract only"
                .into(),
        );
        Ok(())
    } else {
        execute(args, &target, &mut manifest)
    };

    write_profile_session_outputs(args, &run_output_dir, &manifest)?;
    execution_result?;

    println!(
        "Profile session written to {}",
        run_output_dir.join("profile.json").display()
    );
    println!(
        "Profile summary written to {}",
        run_output_dir.join("summary.md").display()
    );
    println!(
        "Latest profile manifest refreshed at {}",
        args.output_dir.join("profile.json").display()
    );
    println!(
        "Latest profile summary refreshed at {}",
        args.output_dir.join("summary.md").display()
    );
    Ok(())
}

fn write_profile_session_outputs(
    args: &ProfileRunArgs,
    run_output_dir: &Path,
    manifest: &ProfileManifest,
) -> Result<()> {
    std::fs::create_dir_all(&args.output_dir)?;
    std::fs::create_dir_all(&run_output_dir)?;
    create_selected_artifact_roots(
        &manifest.native_capture.raw_artifacts,
        &manifest.native_capture.processed_artifacts,
    )?;
    let rendered_summary = render_profile_markdown(&manifest);

    let run_profile_path = run_output_dir.join("profile.json");
    let run_summary_path = run_output_dir.join("summary.md");
    write_semantic_phase_sidecar(manifest)?;
    write_profile_manifest(&run_profile_path, &manifest)?;
    std::fs::write(&run_summary_path, rendered_summary.as_bytes())?;

    let latest_profile_path = args.output_dir.join("profile.json");
    let latest_summary_path = args.output_dir.join("summary.md");
    write_profile_manifest(&latest_profile_path, &manifest)?;
    std::fs::write(&latest_summary_path, rendered_summary.as_bytes())?;
    Ok(())
}

fn write_semantic_phase_sidecar(manifest: &ProfileManifest) -> Result<()> {
    let Some(path) = manifest.semantic_profile.spans_path.as_ref() else {
        return Ok(());
    };
    if manifest.semantic_profile.phases.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&manifest.semantic_profile.phases)?,
    )?;
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

fn semantic_capture_status_label(status: SemanticCaptureStatus) -> &'static str {
    match status {
        SemanticCaptureStatus::Planned => "planned",
        SemanticCaptureStatus::Captured => "captured",
        SemanticCaptureStatus::Partial => "partial",
        SemanticCaptureStatus::Failed => "failed",
    }
}

#[allow(dead_code)]
pub(crate) fn symbolize_android_folded_stacks_with_resolver<F>(
    folded_stacks: &str,
    mut resolve: F,
) -> (String, SymbolizationRecord, String)
where
    F: FnMut(&str, u64) -> Option<String>,
{
    let mut lines = Vec::new();
    let mut resolved_frames = 0;
    let mut unresolved_frames = 0;

    for line in folded_stacks.lines().filter(|line| !line.trim().is_empty()) {
        let symbolized =
            mobench_sdk::builders::android::symbolize_android_native_stack_line_with_resolver(
                line,
                |library_name, offset| resolve(library_name, offset),
            );
        resolved_frames += symbolized.resolved_frames;
        unresolved_frames += symbolized.unresolved_frames;
        lines.push(symbolized.line);
    }

    let symbolized_stacks = lines.join("\n");
    let status = match (resolved_frames, unresolved_frames) {
        (0, 0) => CaptureStatus::Planned,
        (_, 0) => CaptureStatus::Captured,
        (0, _) => CaptureStatus::Failed,
        _ => CaptureStatus::Partial,
    };
    let mut notes = Vec::new();
    if unresolved_frames > 0 {
        notes.push("some native frames could not be symbolized".into());
    }

    let record = SymbolizationRecord {
        status,
        tool: Some("llvm-addr2line".into()),
        resolved_frames,
        unresolved_frames,
        notes,
    };
    let report = if symbolized_stacks.is_empty() {
        "No native frames were symbolized.".into()
    } else {
        symbolized_stacks.clone()
    };

    (symbolized_stacks, record, report)
}

#[allow(dead_code)]
pub(crate) fn symbolize_android_folded_stacks_with_native_libraries<F>(
    folded_stacks: &str,
    native_libraries: &[NativeLibraryArtifact],
    runtime_abi: Option<&str>,
    mut resolve: F,
) -> (String, SymbolizationRecord, String)
where
    F: FnMut(&Path, u64) -> Option<String>,
{
    let runtime_abi = runtime_abi.map(str::to_owned);

    symbolize_android_folded_stacks_with_resolver(folded_stacks, |library_name, offset| {
        let library_path = resolve_android_native_library_path(
            native_libraries,
            library_name,
            runtime_abi.as_deref(),
        )?;
        resolve(library_path, offset)
    })
}

fn resolve_android_native_library_path<'a>(
    native_libraries: &'a [NativeLibraryArtifact],
    library_name: &str,
    runtime_abi: Option<&str>,
) -> Option<&'a Path> {
    match runtime_abi {
        Some(runtime_abi) => native_libraries
            .iter()
            .find(|artifact| artifact.library_name == library_name && artifact.abi == runtime_abi)
            .map(|artifact| artifact.unstripped_path.as_path()),
        None => {
            let mut matching = native_libraries
                .iter()
                .filter(|artifact| artifact.library_name == library_name);
            let artifact = matching.next()?;
            if matching.next().is_some() {
                return None;
            }
            Some(artifact.unstripped_path.as_path())
        }
    }
}

#[allow(dead_code)]
pub(crate) fn write_android_symbolized_outputs(
    folded_stacks: &str,
    native_libraries: &[NativeLibraryArtifact],
    processed_root: &Path,
    runtime_abi: Option<&str>,
    llvm_addr2line_path: &Path,
) -> Result<SymbolizationRecord> {
    write_android_symbolized_outputs_with_resolver(
        folded_stacks,
        native_libraries,
        processed_root,
        runtime_abi,
        |library_path, offset| {
            mobench_sdk::builders::android::resolve_android_native_symbol_with_tool(
                llvm_addr2line_path,
                library_path,
                offset,
            )
        },
    )
}

pub(crate) fn write_android_symbolized_outputs_with_resolver<F>(
    folded_stacks: &str,
    native_libraries: &[NativeLibraryArtifact],
    processed_root: &Path,
    runtime_abi: Option<&str>,
    resolve: F,
) -> Result<SymbolizationRecord>
where
    F: FnMut(&Path, u64) -> Option<String>,
{
    std::fs::create_dir_all(processed_root)?;

    let (symbolized_stacks, mut record, report) = symbolize_android_folded_stacks_with_native_libraries(
        folded_stacks,
        native_libraries,
        runtime_abi,
        resolve,
    );

    std::fs::write(processed_root.join("stacks.folded"), &symbolized_stacks)?;
    std::fs::write(processed_root.join("native-report.txt"), &report)?;
    if let Some(warning) = write_dual_view_flamegraph_bundle(
        &symbolized_stacks,
        processed_root,
        "Android Native Profile",
        ANDROID_BENCHMARK_ANCHORS,
        "../raw/sample.perf",
        "Raw sample.perf",
    )? {
        record.notes.push(warning);
    }

    Ok(record)
}

fn write_dual_view_flamegraph_bundle(
    full_folded_stacks: &str,
    processed_root: &Path,
    title: &str,
    anchors: &[&str],
    raw_artifact_path: &str,
    raw_artifact_label: &str,
) -> Result<Option<String>> {
    std::fs::create_dir_all(processed_root)?;
    std::fs::write(processed_root.join("stacks.folded"), full_folded_stacks)?;

    let focused = derive_benchmark_focused_folded_stacks(full_folded_stacks, anchors);
    std::fs::write(
        processed_root.join("benchmark.focused.folded"),
        &focused.folded,
    )?;

    let full_svg = render_standalone_flamegraph_svg(full_folded_stacks, title)?;
    std::fs::write(processed_root.join("flamegraph.full.svg"), &full_svg)?;

    let full_summary = summarize_folded_stacks(
        full_folded_stacks,
        count_folded_stack_lines(full_folded_stacks),
        0,
        None,
    );

    let focused_warning = if focused.folded.trim().is_empty() {
        Some(
            "No benchmark anchor frames were detected; the benchmark-only view is falling back to the full-process flamegraph."
                .to_string(),
        )
    } else {
        None
    };

    let focused_svg = if focused.folded.trim().is_empty() {
        full_svg.clone()
    } else {
        render_standalone_flamegraph_svg(&focused.folded, title)?
    };
    std::fs::write(processed_root.join("flamegraph.focused.svg"), &focused_svg)?;

    let focused_summary = summarize_folded_stacks(
        if focused.folded.trim().is_empty() {
            full_folded_stacks
        } else {
            &focused.folded
        },
        focused.matched_stack_count,
        focused.excluded_stack_count,
        focused_warning.clone(),
    );

    let viewer_html = render_flamegraph_viewer_html(FlamegraphViewerDoc {
        title: title.to_string(),
        full_svg_document: full_svg,
        focused_svg_document: focused_svg,
        full_summary,
        focused_summary,
        default_mode: FlamegraphMode::Focused,
        artifact_links: vec![
            ViewerArtifactLink::new(raw_artifact_label, raw_artifact_path),
            ViewerArtifactLink::new("Native report", "native-report.txt"),
            ViewerArtifactLink::new("Full folded stacks", "stacks.folded"),
            ViewerArtifactLink::new("Benchmark-focused folded stacks", "benchmark.focused.folded"),
            ViewerArtifactLink::new("Full-process SVG", "flamegraph.full.svg"),
            ViewerArtifactLink::new("Benchmark-only SVG", "flamegraph.focused.svg"),
        ],
    });
    std::fs::write(processed_root.join("flamegraph.html"), viewer_html)?;

    Ok(focused_warning)
}

const DEFAULT_PROFILE_ITERATIONS: u32 = 20;
const DEFAULT_PROFILE_WARMUP: u32 = 3;
const DEFAULT_ANDROID_CAPTURE_DURATION_SECS: u64 = 10;
const DEFAULT_ANDROID_WARMUP_TIMEOUT_SECS: u64 = 60;
const DEFAULT_IOS_CAPTURE_DURATION_SECS: u64 = 10;
const DEFAULT_IOS_BENCH_DELAY_MS: u64 = 1_500;
const DEFAULT_IOS_PROFILE_REPEAT_UNTIL_MS: u64 = DEFAULT_IOS_CAPTURE_DURATION_SECS * 1_000;
const DEFAULT_IOS_LOG_TIMEOUT_SECS: u64 = 60;
const ANDROID_BENCH_LOG_MARKER: &str = "BENCH_JSON";
const ANDROID_BENCHMARK_ANCHORS: &[&str] = &[
    "sample_fns::run_benchmark",
    "mobench_sdk::timing::run_closure",
    "uniffi.",
    "uniffi_",
    "runBenchmark",
];
const IOS_BENCHMARK_ANCHORS: &[&str] = &[
    "runBenchmark(spec:)",
    "sample_fns::run_benchmark",
    "mobench_sdk::timing::run_closure",
    "uniffi_",
    "BenchRunnerFFI.run(params:)",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalIosSimulator {
    udid: String,
    name: String,
    os_version: String,
    state: String,
}

impl LocalIosSimulator {
    fn identifier(&self) -> String {
        format!("{}-{}", self.name, self.os_version)
    }
}

#[derive(Debug, Clone)]
struct AndroidProfilerToolchain {
    sdk_root: PathBuf,
    adb_path: PathBuf,
    app_profiler_path: PathBuf,
    stackcollapse_path: PathBuf,
    python_path: PathBuf,
    llvm_addr2line_path: PathBuf,
}

fn locate_android_profiler_toolchain() -> Result<AndroidProfilerToolchain> {
    let sdk_root = std::env::var_os("ANDROID_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("ANDROID_SDK_ROOT").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("ANDROID_NDK_HOME")
                .map(PathBuf::from)
                .and_then(|ndk_home| ndk_home.parent().and_then(Path::parent).map(PathBuf::from))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join("Library").join("Android").join("sdk"))
        })
        .filter(|path| path.exists())
        .context("Android SDK not found; set ANDROID_HOME or ANDROID_SDK_ROOT")?;

    let ndk_root = std::env::var_os("ANDROID_NDK_HOME")
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .or_else(|| {
            let ndk_dir = sdk_root.join("ndk");
            std::fs::read_dir(&ndk_dir).ok().and_then(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir())
                    .max()
            })
        })
        .context("Android NDK not found; set ANDROID_NDK_HOME or install an NDK under the SDK")?;

    let adb_path = sdk_root.join("platform-tools").join("adb");
    let app_profiler_path = ndk_root.join("simpleperf").join("app_profiler.py");
    let stackcollapse_path = ndk_root.join("simpleperf").join("stackcollapse.py");
    let python_path = std::env::var_os("PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python3"));
    let llvm_addr2line_override = std::env::var_os("MOBENCH_ANDROID_LLVM_ADDR2LINE")
        .or_else(|| std::env::var_os("LLVM_ADDR2LINE"))
        .map(PathBuf::from);
    let llvm_addr2line_path =
        locate_android_llvm_addr2line(&ndk_root, llvm_addr2line_override.as_deref())?;

    for path in [&adb_path, &app_profiler_path, &stackcollapse_path] {
        if !path.exists() {
            bail!(
                "required Android profiling tool not found at {}",
                path.display()
            );
        }
    }

    Ok(AndroidProfilerToolchain {
        sdk_root,
        adb_path,
        app_profiler_path,
        stackcollapse_path,
        python_path,
        llvm_addr2line_path,
    })
}

fn locate_android_llvm_addr2line(ndk_root: &Path, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        if path.exists() {
            return Ok(path.to_path_buf());
        }
        bail!(
            "explicit llvm-addr2line override does not exist at {}",
            path.display()
        );
    }

    let prebuilt_root = ndk_root.join("toolchains").join("llvm").join("prebuilt");
    let tool_name = if cfg!(windows) {
        "llvm-addr2line.exe"
    } else {
        "llvm-addr2line"
    };
    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&prebuilt_root) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("bin").join(tool_name);
            if candidate.exists() {
                candidates.push(candidate);
            }
        }
    }
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .context(
            "llvm-addr2line not found under the Android NDK; set MOBENCH_ANDROID_LLVM_ADDR2LINE or LLVM_ADDR2LINE to override",
        )
}

fn prepend_path_env(toolchain: &AndroidProfilerToolchain) -> Option<std::ffi::OsString> {
    let mut entries = vec![toolchain.sdk_root.join("platform-tools").into_os_string()];
    if let Some(existing) = std::env::var_os("PATH") {
        entries.push(existing);
    }
    std::env::join_paths(entries).ok()
}

fn ensure_android_device_connected(toolchain: &AndroidProfilerToolchain) -> Result<()> {
    let output = Command::new(&toolchain.adb_path)
        .arg("devices")
        .output()
        .context("failed to run `adb devices`")?;
    if !output.status.success() {
        bail!("adb devices failed with status {}", output.status);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout
        .lines()
        .skip(1)
        .any(|line| line.split_whitespace().nth(1) == Some("device"))
    {
        return Ok(());
    }

    let avd_hint = sdk_root_emulator_hint(&toolchain.sdk_root)
        .unwrap_or_else(|| "start an Android emulator or connect a device over adb".into());
    bail!("no Android device is connected via adb; {avd_hint}");
}

fn sdk_root_emulator_hint(sdk_root: &Path) -> Option<String> {
    let emulator_path = sdk_root.join("emulator").join("emulator");
    if !emulator_path.exists() {
        return None;
    }
    let output = Command::new(&emulator_path)
        .arg("-list-avds")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let avd = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| !line.trim().is_empty())?
        .trim()
        .to_string();
    Some(format!(
        "start one with `{}` -avd `{}`",
        emulator_path.display(),
        avd
    ))
}

fn read_android_application_id(android_root: &Path) -> Result<String> {
    let build_gradle = android_root.join("app").join("build.gradle");
    let contents = std::fs::read_to_string(&build_gradle)
        .with_context(|| format!("reading {}", build_gradle.display()))?;
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("applicationId ") {
            return extract_quoted_value(value)
                .with_context(|| format!("parsing applicationId from {}", build_gradle.display()));
        }
    }
    bail!("applicationId not found in {}", build_gradle.display())
}

fn extract_quoted_value(source: &str) -> Result<String> {
    let start = source.find('"').context("missing opening quote")? + 1;
    let end = source[start..]
        .find('"')
        .map(|index| start + index)
        .context("missing closing quote")?;
    Ok(source[start..end].to_string())
}

fn run_android_stackcollapse(
    toolchain: &AndroidProfilerToolchain,
    perf_data_path: &Path,
    working_dir: &Path,
) -> Result<String> {
    let mut command = Command::new(&toolchain.python_path);
    command
        .arg(&toolchain.stackcollapse_path)
        .arg("-i")
        .arg(perf_data_path)
        .current_dir(working_dir);
    if let Some(path_env) = prepend_path_env(toolchain) {
        command.env("PATH", path_env);
    }
    let output = command
        .output()
        .with_context(|| format!("running {}", toolchain.stackcollapse_path.display()))?;
    if !output.status.success() {
        bail!(
            "stackcollapse.py failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn execute_local_android_capture(
    args: &ProfileRunArgs,
    manifest: &mut ProfileManifest,
) -> Result<()> {
    let toolchain = locate_android_profiler_toolchain()?;
    ensure_android_device_connected(&toolchain)?;
    let runtime_abi = resolve_android_runtime_abi(&toolchain)?;

    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: None,
        crate_path: args.crate_path.as_deref(),
        config_path: args.config.as_deref(),
    })?;
    load_dotenv_for_layout(&layout);
    validate_benchmark_function(&layout, &args.function)?;

    let spec = RunSpec {
        target: MobileTarget::Android,
        function: args.function.clone(),
        iterations: DEFAULT_PROFILE_ITERATIONS,
        warmup: DEFAULT_PROFILE_WARMUP,
        devices: Vec::new(),
        browserstack: None,
        ios_xcuitest: None,
    };
    persist_mobile_spec(&layout, &spec, false)?;

    let build = run_android_build(&layout, "", false, false)?;
    let android_root = layout.output_dir.join("android");
    let package_name = read_android_application_id(&android_root)?;
    let warmup_mode = manifest
        .capture_metadata
        .warmup_mode
        .unwrap_or(CaptureWarmupMode::Cold);

    let raw_perf_path = manifest
        .native_capture
        .raw_artifacts
        .iter()
        .find(|artifact| artifact.label == "simpleperf")
        .map(|artifact| artifact.path.clone())
        .context("android profile plan missing simpleperf artifact")?;
    let processed_root = manifest
        .native_capture
        .processed_artifacts
        .iter()
        .find_map(|artifact| artifact.path.parent().map(Path::to_path_buf))
        .context("android profile plan missing processed artifact root")?;
    if let Some(parent) = raw_perf_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&processed_root)?;

    let mut install = Command::new(&toolchain.adb_path);
    install.arg("install").arg("-r").arg(&build.app_path);
    if let Some(path_env) = prepend_path_env(&toolchain) {
        install.env("PATH", path_env.clone());
    }
    let install_output = install
        .output()
        .with_context(|| format!("installing {}", build.app_path.display()))?;
    if !install_output.status.success() {
        bail!(
            "adb install failed with status {}\nstdout:\n{}\nstderr:\n{}",
            install_output.status,
            String::from_utf8_lossy(&install_output.stdout),
            String::from_utf8_lossy(&install_output.stderr)
        );
    }

    prepare_android_profile_capture(&toolchain, &package_name, warmup_mode)?;
    manifest.capture_metadata.warmup_mode = Some(warmup_mode);
    if let Err(error) = android_clear_logcat(&toolchain) {
        manifest.capture_metadata.warnings.push(format!(
            "failed to clear Android logcat before the recorded profile run: {error}"
        ));
    }

    let mut profiler = Command::new(&toolchain.python_path);
    profiler
        .arg(&toolchain.app_profiler_path)
        .arg("-p")
        .arg(&package_name)
        .arg("-a")
        .arg(".MainActivity")
        .arg("-o")
        .arg(&raw_perf_path)
        .arg("-r")
        .arg(format!(
            "-e task-clock:u -f 1000 -g --duration {}",
            DEFAULT_ANDROID_CAPTURE_DURATION_SECS
        ))
        .current_dir(
            raw_perf_path
                .parent()
                .context("simpleperf artifact path missing parent directory")?,
        );
    if let Some(path_env) = prepend_path_env(&toolchain) {
        profiler.env("PATH", path_env);
    }
    let profiler_output = profiler.output().with_context(|| {
        format!(
            "running Android profiler script {}",
            toolchain.app_profiler_path.display()
        )
    })?;
    if !profiler_output.status.success() {
        bail!(
            "app_profiler.py failed with status {}\nstdout:\n{}\nstderr:\n{}",
            profiler_output.status,
            String::from_utf8_lossy(&profiler_output.stdout),
            String::from_utf8_lossy(&profiler_output.stderr)
        );
    }

    let folded_stacks = run_android_stackcollapse(
        &toolchain,
        &raw_perf_path,
        raw_perf_path
            .parent()
            .context("simpleperf artifact path missing parent directory")?,
    )?;
    let symbolization = write_android_symbolized_outputs(
        &folded_stacks,
        &build.native_libraries,
        &processed_root,
        runtime_abi.as_deref(),
        &toolchain.llvm_addr2line_path,
    )?;

    manifest.native_capture.symbolization = symbolization.clone();
    manifest.native_capture.status = match symbolization.status {
        CaptureStatus::Planned | CaptureStatus::Captured => CaptureStatus::Captured,
        CaptureStatus::Partial | CaptureStatus::Failed => CaptureStatus::Partial,
    };
    manifest.capture_metadata.sample_duration_secs = Some(DEFAULT_ANDROID_CAPTURE_DURATION_SECS);
    manifest.capture_metadata.capture_method = Some("simpleperf/app_profiler.py".into());
    manifest.capture_metadata.warnings.push(format!(
        "android profile run used default benchmark settings: iterations={}, warmup={}",
        DEFAULT_PROFILE_ITERATIONS, DEFAULT_PROFILE_WARMUP
    ));
    if warmup_mode == CaptureWarmupMode::Warm {
        manifest.capture_metadata.warnings.push(
            "performed one preparatory warm launch before recording; startup caches are warmed, but per-process bridge initialization may still appear in the captured run".into(),
        );
    }
    match android_read_logcat(&toolchain) {
        Ok(logs) => {
            let reports = extract_benchmark_reports_from_logs(&logs);
            if let Some(report) = select_benchmark_value_for_function(&reports, &args.function) {
                merge_semantic_profile_from_bench_report(manifest, report)?;
            }
        }
        Err(error) => {
            manifest.capture_metadata.warnings.push(format!(
                "semantic phase capture was unavailable because Android logcat could not be read: {error}"
            ));
        }
    }

    Ok(())
}

fn prepare_android_profile_capture(
    toolchain: &AndroidProfilerToolchain,
    package_name: &str,
    warmup_mode: CaptureWarmupMode,
) -> Result<()> {
    android_force_stop(toolchain, package_name)?;
    if warmup_mode == CaptureWarmupMode::Cold {
        return Ok(());
    }

    android_clear_logcat(toolchain)?;
    android_start_activity(toolchain, package_name, ".MainActivity")?;
    wait_for_android_bench_log_marker(
        toolchain,
        ANDROID_BENCH_LOG_MARKER,
        DEFAULT_ANDROID_WARMUP_TIMEOUT_SECS,
    )?;
    android_force_stop(toolchain, package_name)?;
    Ok(())
}

fn android_force_stop(toolchain: &AndroidProfilerToolchain, package_name: &str) -> Result<()> {
    let output = Command::new(&toolchain.adb_path)
        .args(["shell", "am", "force-stop"])
        .arg(package_name)
        .output()
        .with_context(|| format!("force-stopping Android package {package_name}"))?;
    if !output.status.success() {
        bail!(
            "adb force-stop failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn android_clear_logcat(toolchain: &AndroidProfilerToolchain) -> Result<()> {
    let output = Command::new(&toolchain.adb_path)
        .args(["logcat", "-c"])
        .output()
        .context("clearing Android logcat before warm profile capture")?;
    if !output.status.success() {
        bail!(
            "adb logcat -c failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn android_start_activity(
    toolchain: &AndroidProfilerToolchain,
    package_name: &str,
    activity_name: &str,
) -> Result<()> {
    let component = format!("{package_name}/{activity_name}");
    let output = Command::new(&toolchain.adb_path)
        .args(["shell", "am", "start", "-W", "-n"])
        .arg(&component)
        .output()
        .with_context(|| format!("starting Android activity {component}"))?;
    if !output.status.success() {
        bail!(
            "adb am start failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn wait_for_android_bench_log_marker(
    toolchain: &AndroidProfilerToolchain,
    marker: &str,
    timeout_secs: u64,
) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        let logcat = android_read_logcat(toolchain)?;
        if android_log_contains_marker(&logcat, marker) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    bail!("timed out waiting for Android warmup marker `{marker}` in logcat");
}

fn android_read_logcat(toolchain: &AndroidProfilerToolchain) -> Result<String> {
    let output = Command::new(&toolchain.adb_path)
        .args(["logcat", "-d", "-s", "BenchRunner:I", "MainActivity:D"])
        .output()
        .context("reading Android logcat for warm profile capture")?;
    if !output.status.success() {
        bail!(
            "adb logcat -d failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn android_log_contains_marker(logcat: &str, marker: &str) -> bool {
    logcat.lines().any(|line| line.contains(marker))
}

fn execute_local_ios_capture(args: &ProfileRunArgs, manifest: &mut ProfileManifest) -> Result<()> {
    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: None,
        crate_path: args.crate_path.as_deref(),
        config_path: args.config.as_deref(),
    })?;
    load_dotenv_for_layout(&layout);
    validate_benchmark_function(&layout, &args.function)?;

    let spec = RunSpec {
        target: MobileTarget::Ios,
        function: args.function.clone(),
        iterations: DEFAULT_PROFILE_ITERATIONS,
        warmup: DEFAULT_PROFILE_WARMUP,
        devices: Vec::new(),
        browserstack: None,
        ios_xcuitest: None,
    };
    persist_mobile_spec(&layout, &spec, false)?;

    let requested_device = resolve_profile_device(args)?;
    let simulator = resolve_local_ios_simulator(requested_device.as_ref())?;
    ensure_local_ios_simulator_booted(&simulator)?;
    manifest.capture_metadata.device = Some(simulator.identifier());

    run_ios_build(&layout, false, false)?;
    let app_path = build_local_ios_simulator_app(&layout, &simulator)?;
    install_local_ios_app(&simulator, &app_path)?;

    let bundle_id = local_ios_bundle_identifier(&layout.crate_name);
    let warmup_mode = manifest
        .capture_metadata
        .warmup_mode
        .unwrap_or(CaptureWarmupMode::Cold);
    manifest.capture_metadata.warmup_mode = Some(warmup_mode);

    let raw_sample_path = manifest
        .native_capture
        .raw_artifacts
        .iter()
        .find(|artifact| artifact.label == "sample")
        .map(|artifact| artifact.path.clone())
        .context("ios profile plan missing sample artifact")?;
    let processed_root = manifest
        .native_capture
        .processed_artifacts
        .iter()
        .find_map(|artifact| artifact.path.parent().map(Path::to_path_buf))
        .context("ios profile plan missing processed artifact root")?;
    if let Some(parent) = raw_sample_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&processed_root)?;

    if warmup_mode == CaptureWarmupMode::Warm
        && let Err(error) = run_local_ios_warmup_pass(&simulator, &bundle_id, &raw_sample_path)
    {
        manifest.capture_metadata.warnings.push(format!(
            "failed to complete the preparatory iOS warm launch cleanly; continuing with the recorded run cold-ish: {error}"
        ));
    }

    let log_dir = raw_sample_path
        .parent()
        .context("ios sample artifact missing parent directory")?;
    let stdout_path = log_dir.join("app.stdout.log");
    let stderr_path = log_dir.join("app.stderr.log");
    let app_args = [
        format!("--mobench-profile-bench-delay-ms={DEFAULT_IOS_BENCH_DELAY_MS}"),
        format!(
            "--mobench-profile-repeat-until-ms={DEFAULT_IOS_PROFILE_REPEAT_UNTIL_MS}"
        ),
        format!(
            "--mobench-profile-result-hold-ms={}",
            DEFAULT_IOS_CAPTURE_DURATION_SECS * 1_000
        ),
    ];
    let app_env = [
        (
            "MOBENCH_BENCH_DELAY_MS",
            DEFAULT_IOS_BENCH_DELAY_MS.to_string(),
        ),
        (
            "MOBENCH_PROFILE_REPEAT_UNTIL_MS",
            DEFAULT_IOS_PROFILE_REPEAT_UNTIL_MS.to_string(),
        ),
        (
            "MOBENCH_PROFILE_RESULT_HOLD_MS",
            (DEFAULT_IOS_CAPTURE_DURATION_SECS * 1_000).to_string(),
        ),
    ];
    let pid = launch_local_ios_app(
        &simulator,
        &bundle_id,
        &stdout_path,
        &stderr_path,
        &app_args,
        &app_env,
    )?;

    let sample_result = run_ios_sample_capture(pid, &raw_sample_path);
    let log_wait_result = wait_for_ios_log_marker(
        &[stdout_path.clone(), stderr_path.clone()],
        "BENCH_REPORT_JSON_END",
        DEFAULT_IOS_LOG_TIMEOUT_SECS,
    );
    let terminate_result = terminate_local_ios_app(&simulator, &bundle_id);

    sample_result?;
    if let Err(error) = log_wait_result {
        manifest.capture_metadata.warnings.push(format!(
            "semantic phase capture may be incomplete because the iOS benchmark log marker was not observed before timeout: {error}"
        ));
    }
    if let Err(error) = terminate_result {
        manifest.capture_metadata.warnings.push(format!(
            "failed to terminate the profiled iOS simulator app after capture: {error}"
        ));
    }

    let sample_output = std::fs::read_to_string(&raw_sample_path)
        .with_context(|| format!("reading iOS sample output at {}", raw_sample_path.display()))?;
    let symbolization = write_ios_processed_outputs(&sample_output, &processed_root)?;
    manifest.native_capture.symbolization = symbolization.clone();
    manifest.native_capture.status = match symbolization.status {
        CaptureStatus::Planned | CaptureStatus::Captured => CaptureStatus::Captured,
        CaptureStatus::Partial | CaptureStatus::Failed => CaptureStatus::Partial,
    };
    manifest.capture_metadata.sample_duration_secs = Some(DEFAULT_IOS_CAPTURE_DURATION_SECS);
    manifest.capture_metadata.capture_method = Some("sample/simctl".into());
    manifest.capture_metadata.warnings.push(format!(
        "ios profile run used default benchmark settings: iterations={}, warmup={}",
        DEFAULT_PROFILE_ITERATIONS, DEFAULT_PROFILE_WARMUP
    ));
    if warmup_mode == CaptureWarmupMode::Warm {
        manifest.capture_metadata.warnings.push(
            "performed one preparatory warm launch before recording so the measured sample de-emphasizes first-run bridge and UI setup costs".into(),
        );
    }
    manifest.capture_metadata.warnings.push(format!(
        "iOS profile capture repeated benchmark work for about {} ms so fast functions remain visible in sampled stacks",
        DEFAULT_IOS_PROFILE_REPEAT_UNTIL_MS
    ));

    match read_combined_text_files(&[stdout_path, stderr_path]) {
        Ok(logs) => {
            if let Some(report) = extract_ios_benchmark_json(&logs) {
                merge_semantic_profile_from_bench_report(manifest, &report)?;
            } else {
                manifest.capture_metadata.warnings.push(
                    "semantic phase capture was unavailable because the iOS log output did not contain BENCH_REPORT_JSON markers".into(),
                );
            }
        }
        Err(error) => {
            manifest.capture_metadata.warnings.push(format!(
                "semantic phase capture was unavailable because iOS app logs could not be read: {error}"
            ));
        }
    }

    Ok(())
}

fn run_local_ios_warmup_pass(
    simulator: &LocalIosSimulator,
    bundle_id: &str,
    raw_sample_path: &Path,
) -> Result<()> {
    let log_dir = raw_sample_path
        .parent()
        .context("ios sample artifact missing parent directory")?;
    let stdout_path = log_dir.join("warmup.stdout.log");
    let stderr_path = log_dir.join("warmup.stderr.log");
    let app_args = [String::from("--mobench-profile-warmup-only=1")];
    let app_env = [("MOBENCH_PROFILE_WARMUP_ONLY", String::from("1"))];
    let _pid = launch_local_ios_app(
        simulator,
        bundle_id,
        &stdout_path,
        &stderr_path,
        &app_args,
        &app_env,
    )?;

    let wait_result = wait_for_ios_log_marker(
        &[stdout_path, stderr_path],
        "BENCH_REPORT_JSON_END",
        DEFAULT_IOS_LOG_TIMEOUT_SECS,
    );
    let terminate_result = terminate_local_ios_app(simulator, bundle_id);

    wait_result?;
    terminate_result?;
    Ok(())
}

fn resolve_local_ios_simulator(
    requested: Option<&ResolvedProfileDevice>,
) -> Result<LocalIosSimulator> {
    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available", "--json"])
        .output()
        .context("listing available iOS simulators with simctl")?;
    if !output.status.success() {
        bail!(
            "xcrun simctl list devices available --json failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let value: Value = serde_json::from_slice(&output.stdout)
        .context("parsing iOS simulator list JSON from simctl")?;
    let mut simulators = Vec::new();
    let Some(devices) = value.get("devices").and_then(Value::as_object) else {
        bail!("simctl JSON did not contain a `devices` object");
    };

    for (runtime_key, entries) in devices {
        let Some(os_version) = parse_ios_runtime_version(runtime_key) else {
            continue;
        };
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(udid) = entry.get("udid").and_then(Value::as_str) else {
                continue;
            };
            let Some(state) = entry.get("state").and_then(Value::as_str) else {
                continue;
            };
            if entry
                .get("isAvailable")
                .and_then(Value::as_bool)
                .unwrap_or(true)
            {
                simulators.push(LocalIosSimulator {
                    udid: udid.to_string(),
                    name: name.to_string(),
                    os_version: os_version.clone(),
                    state: state.to_string(),
                });
            }
        }
    }

    if simulators.is_empty() {
        bail!("no available iOS simulators were returned by `xcrun simctl list devices available`");
    }

    if let Some(requested) = requested {
        let mut matches: Vec<_> = simulators
            .into_iter()
            .filter(|simulator| {
                simulator.name == requested.name
                    && ios_versions_match(&requested.os_version, &simulator.os_version)
            })
            .collect();
        matches.sort_by_key(|simulator| simulator.state != "Booted");
        return matches.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!(
                "requested local iOS simulator {} {} was not found; available simulators include: {}",
                requested.name,
                requested.os_version,
                available_ios_simulator_summary(devices)
            )
        });
    }

    simulators.sort_by_key(|simulator| simulator.state != "Booted");
    simulators
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no iOS simulators were available for local profiling"))
}

fn available_ios_simulator_summary(devices: &serde_json::Map<String, Value>) -> String {
    let mut labels = Vec::new();
    for (runtime_key, entries) in devices {
        let Some(os_version) = parse_ios_runtime_version(runtime_key) else {
            continue;
        };
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            if entry
                .get("isAvailable")
                .and_then(Value::as_bool)
                .unwrap_or(true)
                && let Some(name) = entry.get("name").and_then(Value::as_str)
            {
                labels.push(format!("{name} {os_version}"));
            }
        }
    }
    labels.sort();
    labels.dedup();
    labels.into_iter().take(6).collect::<Vec<_>>().join(", ")
}

fn parse_ios_runtime_version(runtime_key: &str) -> Option<String> {
    runtime_key
        .strip_prefix("com.apple.CoreSimulator.SimRuntime.iOS-")
        .map(|value| value.replace('-', "."))
}

fn ios_versions_match(requested: &str, candidate: &str) -> bool {
    let requested = requested.trim();
    let candidate = candidate.trim();
    requested == candidate
        || candidate.starts_with(&format!("{requested}."))
        || requested.starts_with(&format!("{candidate}."))
}

fn ensure_local_ios_simulator_booted(simulator: &LocalIosSimulator) -> Result<()> {
    let output = Command::new("xcrun")
        .args(["simctl", "bootstatus", &simulator.udid, "-b"])
        .output()
        .with_context(|| format!("booting iOS simulator {}", simulator.identifier()))?;
    if !output.status.success() {
        bail!(
            "xcrun simctl bootstatus {} -b failed with status {}\nstdout:\n{}\nstderr:\n{}",
            simulator.udid,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn build_local_ios_simulator_app(
    layout: &crate::ResolvedProjectLayout,
    simulator: &LocalIosSimulator,
) -> Result<PathBuf> {
    let project_path = layout
        .output_dir
        .join("ios")
        .join("BenchRunner")
        .join("BenchRunner.xcodeproj");
    if !project_path.exists() {
        bail!(
            "generated BenchRunner project was not found at {}; run `cargo mobench build --target ios` or rerun the profile build step",
            project_path.display()
        );
    }

    let build_root = layout
        .output_dir
        .join("ios")
        .join("profile-simulator-build");
    let mut cmd = Command::new("xcodebuild");
    cmd.arg("-project")
        .arg(&project_path)
        .arg("-target")
        .arg("BenchRunner")
        .arg("-sdk")
        .arg("iphonesimulator")
        .arg("-configuration")
        .arg("Debug")
        .arg("build")
        .arg(format!("SYMROOT={}", build_root.display()))
        .arg(format!("OBJROOT={}", build_root.display()))
        .arg("CODE_SIGNING_ALLOWED=NO")
        .arg("CODE_SIGNING_REQUIRED=NO");
    let output = cmd.output().with_context(|| {
        format!(
            "building the local iOS BenchRunner simulator app for {}",
            simulator.identifier()
        )
    })?;
    let app_path = build_root
        .join("Debug-iphonesimulator")
        .join("BenchRunner.app");
    if !output.status.success() || !app_path.exists() {
        bail!(
            "xcodebuild simulator build failed for {}\nproject: {}\napp path: {}\nexit status: {}\nstdout:\n{}\nstderr:\n{}",
            simulator.identifier(),
            project_path.display(),
            app_path.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(app_path)
}

fn install_local_ios_app(simulator: &LocalIosSimulator, app_path: &Path) -> Result<()> {
    let output = Command::new("xcrun")
        .args(["simctl", "install", &simulator.udid])
        .arg(app_path)
        .output()
        .with_context(|| {
            format!(
                "installing {} on iOS simulator {}",
                app_path.display(),
                simulator.identifier()
            )
        })?;
    if !output.status.success() {
        bail!(
            "xcrun simctl install failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn local_ios_bundle_identifier(crate_name: &str) -> String {
    format!(
        "dev.world.{}.BenchRunner",
        mobench_sdk::codegen::sanitize_bundle_id_component(crate_name)
    )
}

fn launch_local_ios_app(
    simulator: &LocalIosSimulator,
    bundle_id: &str,
    stdout_path: &Path,
    stderr_path: &Path,
    app_args: &[String],
    app_env: &[(&str, String)],
) -> Result<u32> {
    let stdout_path = absolutize_profile_path(stdout_path)?;
    let stderr_path = absolutize_profile_path(stderr_path)?;

    if let Some(parent) = stdout_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = stderr_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&stdout_path, "")?;
    std::fs::write(&stderr_path, "")?;

    let mut cmd = Command::new("xcrun");
    cmd.args(["simctl", "launch"])
        .arg(format!("--stdout={}", stdout_path.display()))
        .arg(format!("--stderr={}", stderr_path.display()))
        .arg("--terminate-running-process")
        .arg(&simulator.udid)
        .arg(bundle_id);
    for (key, value) in app_env {
        cmd.env(format!("SIMCTL_CHILD_{key}"), value);
    }
    for app_arg in app_args {
        cmd.arg(app_arg);
    }

    let output = cmd.output().with_context(|| {
        format!(
            "launching {} on iOS simulator {}",
            bundle_id,
            simulator.identifier()
        )
    })?;
    if !output.status.success() {
        bail!(
            "xcrun simctl launch failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    parse_simctl_launch_pid(&String::from_utf8_lossy(&output.stdout))
}

fn absolutize_profile_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("resolving absolute path for iOS simulator logs")?
            .join(path))
    }
}

fn parse_simctl_launch_pid(stdout: &str) -> Result<u32> {
    stdout
        .split_whitespace()
        .rev()
        .find_map(|token| token.parse::<u32>().ok())
        .context("simctl launch did not report an application pid")
}

fn terminate_local_ios_app(simulator: &LocalIosSimulator, bundle_id: &str) -> Result<()> {
    let output = Command::new("xcrun")
        .args(["simctl", "terminate", &simulator.udid, bundle_id])
        .output()
        .with_context(|| {
            format!(
                "terminating {} on iOS simulator {}",
                bundle_id,
                simulator.identifier()
            )
        })?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("found nothing to terminate") || stderr.contains("not running") {
        return Ok(());
    }

    bail!(
        "xcrun simctl terminate failed with status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        stderr
    );
}

fn run_ios_sample_capture(pid: u32, output_path: &Path) -> Result<()> {
    let output = Command::new("sample")
        .arg(pid.to_string())
        .arg(DEFAULT_IOS_CAPTURE_DURATION_SECS.to_string())
        .arg("1")
        .arg("-mayDie")
        .arg("-file")
        .arg(output_path)
        .output()
        .with_context(|| format!("sampling iOS simulator process {pid}"))?;
    if !output.status.success() {
        bail!(
            "sample failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn wait_for_ios_log_marker(paths: &[PathBuf], marker: &str, timeout_secs: u64) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        if let Ok(logs) = read_combined_text_files(paths)
            && logs.contains(marker)
        {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    bail!("timed out waiting for iOS log marker `{marker}`");
}

fn read_combined_text_files(paths: &[PathBuf]) -> Result<String> {
    let mut combined = String::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        combined.push_str(
            &std::fs::read_to_string(path)
                .with_context(|| format!("reading text file at {}", path.display()))?,
        );
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }
    Ok(combined)
}

fn write_ios_processed_outputs(
    sample_output: &str,
    processed_root: &Path,
) -> Result<SymbolizationRecord> {
    std::fs::create_dir_all(processed_root)?;
    std::fs::write(processed_root.join("native-report.txt"), sample_output)?;

    match collapse_ios_sample_call_graph(sample_output) {
        Ok((folded_stacks, mut record)) => {
            std::fs::write(processed_root.join("stacks.folded"), &folded_stacks)?;
            if let Some(warning) = write_dual_view_flamegraph_bundle(
                &folded_stacks,
                processed_root,
                "iOS Native Profile",
                IOS_BENCHMARK_ANCHORS,
                "../raw/sample.txt",
                "Raw sample.txt",
            )? {
                record.notes.push(warning);
            }
            if record.tool.is_none() {
                record.tool = Some("sample".into());
            }
            Ok(record)
        }
        Err(error) => {
            std::fs::write(processed_root.join("stacks.folded"), "")?;
            let _ = write_dual_view_flamegraph_bundle(
                "",
                processed_root,
                "iOS Native Profile",
                IOS_BENCHMARK_ANCHORS,
                "../raw/sample.txt",
                "Raw sample.txt",
            )?;
            Ok(SymbolizationRecord {
                status: CaptureStatus::Failed,
                tool: Some("sample".into()),
                resolved_frames: 0,
                unresolved_frames: 0,
                notes: vec![format!("failed to collapse iOS sample call graph: {error}")],
            })
        }
    }
}

#[cfg(test)]
fn collapse_ios_sample_call_graph_to_folded_stacks(sample_output: &str) -> Result<String> {
    collapse_ios_sample_call_graph(sample_output).map(|(folded_stacks, _record)| folded_stacks)
}

fn collapse_ios_sample_call_graph(sample_output: &str) -> Result<(String, SymbolizationRecord)> {
    #[derive(Clone)]
    struct StackFrame {
        indent: usize,
        frame: String,
        unresolved: bool,
    }

    #[derive(Clone)]
    struct ParsedNode {
        depth: usize,
        count: u64,
        frames: Vec<String>,
        unresolved_frames: u64,
    }

    let mut saw_call_graph = false;
    let mut in_call_graph = false;
    let mut stack: Vec<StackFrame> = Vec::new();
    let mut nodes = Vec::new();

    for line in sample_output.lines() {
        let trimmed = line.trim();
        if !in_call_graph {
            if trimmed == "Call graph:" {
                saw_call_graph = true;
                in_call_graph = true;
            }
            continue;
        }
        if trimmed.starts_with("Total number in stack")
            || trimmed.starts_with("Sort by top of stack")
            || trimmed.starts_with("Binary Images:")
        {
            break;
        }
        let Some(parsed) = parse_ios_sample_call_graph_line(line) else {
            continue;
        };
        if parsed.is_thread_root {
            stack.clear();
            continue;
        }

        if !parsed.is_plus {
            while stack
                .last()
                .is_some_and(|existing| existing.indent >= parsed.indent)
            {
                stack.pop();
            }
        }

        stack.push(StackFrame {
            indent: parsed.indent,
            frame: parsed.frame.clone(),
            unresolved: parsed.frame == "???",
        });
        nodes.push(ParsedNode {
            depth: stack.len(),
            count: parsed.count,
            frames: stack.iter().map(|frame| frame.frame.clone()).collect(),
            unresolved_frames: stack.iter().filter(|frame| frame.unresolved).count() as u64,
        });
    }

    if !saw_call_graph {
        bail!("iOS sample output did not contain a `Call graph:` section");
    }
    if nodes.is_empty() {
        bail!("iOS sample output did not contain any callable frames");
    }

    let mut folded_lines = Vec::new();
    let mut resolved_frames = 0_u64;
    let mut unresolved_frames = 0_u64;
    for (index, node) in nodes.iter().enumerate() {
        let next_depth = nodes.get(index + 1).map(|next| next.depth).unwrap_or(0);
        if next_depth > node.depth {
            continue;
        }
        if node.frames.is_empty() {
            continue;
        }
        folded_lines.push(format!("{} {}", node.frames.join(";"), node.count));
        unresolved_frames += node.unresolved_frames;
        resolved_frames += node.frames.len() as u64 - node.unresolved_frames;
    }

    let mut notes = Vec::new();
    let status = if folded_lines.is_empty() {
        notes.push("no leaf frames were emitted from the iOS sample call graph".into());
        CaptureStatus::Failed
    } else if unresolved_frames > 0 {
        notes.push(format!(
            "iOS sample capture retained {unresolved_frames} unresolved frame(s) as `???`"
        ));
        CaptureStatus::Partial
    } else {
        CaptureStatus::Captured
    };

    Ok((
        folded_lines.join("\n"),
        SymbolizationRecord {
            status,
            tool: Some("sample".into()),
            resolved_frames,
            unresolved_frames,
            notes,
        },
    ))
}

struct ParsedIosSampleLine {
    indent: usize,
    count: u64,
    frame: String,
    is_plus: bool,
    is_thread_root: bool,
}

fn parse_ios_sample_call_graph_line(line: &str) -> Option<ParsedIosSampleLine> {
    // `sample` encodes stack depth by the column where the sample count appears.
    // The tree prefix can include `+`, `|`, `!`, and `:` markers, so leading
    // spaces alone are not enough to reconstruct the stack shape.
    let digits_start = line.find(|ch: char| ch.is_ascii_digit())?;
    let indent = digits_start;
    let prefix = &line[..digits_start];
    let is_plus = prefix.trim_end().ends_with('+');
    let remainder = &line[digits_start..];
    let digits_end = remainder.find(|ch: char| !ch.is_ascii_digit())?;
    let count = remainder[..digits_end].parse().ok()?;
    let frame_part = remainder[digits_end..].trim_start();
    let frame = frame_part
        .split("  (in ")
        .next()
        .unwrap_or(frame_part)
        .split("  [")
        .next()
        .unwrap_or(frame_part)
        .trim();
    if frame.is_empty() {
        return None;
    }
    let is_thread_root = frame.starts_with("Thread_");

    Some(ParsedIosSampleLine {
        indent,
        count,
        frame: frame.to_string(),
        is_plus,
        is_thread_root,
    })
}

fn extract_benchmark_reports_from_logs(logs: &str) -> Vec<Value> {
    let mut results = Vec::new();
    if let Some(json) = extract_ios_benchmark_json(logs) {
        results.push(json);
    }

    let marker = "BENCH_JSON ";
    for line in logs.lines() {
        if let Some(index) = line.find(marker) {
            let json_part = &line[index + marker.len()..];
            if let Ok(parsed) = serde_json::from_str::<Value>(json_part) {
                results.push(parsed);
            }
        }
    }

    results
}

fn extract_ios_benchmark_json(logs: &str) -> Option<Value> {
    let start_marker = "BENCH_REPORT_JSON_START";
    let end_marker = "BENCH_REPORT_JSON_END";
    let start_pos = logs.rfind(start_marker)?;
    let after_start = &logs[start_pos + start_marker.len()..];
    let end_pos = after_start.find(end_marker)?;
    extract_ios_json_from_log_section(&after_start[..end_pos])
}

fn extract_ios_json_from_log_section(section: &str) -> Option<Value> {
    let trimmed = section.trim();
    if trimmed.starts_with('{')
        && trimmed.ends_with('}')
        && let Ok(parsed) = serde_json::from_str::<Value>(trimmed)
    {
        return Some(parsed);
    }

    for line in section.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(json_start) = line.find('{')
            && let Some(json) = extract_balanced_json(&line[json_start..])
            && let Ok(parsed) = serde_json::from_str::<Value>(&json)
        {
            return Some(parsed);
        }
    }

    let collapsed: String = section
        .lines()
        .map(|line| {
            if let Some(prefix_end) = line.find("] ") {
                &line[prefix_end + 2..]
            } else {
                line.trim()
            }
        })
        .collect::<Vec<_>>()
        .join("");
    let json_start = collapsed.find('{')?;
    let json = extract_balanced_json(&collapsed[json_start..])?;
    serde_json::from_str(&json).ok()
}

fn extract_balanced_json(input: &str) -> Option<String> {
    if !input.starts_with('{') {
        return None;
    }

    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;
    for (index, ch) in input.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(input[..=index].to_string());
                }
            }
            _ => {}
        }
    }

    None
}

fn benchmark_value_function(value: &Value) -> Option<&str> {
    value.get("function").and_then(Value::as_str).or_else(|| {
        value
            .get("spec")
            .and_then(|spec| spec.get("name"))
            .and_then(Value::as_str)
    })
}

fn select_benchmark_value_for_function<'a>(
    values: &'a [Value],
    function: &str,
) -> Option<&'a Value> {
    let simple_name = function.split("::").last().unwrap_or(function);
    values
        .iter()
        .rev()
        .find(|value| {
            benchmark_value_function(value).is_some_and(|name| {
                name == function
                    || name == simple_name
                    || name.ends_with(&format!("::{simple_name}"))
                    || function.ends_with(&format!("::{name}"))
            })
        })
        .or_else(|| values.last())
}

fn benchmark_value_sample_duration_total_ns(benchmark_value: &Value) -> u64 {
    let sample_objects_total_ns: u64 = benchmark_value
        .get("samples")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|sample| sample.get("duration_ns").and_then(Value::as_u64))
        .sum();
    if sample_objects_total_ns > 0 {
        return sample_objects_total_ns;
    }

    benchmark_value
        .get("samples_ns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .sum()
}

fn populate_semantic_profile_from_benchmark_value(
    manifest: &mut ProfileManifest,
    benchmark_value: &Value,
) {
    let Some(phases) = benchmark_value.get("phases").and_then(Value::as_array) else {
        return;
    };

    let phase_duration_total_ns: u64 = phases
        .iter()
        .filter_map(|phase| phase.get("duration_ns").and_then(Value::as_u64))
        .sum();
    let sample_duration_total_ns = benchmark_value_sample_duration_total_ns(benchmark_value);
    let total_duration_ns = if sample_duration_total_ns > 0 {
        sample_duration_total_ns
    } else {
        phase_duration_total_ns
    };

    let mut semantic_phases = Vec::new();
    let mut partial = false;
    for phase in phases {
        let Some(name) = phase.get("name").and_then(Value::as_str) else {
            partial = true;
            continue;
        };
        let duration_ns = phase.get("duration_ns").and_then(Value::as_u64);
        let percent_total = duration_ns.and_then(|duration_ns| {
            (total_duration_ns > 0).then_some(
                (duration_ns.saturating_mul(100) + (total_duration_ns / 2)) / total_duration_ns,
            )
        });
        if duration_ns.is_none() {
            partial = true;
        }
        semantic_phases.push(SemanticPhaseRecord {
            name: name.to_string(),
            duration_ns,
            percent_total,
        });
    }

    if semantic_phases.is_empty() {
        return;
    }

    manifest.semantic_profile.status = if partial {
        SemanticCaptureStatus::Partial
    } else {
        SemanticCaptureStatus::Captured
    };
    manifest.semantic_profile.phases = semantic_phases;
}

fn merge_semantic_profile_from_bench_report(
    manifest: &mut ProfileManifest,
    bench_report: &Value,
) -> Result<()> {
    populate_semantic_profile_from_benchmark_value(manifest, bench_report);
    Ok(())
}

fn resolve_android_runtime_abi(toolchain: &AndroidProfilerToolchain) -> Result<Option<String>> {
    let primary_abi = read_android_device_property(&toolchain.adb_path, "ro.product.cpu.abi")?;
    if let Some(abi) = primary_abi {
        return Ok(Some(abi));
    }

    let abi_list = read_android_device_property(&toolchain.adb_path, "ro.product.cpu.abilist")?;
    Ok(abi_list.and_then(|value| {
        value
            .split(',')
            .map(str::trim)
            .find(|value| !value.is_empty())
            .map(str::to_owned)
    }))
}

fn read_android_device_property(adb_path: &Path, property: &str) -> Result<Option<String>> {
    let output = Command::new(adb_path)
        .args(["shell", "getprop", property])
        .output()
        .with_context(|| format!("reading Android device property {property}"))?;
    if !output.status.success() {
        bail!(
            "adb shell getprop {property} failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
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

fn build_capture_plan(
    args: &ProfileRunArgs,
    target: &ResolvedProfileTarget,
    output_root: &Path,
) -> Result<ProfileManifest> {
    let backend = target.backend;
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
                    label: "native-report".into(),
                    path: processed_root.join("native-report.txt"),
                },
                ArtifactRecord {
                    label: "benchmark-focused-stacks".into(),
                    path: processed_root.join("benchmark.focused.folded"),
                },
                ArtifactRecord {
                    label: "flamegraph-full-svg".into(),
                    path: processed_root.join("flamegraph.full.svg"),
                },
                ArtifactRecord {
                    label: "flamegraph-focused-svg".into(),
                    path: processed_root.join("flamegraph.focused.svg"),
                },
                ArtifactRecord {
                    label: "flamegraph-viewer".into(),
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
                    label: "native-report".into(),
                    path: processed_root.join("native-report.txt"),
                },
                ArtifactRecord {
                    label: "benchmark-focused-stacks".into(),
                    path: processed_root.join("benchmark.focused.folded"),
                },
                ArtifactRecord {
                    label: "flamegraph-full-svg".into(),
                    path: processed_root.join("flamegraph.full.svg"),
                },
                ArtifactRecord {
                    label: "flamegraph-focused-svg".into(),
                    path: processed_root.join("flamegraph.focused.svg"),
                },
                ArtifactRecord {
                    label: "flamegraph-viewer".into(),
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
        native_capture: NativeCaptureRecord {
            status: CaptureStatus::Planned,
            raw_artifacts,
            processed_artifacts,
            symbolization: SymbolizationRecord::default(),
            viewer_hint,
        },
        semantic_profile: SemanticProfileRecord {
            spans_path: Some(output_root.join("artifacts/semantic/phases.json")),
            ..SemanticProfileRecord::default()
        },
        capture_metadata: CaptureMetadataRecord {
            device: target
                .device
                .as_ref()
                .map(|device| device.identifier.clone()),
            sample_duration_secs: None,
            warmup_mode: resolve_capture_warmup_mode(args.provider, backend, args.warmup_mode),
            capture_method: Some(match backend {
                ProfileBackend::AndroidNative => "simpleperf".into(),
                ProfileBackend::IosInstruments => "sample".into(),
                ProfileBackend::RustTracing => "trace-events".into(),
                ProfileBackend::Auto => unreachable!("auto backend should resolve before planning"),
            }),
            warnings: Vec::new(),
        },
    })
}

fn resolve_capture_warmup_mode(
    provider: ProfileProvider,
    backend: ProfileBackend,
    requested: Option<CaptureWarmupMode>,
) -> Option<CaptureWarmupMode> {
    requested.or(match (provider, backend) {
        (ProfileProvider::Local, ProfileBackend::AndroidNative)
        | (ProfileProvider::Local, ProfileBackend::IosInstruments) => Some(CaptureWarmupMode::Warm),
        _ => None,
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
        manifest.capture_metadata.warnings.push(format!(
            "resolved target device: {} ({}, source: {})",
            device.identifier, device.os, device.source
        ));
    }

    let plan_only_warning = match (args.provider, target.backend) {
        (ProfileProvider::Local, ProfileBackend::AndroidNative) => {
            return execute_capture_with_local_android_executor(
                args,
                manifest,
                execute_local_android_capture,
            );
        }
        (ProfileProvider::Local, ProfileBackend::IosInstruments) => {
            return execute_capture_with_local_ios_executor(
                args,
                manifest,
                execute_local_ios_capture,
            );
        }
        (ProfileProvider::Local, ProfileBackend::RustTracing) => Some(
            "local rust-tracing capture is not implemented yet; this session records the planned trace-events artifact contract only",
        ),
        (ProfileProvider::Browserstack, ProfileBackend::AndroidNative) => {
            bail!(browserstack_native_capture_unsupported_message(
                "android-native",
                "local Android profiling produces simpleperf artifacts and flamegraphs",
            ));
        }
        (ProfileProvider::Browserstack, ProfileBackend::IosInstruments) => {
            bail!(browserstack_native_capture_unsupported_message(
                "ios-instruments",
                "local iOS profiling produces raw sample output (`sample.txt`), collapsed stacks, and `flamegraph.html` from a simulator-hosted capture",
            ));
        }
        (ProfileProvider::Browserstack, ProfileBackend::RustTracing) => {
            bail!(
                "BrowserStack rust-tracing capture is not implemented.\nThis command currently writes a local-first profile contract only.\nUse --provider local for trace-events output, or run a normal BrowserStack benchmark if you only need timing/memory metrics."
            );
        }
        (_, ProfileBackend::Auto) => unreachable!("auto backend should resolve before execution"),
    };

    if let Some(warning) = plan_only_warning {
        manifest.capture_metadata.warnings.push(warning.into());
    }
    Ok(())
}

fn execute_capture_with_local_android_executor<E>(
    args: &ProfileRunArgs,
    manifest: &mut ProfileManifest,
    execute: E,
) -> Result<()>
where
    E: FnOnce(&ProfileRunArgs, &mut ProfileManifest) -> Result<()>,
{
    if let Err(error) = execute(args, manifest) {
        mark_android_capture_attempt_failed(manifest, &error);
        return Err(error);
    }
    Ok(())
}

fn execute_capture_with_local_ios_executor<E>(
    args: &ProfileRunArgs,
    manifest: &mut ProfileManifest,
    execute: E,
) -> Result<()>
where
    E: FnOnce(&ProfileRunArgs, &mut ProfileManifest) -> Result<()>,
{
    if let Err(error) = execute(args, manifest) {
        mark_ios_capture_attempt_failed(manifest, &error);
        return Err(error);
    }
    Ok(())
}

fn mark_android_capture_attempt_failed(manifest: &mut ProfileManifest, error: &anyhow::Error) {
    manifest.native_capture.status = CaptureStatus::Failed;
    manifest.native_capture.symbolization.status = CaptureStatus::Failed;

    let failure_note = format!("local android-native capture failed: {error}");
    if !manifest
        .native_capture
        .symbolization
        .notes
        .iter()
        .any(|note| note == &failure_note)
    {
        manifest
            .native_capture
            .symbolization
            .notes
            .push(failure_note.clone());
    }
    if !manifest
        .capture_metadata
        .warnings
        .iter()
        .any(|warning| warning == &failure_note)
    {
        manifest.capture_metadata.warnings.push(failure_note);
    }
}

fn mark_ios_capture_attempt_failed(manifest: &mut ProfileManifest, error: &anyhow::Error) {
    manifest.native_capture.status = CaptureStatus::Failed;
    manifest.native_capture.symbolization.status = CaptureStatus::Failed;

    let failure_note = format!("local ios-instruments capture failed: {error}");
    if !manifest
        .native_capture
        .symbolization
        .notes
        .iter()
        .any(|note| note == &failure_note)
    {
        manifest
            .native_capture
            .symbolization
            .notes
            .push(failure_note.clone());
    }
    if !manifest
        .capture_metadata
        .warnings
        .iter()
        .any(|warning| warning == &failure_note)
    {
        manifest.capture_metadata.warnings.push(failure_note);
    }
}

fn browserstack_native_capture_unsupported_message(
    backend_label: &str,
    artifact_guidance: &str,
) -> String {
    format!(
        "BrowserStack native profiling is not implemented for {backend_label}.\nThis command currently writes a local-first profile contract only.\nUse --provider local for planning/local capture, or run a normal BrowserStack benchmark if you only need timing/memory metrics.\n{artifact_guidance}."
    )
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
                    "Open artifacts/processed/flamegraph.html for the interactive dual-view flamegraph explorer".into(),
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
                    "Open artifacts/processed/flamegraph.html for the interactive dual-view flamegraph explorer".into(),
                )
            } else if !raw_artifacts.is_empty() {
                Some("Inspect artifacts/raw/sample.txt for the raw iOS sample call graph".into())
            } else if !processed_artifacts.is_empty() {
                Some(
                    "Open artifacts/processed/flamegraph.html for the interactive dual-view flamegraph explorer".into(),
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
            config: None,
            output_dir: PathBuf::from("target/mobench/profile"),
            device: None,
            os_version: None,
            profile: None,
            device_matrix: None,
            provider,
            backend,
            format,
            warmup_mode: None,
        }
    }

    #[test]
    fn local_native_profiles_default_to_warm_capture_mode() {
        let android_target = resolve_profile_target(&sample_run_args(
            MobileTarget::Android,
            ProfileProvider::Local,
            ProfileBackend::AndroidNative,
            ProfileFormat::Both,
        ))
        .expect("resolve android target");
        let android_plan = build_capture_plan(
            &sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::AndroidNative,
                ProfileFormat::Both,
            ),
            &android_target,
            &PathBuf::from("target/mobench/profile"),
        )
        .expect("build android plan");

        assert_eq!(
            android_plan.capture_metadata.warmup_mode,
            Some(CaptureWarmupMode::Warm)
        );
    }

    #[test]
    fn explicit_capture_warmup_mode_overrides_local_default() {
        let mut args = sample_run_args(
            MobileTarget::Android,
            ProfileProvider::Local,
            ProfileBackend::AndroidNative,
            ProfileFormat::Both,
        );
        args.warmup_mode = Some(CaptureWarmupMode::Cold);
        let target = resolve_profile_target(&args).expect("resolve target");
        let plan = build_capture_plan(&args, &target, &PathBuf::from("target/mobench/profile"))
            .expect("build plan");

        assert_eq!(
            plan.capture_metadata.warmup_mode,
            Some(CaptureWarmupMode::Cold)
        );
    }

    #[test]
    fn android_warmup_log_marker_detection_uses_bench_json_marker() {
        assert!(android_log_contains_marker(
            "03-26 19:00:00.000 BenchRunner I BENCH_JSON {\"samples_ns\":[1,2]}",
            ANDROID_BENCH_LOG_MARKER
        ));
        assert!(!android_log_contains_marker(
            "03-26 19:00:00.000 BenchRunner I unrelated log line",
            ANDROID_BENCH_LOG_MARKER
        ));
    }

    #[test]
    fn benchmark_logs_extract_android_bench_json_reports() {
        let reports = extract_benchmark_reports_from_logs(
            "03-26 19:00:00.000 BenchRunner I BENCH_JSON {\"function\":\"sample_fns::fibonacci\",\"phases\":[{\"name\":\"prove\",\"duration_ns\":90},{\"name\":\"serialize\",\"duration_ns\":10}]}",
        );

        assert_eq!(reports.len(), 1);
        assert_eq!(
            benchmark_value_function(&reports[0]),
            Some("sample_fns::fibonacci")
        );
    }

    #[test]
    fn semantic_profile_populates_from_benchmark_phase_payload() {
        let mut manifest = build_capture_plan(
            &sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::AndroidNative,
                ProfileFormat::Both,
            ),
            &resolve_profile_target(&sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::AndroidNative,
                ProfileFormat::Both,
            ))
            .expect("resolve target"),
            &PathBuf::from("target/mobench/profile"),
        )
        .expect("build plan");

        populate_semantic_profile_from_benchmark_value(
            &mut manifest,
            &serde_json::json!({
                "function": "sample_fns::fibonacci",
                "phases": [
                    {"name": "prove", "duration_ns": 90},
                    {"name": "serialize", "duration_ns": 10}
                ]
            }),
        );

        assert_eq!(
            manifest.semantic_profile.status,
            SemanticCaptureStatus::Captured
        );
        assert_eq!(manifest.semantic_profile.phases.len(), 2);
        assert_eq!(manifest.semantic_profile.phases[0].name, "prove");
        assert_eq!(manifest.semantic_profile.phases[0].duration_ns, Some(90));
        assert_eq!(manifest.semantic_profile.phases[0].percent_total, Some(90));
        assert_eq!(manifest.semantic_profile.phases[1].percent_total, Some(10));
    }

    #[test]
    fn semantic_profile_uses_samples_ns_totals_for_android_log_payloads() {
        let mut manifest = build_capture_plan(
            &sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::AndroidNative,
                ProfileFormat::Both,
            ),
            &resolve_profile_target(&sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::AndroidNative,
                ProfileFormat::Both,
            ))
            .expect("resolve target"),
            &PathBuf::from("target/mobench/profile"),
        )
        .expect("build plan");

        merge_semantic_profile_from_bench_report(
            &mut manifest,
            &serde_json::json!({
                "function": "sample_fns::fibonacci",
                "samples_ns": [100, 300],
                "phases": [
                    {"name": "prove", "duration_ns": 320},
                    {"name": "serialize", "duration_ns": 40}
                ]
            }),
        )
        .expect("merge semantic profile");

        assert_eq!(
            manifest.semantic_profile.status,
            SemanticCaptureStatus::Captured
        );
        assert_eq!(manifest.semantic_profile.phases[0].percent_total, Some(80));
        assert_eq!(manifest.semantic_profile.phases[1].percent_total, Some(10));
    }

    #[test]
    fn write_profile_session_outputs_persists_semantic_phase_sidecar() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut manifest = sample_manifest();
        manifest.semantic_profile.spans_path = Some(
            dir.path()
                .join("android-sample/artifacts/semantic/phases.json"),
        );
        let args = ProfileRunArgs {
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::AndroidNative,
            format: ProfileFormat::Both,
            output_dir: dir.path().to_path_buf(),
            crate_path: None,
            device: None,
            os_version: None,
            profile: None,
            device_matrix: None,
            config: None,
            warmup_mode: Some(CaptureWarmupMode::Warm),
        };
        let run_output_dir = dir.path().join("android-sample");

        write_profile_session_outputs(&args, &run_output_dir, &manifest)
            .expect("write profile outputs");

        let sidecar = std::fs::read_to_string(
            manifest
                .semantic_profile
                .spans_path
                .as_ref()
                .expect("semantic spans path"),
        )
        .expect("read semantic sidecar");
        assert!(sidecar.contains("\"prove\""));
        assert!(sidecar.contains("\"serialize\""));
    }

    #[test]
    fn profile_manifest_serializes_partial_failure_state() {
        let manifest = sample_manifest();

        let json = serde_json::to_value(&manifest).expect("serialize manifest");
        assert_eq!(json["capture_metadata"]["warnings"][0], "missing symbols");
        assert_eq!(json["native_capture"]["status"], "partial");
    }

    #[test]
    fn render_profile_summary_mentions_backend_and_artifacts() {
        let manifest = sample_manifest();
        let markdown = render_profile_markdown(&manifest);

        assert!(markdown.contains("android-native"));
        assert!(markdown.contains("## Native capture"));
        assert!(markdown.contains("artifacts/raw/sample.perf"));
        assert!(markdown.contains("## Semantic phases"));
        assert!(markdown.contains("missing symbols"));
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
        assert!(
            json["native_capture"].get("viewer_hint").is_some(),
            "expected native capture metadata to include viewer hints, got: {json}"
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
    fn legacy_profile_manifest_deserializes_into_nested_sections() {
        let legacy = serde_json::json!({
            "run_id": "run-123",
            "target": "android",
            "function": "sample_fns::fibonacci",
            "provider": "local",
            "backend": "android-native",
            "format": "both",
            "capture_status": "partial",
            "raw_artifacts": [
                {"label": "simpleperf", "path": "artifacts/raw/sample.perf"}
            ],
            "processed_artifacts": [
                {"label": "flamegraph", "path": "artifacts/processed/flamegraph.html"}
            ],
            "warnings": ["legacy manifest"],
            "viewer_hint": "Open flamegraph.html in a browser"
        });

        let manifest: ProfileManifest =
            serde_json::from_value(legacy).expect("deserialize legacy manifest");

        assert_eq!(manifest.native_capture.status, CaptureStatus::Partial);
        assert_eq!(manifest.native_capture.raw_artifacts.len(), 1);
        assert_eq!(manifest.native_capture.processed_artifacts.len(), 1);
        assert_eq!(
            manifest.native_capture.viewer_hint.as_deref(),
            Some("Open flamegraph.html in a browser")
        );
        assert_eq!(manifest.capture_metadata.warnings, vec!["legacy manifest"]);
        assert_eq!(
            manifest.semantic_profile.status,
            SemanticCaptureStatus::Planned
        );
    }

    #[test]
    fn render_profile_summary_separates_native_and_semantic_outputs() {
        let markdown = render_profile_markdown(&sample_manifest());

        assert!(
            markdown.contains("artifacts/raw/sample.perf")
                || markdown.contains("artifacts/processed/flamegraph.html"),
            "expected native capture output to remain visible, got:\n{markdown}"
        );
        assert!(
            markdown.contains("Semantic phases"),
            "expected semantic phases to be rendered separately, got:\n{markdown}"
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
    fn build_capture_plan_reserves_semantic_phase_sidecar_path() {
        let args = sample_run_args(
            MobileTarget::Android,
            ProfileProvider::Local,
            ProfileBackend::AndroidNative,
            ProfileFormat::Both,
        );
        let target = resolve_profile_target(&args).expect("resolve target");
        let manifest = build_capture_plan(&args, &target, &PathBuf::from("target/mobench/profile"))
            .expect("build capture plan");

        assert_eq!(
            manifest.semantic_profile.spans_path,
            Some(PathBuf::from(
                "target/mobench/profile/artifacts/semantic/phases.json"
            ))
        );
    }

    #[test]
    fn semantic_profile_ingests_phase_timings_from_bench_report_json() {
        let args = sample_run_args(
            MobileTarget::Android,
            ProfileProvider::Local,
            ProfileBackend::AndroidNative,
            ProfileFormat::Both,
        );
        let target = resolve_profile_target(&args).expect("resolve target");
        let mut manifest =
            build_capture_plan(&args, &target, &PathBuf::from("target/mobench/profile"))
                .expect("build capture plan");
        let bench_report = serde_json::json!({
            "spec": {
                "name": "sample_fns::fibonacci",
                "iterations": 2,
                "warmup": 1
            },
            "samples": [
                {"duration_ns": 100},
                {"duration_ns": 300}
            ],
            "phases": [
                {"name": "prove", "duration_ns": 320},
                {"name": "serialize", "duration_ns": 40}
            ]
        });

        populate_semantic_profile_from_benchmark_value(&mut manifest, &bench_report);

        assert_eq!(
            manifest.semantic_profile.status,
            SemanticCaptureStatus::Captured
        );
        assert_eq!(manifest.semantic_profile.phases.len(), 2);
        assert_eq!(manifest.semantic_profile.phases[0].name, "prove");
        assert_eq!(manifest.semantic_profile.phases[0].duration_ns, Some(320));
        assert_eq!(manifest.semantic_profile.phases[0].percent_total, Some(80));
        assert_eq!(manifest.semantic_profile.phases[1].name, "serialize");
        assert_eq!(manifest.semantic_profile.phases[1].percent_total, Some(10));
    }

    #[test]
    fn android_native_offsets_are_symbolized_into_rust_frames() {
        let (symbolized, record, report) = symbolize_android_folded_stacks_with_resolver(
            "dev.world.samplefns;uniffi.sample_fns.Sample_fnsKt.runBenchmark;libsample_fns.so[+94138] 1",
            |library_name, offset| {
                if library_name == "libsample_fns.so" && offset == 94_138 {
                    Some("sample_fns::fibonacci".into())
                } else {
                    None
                }
            },
        );

        assert!(symbolized.contains("sample_fns::fibonacci"));
        assert_eq!(record.status, CaptureStatus::Captured);
        assert_eq!(record.resolved_frames, 1);
        assert_eq!(record.unresolved_frames, 0);
        assert!(report.contains("sample_fns::fibonacci"));
    }

    #[test]
    fn android_native_offsets_use_runtime_abi_to_select_unstripped_library_paths() {
        let unstripped_path =
            PathBuf::from("/cargo/target/aarch64-linux-android/release/libsample_fns.so");
        let packaged_path = PathBuf::from("/apk/jniLibs/arm64-v8a/libsample_fns.so");
        let other_unstripped_path =
            PathBuf::from("/cargo/target/x86_64-linux-android/release/libsample_fns.so");
        let other_packaged_path = PathBuf::from("/apk/jniLibs/x86_64/libsample_fns.so");
        let native_libraries = vec![
            NativeLibraryArtifact {
                abi: "arm64-v8a".into(),
                library_name: "libsample_fns.so".into(),
                unstripped_path: unstripped_path.clone(),
                packaged_path,
            },
            NativeLibraryArtifact {
                abi: "x86_64".into(),
                library_name: "libsample_fns.so".into(),
                unstripped_path: other_unstripped_path.clone(),
                packaged_path: other_packaged_path,
            },
        ];
        let mut seen_paths = Vec::new();

        let (symbolized, record, report) = symbolize_android_folded_stacks_with_native_libraries(
            "dev.world.samplefns;libsample_fns.so[+94138] 1",
            &native_libraries,
            Some("x86_64"),
            |path, offset| {
                seen_paths.push((path.to_path_buf(), offset));
                Some("sample_fns::fibonacci".into())
            },
        );

        assert!(symbolized.contains("sample_fns::fibonacci"));
        assert_eq!(seen_paths.len(), 1);
        assert_eq!(seen_paths[0].0, other_unstripped_path);
        assert_eq!(seen_paths[0].1, 94_138);
        assert_eq!(record.status, CaptureStatus::Captured);
        assert_eq!(record.resolved_frames, 1);
        assert!(report.contains("sample_fns::fibonacci"));
    }

    #[test]
    fn android_native_offsets_do_not_collapse_multiple_abis_without_a_runtime_selection() {
        let native_libraries = vec![
            NativeLibraryArtifact {
                abi: "arm64-v8a".into(),
                library_name: "libsample_fns.so".into(),
                unstripped_path: PathBuf::from(
                    "/cargo/target/aarch64-linux-android/release/libsample_fns.so",
                ),
                packaged_path: PathBuf::from("/apk/jniLibs/arm64-v8a/libsample_fns.so"),
            },
            NativeLibraryArtifact {
                abi: "x86_64".into(),
                library_name: "libsample_fns.so".into(),
                unstripped_path: PathBuf::from(
                    "/cargo/target/x86_64-linux-android/release/libsample_fns.so",
                ),
                packaged_path: PathBuf::from("/apk/jniLibs/x86_64/libsample_fns.so"),
            },
        ];
        let mut seen_paths = Vec::new();

        let (symbolized, record, report) = symbolize_android_folded_stacks_with_native_libraries(
            "dev.world.samplefns;libsample_fns.so[+94138] 1",
            &native_libraries,
            None,
            |path, offset| {
                seen_paths.push((path.to_path_buf(), offset));
                Some("sample_fns::fibonacci".into())
            },
        );

        assert!(symbolized.contains("libsample_fns.so[+94138]"));
        assert!(report.contains("libsample_fns.so[+94138]"));
        assert!(seen_paths.is_empty());
        assert!(!symbolized.contains("sample_fns::fibonacci"));
        assert_eq!(record.status, CaptureStatus::Failed);
        assert_eq!(record.resolved_frames, 0);
        assert_eq!(record.unresolved_frames, 1);
    }

    #[test]
    fn android_post_processing_writes_symbolized_outputs_before_flamegraph_rendering() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let processed_root = temp_dir.path().join("artifacts/processed");
        let native_libraries = vec![NativeLibraryArtifact {
            abi: "arm64-v8a".into(),
            library_name: "libsample_fns.so".into(),
            unstripped_path: PathBuf::from(
                "/cargo/target/aarch64-linux-android/release/libsample_fns.so",
            ),
            packaged_path: PathBuf::from("/apk/jniLibs/arm64-v8a/libsample_fns.so"),
        }];

        let record = write_android_symbolized_outputs_with_resolver(
            "dev.world.samplefns;libsample_fns.so[+94138] 1",
            &native_libraries,
            &processed_root,
            Some("arm64-v8a"),
            |_path, offset| {
                if offset == 94_138 {
                    Some("sample_fns::fibonacci".into())
                } else {
                    None
                }
            },
        )
        .expect("write symbolized outputs");

        let folded = std::fs::read_to_string(processed_root.join("stacks.folded"))
            .expect("read stacks.folded");
        let report = std::fs::read_to_string(processed_root.join("native-report.txt"))
            .expect("read native report");
        let flamegraph = std::fs::read_to_string(processed_root.join("flamegraph.html"))
            .expect("read flamegraph");

        assert!(folded.contains("sample_fns::fibonacci"));
        assert!(report.contains("sample_fns::fibonacci"));
        assert!(flamegraph.contains("<svg"));
        assert_eq!(record.status, CaptureStatus::Captured);
        assert_eq!(record.resolved_frames, 1);
        assert_eq!(record.unresolved_frames, 0);
    }

    #[test]
    fn flamegraph_html_defaults_to_viewport_width_for_standalone_svg() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let output_path = temp_dir.path().join("flamegraph.svg");

        let flamegraph =
            render_standalone_flamegraph_svg("root;sample_fns::fibonacci 1", "Test Flamegraph")
                .expect("render flamegraph");
        std::fs::write(&output_path, flamegraph).expect("write flamegraph");

        let flamegraph = std::fs::read_to_string(&output_path).expect("read flamegraph");

        assert!(
            flamegraph.contains("var fluiddrawing = false;"),
            "expected standalone flamegraph HTML to disable inferno's fluiddrawing script for file:// rendering, got:\n{flamegraph}"
        );
        assert!(
            flamegraph.contains("width:100vw")
                || flamegraph.contains("min-width:100vw")
                || flamegraph.contains("max-width:100vw"),
            "expected standalone flamegraph HTML to size the SVG to the viewport width, got:\n{flamegraph}"
        );
    }

    #[test]
    fn android_ndk_addr2line_discovery_prefers_ndk_toolchain_bin() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let ndk_root = temp_dir.path().join("ndk/26.3.11579264");
        let tool_path = ndk_root
            .join("toolchains")
            .join("llvm")
            .join("prebuilt")
            .join("darwin-x86_64")
            .join("bin")
            .join(if cfg!(windows) {
                "llvm-addr2line.exe"
            } else {
                "llvm-addr2line"
            });
        std::fs::create_dir_all(tool_path.parent().expect("tool parent")).expect("create tool dir");
        std::fs::write(&tool_path, "#!/bin/sh\n").expect("write tool");

        let discovered = locate_android_llvm_addr2line(&ndk_root, None).expect("discover tool");

        assert_eq!(discovered, tool_path);
    }

    #[test]
    fn android_ndk_addr2line_discovery_honors_explicit_override() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let override_path = temp_dir.path().join("custom-llvm-addr2line");
        std::fs::write(&override_path, "#!/bin/sh\n").expect("write override");

        let discovered =
            locate_android_llvm_addr2line(Path::new("/does/not/matter"), Some(&override_path))
                .expect("discover override");

        assert_eq!(discovered, override_path);
    }

    #[test]
    fn local_android_attempted_capture_marks_failed_state() {
        let args = sample_run_args(
            MobileTarget::Android,
            ProfileProvider::Local,
            ProfileBackend::AndroidNative,
            ProfileFormat::Both,
        );
        let target = resolve_profile_target(&args).expect("resolve target");
        let mut manifest =
            build_capture_plan(&args, &target, &PathBuf::from("target/mobench/profile"))
                .expect("build capture plan");

        let error = execute_capture_with_local_android_executor(
            &args,
            &mut manifest,
            |_args, _manifest| anyhow::bail!("simulated android capture failure"),
        )
        .expect_err("simulated capture failure");

        assert!(
            error
                .to_string()
                .contains("simulated android capture failure")
        );
        assert_eq!(manifest.native_capture.status, CaptureStatus::Failed);
        assert_eq!(
            manifest.native_capture.symbolization.status,
            CaptureStatus::Failed
        );
        assert_eq!(manifest.native_capture.symbolization.tool, None);
        assert!(
            manifest
                .capture_metadata
                .warnings
                .iter()
                .any(|warning| warning.contains("simulated android capture failure"))
        );
    }

    #[test]
    fn profile_session_writes_failed_android_manifest_after_attempted_execution() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut args = sample_run_args(
            MobileTarget::Android,
            ProfileProvider::Local,
            ProfileBackend::AndroidNative,
            ProfileFormat::Both,
        );
        args.output_dir = dir.path().to_path_buf();

        let error = run_profile_session_with_executor(&args, false, |args, _target, manifest| {
            execute_capture_with_local_android_executor(args, manifest, |_args, _manifest| {
                anyhow::bail!("simulated android capture failure")
            })
        })
        .expect_err("simulated execution failure should bubble up");

        assert!(
            error
                .to_string()
                .contains("simulated android capture failure")
        );

        let run_dir = dir.path().join(build_run_id(args.target, &args.function));
        let manifest = load_profile_manifest(&run_dir.join("profile.json"))
            .expect("load failed profile manifest");

        assert_eq!(manifest.native_capture.status, CaptureStatus::Failed);
        assert_eq!(
            manifest.native_capture.symbolization.status,
            CaptureStatus::Failed
        );
        assert_eq!(manifest.native_capture.symbolization.tool, None);
        assert!(
            manifest
                .capture_metadata
                .warnings
                .iter()
                .any(|warning| warning.contains("simulated android capture failure"))
        );
        assert!(run_dir.join("summary.md").exists());
        assert!(dir.path().join("profile.json").exists());
        assert!(dir.path().join("summary.md").exists());
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
            &resolve_profile_target(&sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::AndroidNative,
                ProfileFormat::Both,
            ))
            .expect("resolve target"),
            &PathBuf::from("target/mobench/profile"),
        )
        .expect("android capture plan");

        assert!(
            plan.native_capture
                .raw_artifacts
                .iter()
                .any(|p| p.path.ends_with("sample.perf"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("flamegraph.html"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("benchmark.focused.folded"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("flamegraph.full.svg"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("flamegraph.focused.svg"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("stacks.folded"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("native-report.txt"))
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
            &resolve_profile_target(&sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::AndroidNative,
                ProfileFormat::Native,
            ))
            .expect("resolve target"),
            &PathBuf::from("target/mobench/profile"),
        )
        .expect("native-only capture plan");

        assert_eq!(plan.native_capture.raw_artifacts.len(), 1);
        assert!(plan.native_capture.processed_artifacts.is_empty());
        assert_eq!(
            plan.native_capture.viewer_hint.as_deref(),
            Some("Inspect artifacts/raw/sample.perf with the Android profiling toolchain")
        );
    }

    #[test]
    fn ios_backend_allocates_sample_and_flamegraph_artifacts() {
        let plan = build_capture_plan(
            &sample_run_args(
                MobileTarget::Ios,
                ProfileProvider::Local,
                ProfileBackend::IosInstruments,
                ProfileFormat::Both,
            ),
            &resolve_profile_target(&sample_run_args(
                MobileTarget::Ios,
                ProfileProvider::Local,
                ProfileBackend::IosInstruments,
                ProfileFormat::Both,
            ))
            .expect("resolve target"),
            &PathBuf::from("target/mobench/profile"),
        )
        .expect("ios capture plan");

        assert!(
            plan.native_capture
                .raw_artifacts
                .iter()
                .any(|p| p.path.ends_with("sample.txt"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("stacks.folded"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("native-report.txt"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("flamegraph.html"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("benchmark.focused.folded"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("flamegraph.full.svg"))
        );
        assert!(
            plan.native_capture
                .processed_artifacts
                .iter()
                .any(|p| p.path.ends_with("flamegraph.focused.svg"))
        );
    }

    #[test]
    fn ios_sample_call_graph_collapses_into_folded_stacks() {
        let sample = r#"Call graph:
    778 Thread_27177597   DispatchQueue_1: com.apple.main-thread  (serial)
      778 start  (in dyld) + 7184  [0x18ac81d54]
        776 uniffi_sample_fns_fn_func_run_benchmark  (in sample_fns) + 88  [0x100000588]
        + 776 sample_fns::run_benchmark  (in sample_fns) + 40  [0x100000610]
        + 776 mobench_sdk::timing::run_closure  (in sample_fns) + 24  [0x100000650]
        + 776 sample_fns::fibonacci_batch  (in sample_fns) + 24  [0x100000710]
        + 776 sample_fns::fibonacci  (in sample_fns) + 24  [0x100000780]
        2 write  (in libsystem_kernel.dylib) + 40  [0x18b00c840]
"#;

        let folded =
            collapse_ios_sample_call_graph_to_folded_stacks(sample).expect("collapse sample");

        assert!(folded.contains(
            "start;uniffi_sample_fns_fn_func_run_benchmark;sample_fns::run_benchmark;mobench_sdk::timing::run_closure;sample_fns::fibonacci_batch;sample_fns::fibonacci 776"
        ));
        assert!(folded.contains("start;write 2"));
    }

    #[test]
    fn ios_sample_call_graph_preserves_rust_branch_with_sample_tree_markers() {
        let sample = r#"Call graph:
    7863 Thread_27276912: Main Thread   DispatchQueue_<multiple>
    +           ! : 8 runBenchmark(spec:)  (in BenchRunner) + 212  [0x104c8fd04]  sample_fns.swift:883
    +           ! :   8 rustCallWithError<A, B>(_:_:)  (in BenchRunner) + 136  [0x104c88e18]  sample_fns.swift:277
    +           ! :     8 makeRustCall<A, B>(_:errorHandler:)  (in BenchRunner) + 272  [0x104c889d4]  sample_fns.swift:286
    +           ! :       8 closure #1 in runBenchmark(spec:)  (in BenchRunner) + 196  [0x104c8ff74]  sample_fns.swift:884
    +           ! :         8 uniffi_sample_fns_fn_func_run_benchmark  (in BenchRunner) + 128  [0x104ca0048]
    +           ! :           8 uniffi_core::ffi::rustcalls::rust_call::hd7f37ba68899eb94  (in BenchRunner) + 60  [0x104c9e050]
    +           ! :             8 uniffi_core::ffi::rustcalls::rust_call_with_out_status::hb407fdd2dbf3b59b  (in BenchRunner) + 60  [0x104c9dbc8]
    +           ! :               8 std::panic::catch_unwind::h37b9566b8b963094  (in BenchRunner) + 96  [0x104c9aba4]
    +           ! :                 8 __rust_try  (in BenchRunner) + 32  [0x104c9ac48]
    +           ! :                   8 std::panicking::catch_unwind::do_call::h426d206e0216d0d8  (in BenchRunner) + 64  [0x104ca1400]
    +           ! :                     8 sample_fns::uniffi_sample_fns_fn_func_run_benchmark::_$u7b$$u7b$closure$u7d$$u7d$::h239802906291ec5b  (in BenchRunner) + 180  [0x104c96a1c]
    +           ! :                       8 sample_fns::run_benchmark::h9909bea304da6ad4  (in BenchRunner) + 244  [0x104c9ecf8]
    +           ! :                         | 6 sample_fns::run_benchmark::_$u7b$$u7b$closure$u7d$$u7d$::h93f4e9319d117771  (in BenchRunner) + 40  [0x104c96648]
    +           ! :                         |   6 mobench_sdk::timing::profile_phase::hea85f2c7c3e95291  (in BenchRunner) + 116  [0x104ca0d80]
    +           ! :                         |     6 sample_fns::run_benchmark::_$u7b$$u7b$closure$u7d$$u7d$::_$u7b$$u7b$closure$u7d$$u7d$::h4716261690d4fa31  (in BenchRunner) + 24  [0x104c96800]
    +           ! :                         |       6 sample_fns::fibonacci_batch::hc8a1ee7297b9bb66  (in BenchRunner) + 80  [0x104c9f074]
    +           ! :                         |         5 sample_fns::fibonacci::ha1ebbae54edac99d  (in BenchRunner) + 152  [0x104c9f168]
"#;

        let folded =
            collapse_ios_sample_call_graph_to_folded_stacks(sample).expect("collapse sample");

        assert!(
            folded.contains(
                "runBenchmark(spec:);rustCallWithError<A, B>(_:_:);makeRustCall<A, B>(_:errorHandler:);closure #1 in runBenchmark(spec:);uniffi_sample_fns_fn_func_run_benchmark;uniffi_core::ffi::rustcalls::rust_call::hd7f37ba68899eb94;uniffi_core::ffi::rustcalls::rust_call_with_out_status::hb407fdd2dbf3b59b;std::panic::catch_unwind::h37b9566b8b963094;__rust_try;std::panicking::catch_unwind::do_call::h426d206e0216d0d8;sample_fns::uniffi_sample_fns_fn_func_run_benchmark::_$u7b$$u7b$closure$u7d$$u7d$::h239802906291ec5b;sample_fns::run_benchmark::h9909bea304da6ad4;sample_fns::run_benchmark::_$u7b$$u7b$closure$u7d$$u7d$::h93f4e9319d117771;mobench_sdk::timing::profile_phase::hea85f2c7c3e95291;sample_fns::run_benchmark::_$u7b$$u7b$closure$u7d$$u7d$::_$u7b$$u7b$closure$u7d$$u7d$::h4716261690d4fa31;sample_fns::fibonacci_batch::hc8a1ee7297b9bb66;sample_fns::fibonacci::ha1ebbae54edac99d 5"
            ),
            "expected folded stacks to preserve the deep Rust branch emitted by `sample`, got:\n{folded}"
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
            build_capture_plan(&args, &target, &PathBuf::from("target/mobench/profile"))
                .expect("plan");
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
            build_capture_plan(&args, &target, &PathBuf::from("target/mobench/profile"))
                .expect("plan");
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
                || message.contains("time-profiler.trace")
                || message.contains("time-profiler.xml")
                || message.contains("flamegraph"),
            "expected the error to clarify the iOS artifact story, got: {message}"
        );
    }

    #[test]
    fn profile_rust_tracing_processed_only_is_rejected() {
        let target = resolve_profile_target(&sample_run_args(
            MobileTarget::Android,
            ProfileProvider::Local,
            ProfileBackend::RustTracing,
            ProfileFormat::Both,
        ))
        .expect("resolve target");
        let error = build_capture_plan(
            &sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Local,
                ProfileBackend::RustTracing,
                ProfileFormat::Processed,
            ),
            &target,
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
        run_profile_session_with_executor(&ios_args, false, |_args, _target, _manifest| Ok(()))
            .expect("write second profile session");

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
            &resolve_profile_target(&sample_run_args(
                MobileTarget::Android,
                ProfileProvider::Browserstack,
                ProfileBackend::RustTracing,
                ProfileFormat::Both,
            ))
            .expect("resolve target"),
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
        assert_eq!(manifest.native_capture.status, CaptureStatus::Planned);
        assert!(
            manifest
                .capture_metadata
                .warnings
                .iter()
                .any(|warning| warning.contains("dry-run enabled")),
            "expected dry-run warning in manifest: {:?}",
            manifest.capture_metadata.warnings
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
        let target = ResolvedProfileTarget {
            backend: ProfileBackend::AndroidNative,
            device: Some(ResolvedProfileDevice {
                name: "Pixel 7".into(),
                os: "android".into(),
                os_version: "13".into(),
                identifier: "Pixel 7-13.0".into(),
                profile: Some("high-spec".into()),
                source: "matrix".into(),
            }),
        };
        ProfileManifest {
            run_id: "run-123".into(),
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::AndroidNative,
            format: ProfileFormat::Both,
            native_capture: NativeCaptureRecord {
                status: CaptureStatus::Partial,
                raw_artifacts: vec![ArtifactRecord {
                    label: "simpleperf".into(),
                    path: PathBuf::from("artifacts/raw/sample.perf"),
                }],
                processed_artifacts: vec![
                    ArtifactRecord {
                        label: "collapsed-stacks".into(),
                        path: PathBuf::from("artifacts/processed/stacks.folded"),
                    },
                    ArtifactRecord {
                        label: "benchmark-focused-stacks".into(),
                        path: PathBuf::from("artifacts/processed/benchmark.focused.folded"),
                    },
                    ArtifactRecord {
                        label: "native-report".into(),
                        path: PathBuf::from("artifacts/processed/native-report.txt"),
                    },
                    ArtifactRecord {
                        label: "flamegraph-full-svg".into(),
                        path: PathBuf::from("artifacts/processed/flamegraph.full.svg"),
                    },
                    ArtifactRecord {
                        label: "flamegraph-focused-svg".into(),
                        path: PathBuf::from("artifacts/processed/flamegraph.focused.svg"),
                    },
                    ArtifactRecord {
                        label: "flamegraph-viewer".into(),
                        path: PathBuf::from("artifacts/processed/flamegraph.html"),
                    },
                ],
                symbolization: SymbolizationRecord {
                    status: CaptureStatus::Partial,
                    tool: Some("llvm-addr2line".into()),
                    resolved_frames: 3,
                    unresolved_frames: 1,
                    notes: vec!["missing symbols".into()],
                },
                viewer_hint: Some(
                    "Open artifacts/processed/flamegraph.html for the interactive dual-view flamegraph explorer"
                        .into(),
                ),
            },
            semantic_profile: SemanticProfileRecord {
                status: SemanticCaptureStatus::Captured,
                phases: vec![
                    SemanticPhaseRecord {
                        name: "prove".into(),
                        duration_ns: Some(120_000),
                        percent_total: None,
                    },
                    SemanticPhaseRecord {
                        name: "serialize".into(),
                        duration_ns: Some(8_000),
                        percent_total: None,
                    },
                ],
                spans_path: Some(PathBuf::from("artifacts/semantic/spans.json")),
            },
            capture_metadata: CaptureMetadataRecord {
                device: target
                    .device
                    .as_ref()
                    .map(|device| device.identifier.clone()),
                sample_duration_secs: Some(15),
                warmup_mode: Some(CaptureWarmupMode::Warm),
                capture_method: Some("simpleperf".into()),
                warnings: vec!["missing symbols".into()],
            },
        }
    }
}
