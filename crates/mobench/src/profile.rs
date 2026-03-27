use anyhow::{bail, Result};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use crate::{resolve_devices_for_profile, DevicePlatform, MobileTarget, ResolvedMatrixDevice};
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

#[derive(Debug, Clone, Args)]
#[command(
    about = "Plan or execute a native profiling session depending on backend/provider support",
    after_help = concat!(
        "Capability matrix:\n",
        "  local + android-native: planned manifest today; native simpleperf capture is not implemented yet\n",
        "  local + ios-instruments: planned manifest today; Instruments trace export capture is not implemented yet\n",
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
    pub warmup_mode: Option<String>,
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
            let _ = writeln!(markdown, "- Warmup mode: `{warmup_mode}`");
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
    let target = resolve_profile_target(args)?;
    let run_id = build_run_id(args.target, &args.function);
    let run_output_dir = args.output_dir.join(&run_id);
    let mut manifest = build_capture_plan(args, &target, &run_output_dir)?;
    if dry_run {
        manifest.capture_metadata.warnings.push(
            "dry-run enabled; capture planning stopped before execution and recorded the planned artifact contract only"
                .into(),
        );
    } else {
        execute_capture(args, &target, &mut manifest)?;
    }

    std::fs::create_dir_all(&args.output_dir)?;
    std::fs::create_dir_all(&run_output_dir)?;
    create_selected_artifact_roots(
        &manifest.native_capture.raw_artifacts,
        &manifest.native_capture.processed_artifacts,
    )?;
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
    mut resolve: F,
) -> (String, SymbolizationRecord, String)
where
    F: FnMut(&Path, u64) -> Option<String>,
{
    let library_paths: HashMap<String, PathBuf> = native_libraries
        .iter()
        .map(|artifact| {
            (
                artifact.library_name.clone(),
                artifact.unstripped_path.clone(),
            )
        })
        .collect();

    symbolize_android_folded_stacks_with_resolver(folded_stacks, |library_name, offset| {
        let library_path = library_paths.get(library_name)?;
        resolve(library_path.as_path(), offset)
    })
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
                    label: "native-report".into(),
                    path: processed_root.join("native-report.txt"),
                },
                ArtifactRecord {
                    label: "flamegraph".into(),
                    path: processed_root.join("flamegraph.html"),
                },
            ],
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
        semantic_profile: SemanticProfileRecord::default(),
        capture_metadata: CaptureMetadataRecord {
            device: target
                .device
                .as_ref()
                .map(|device| device.identifier.clone()),
            sample_duration_secs: None,
            warmup_mode: None,
            capture_method: Some(match backend {
                ProfileBackend::AndroidNative => "simpleperf".into(),
                ProfileBackend::IosInstruments => "instruments".into(),
                ProfileBackend::RustTracing => "trace-events".into(),
                ProfileBackend::Auto => unreachable!("auto backend should resolve before planning"),
            }),
            warnings: Vec::new(),
        },
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
    let plan_only_warning = match (args.provider, target.backend) {
        (ProfileProvider::Local, ProfileBackend::AndroidNative) => Some(
            "local android-native capture is not implemented yet; this session records the planned simpleperf artifact contract only",
        ),
        (ProfileProvider::Local, ProfileBackend::IosInstruments) => Some(
            "local ios-instruments capture is not implemented yet; this session records the planned Instruments trace/XML artifact contract only",
        ),
        (ProfileProvider::Local, ProfileBackend::RustTracing) => Some(
            "local rust-tracing capture is not implemented yet; this session records the planned trace-events artifact contract only",
        ),
        (ProfileProvider::Browserstack, ProfileBackend::AndroidNative) => {
            bail!(browserstack_native_capture_unsupported_message(
                "android-native",
                "local Android profiling produces simpleperf artifacts and flamegraphs when implemented",
            ));
        }
        (ProfileProvider::Browserstack, ProfileBackend::IosInstruments) => {
            bail!(browserstack_native_capture_unsupported_message(
                "ios-instruments",
                "local iOS profiling produces Instruments traces (`time-profiler.trace`) and XML exports (`time-profiler.xml`), not flamegraphs",
            ));
        }
        (ProfileProvider::Browserstack, ProfileBackend::RustTracing) => {
            bail!(
                "BrowserStack rust-tracing capture is not implemented.\nThis command currently writes a local-first profile contract only.\nUse --provider local for trace-events output, or run a normal BrowserStack benchmark if you only need timing/memory metrics."
            );
        }
        (_, ProfileBackend::Auto) => unreachable!("auto backend should resolve before execution"),
    };

    if let Some(device) = &target.device {
        manifest.capture_metadata.warnings.push(format!(
            "resolved target device: {} ({}, source: {})",
            device.identifier, device.os, device.source
        ));
    }
    if let Some(warning) = plan_only_warning {
        manifest.capture_metadata.warnings.push(warning.into());
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
        }
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
        assert_eq!(manifest.semantic_profile.status, SemanticCaptureStatus::Planned);
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
    fn android_native_offsets_use_unstripped_library_paths() {
        let unstripped_path = PathBuf::from("/cargo/target/aarch64-linux-android/release/libsample_fns.so");
        let packaged_path = PathBuf::from("/apk/jniLibs/arm64-v8a/libsample_fns.so");
        let native_libraries = vec![NativeLibraryArtifact {
            abi: "arm64-v8a".into(),
            library_name: "libsample_fns.so".into(),
            unstripped_path: unstripped_path.clone(),
            packaged_path,
        }];
        let mut seen_paths = Vec::new();

        let (symbolized, record, report) = symbolize_android_folded_stacks_with_native_libraries(
            "dev.world.samplefns;libsample_fns.so[+94138] 1",
            &native_libraries,
            |path, offset| {
                seen_paths.push((path.to_path_buf(), offset));
                Some("sample_fns::fibonacci".into())
            },
        );

        assert!(symbolized.contains("sample_fns::fibonacci"));
        assert_eq!(seen_paths.len(), 1);
        assert_eq!(seen_paths[0].0, unstripped_path);
        assert_eq!(seen_paths[0].1, 94_138);
        assert_eq!(record.status, CaptureStatus::Captured);
        assert_eq!(record.resolved_frames, 1);
        assert!(report.contains("sample_fns::fibonacci"));
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

        assert!(plan
            .native_capture
            .raw_artifacts
            .iter()
            .any(|p| p.path.ends_with("sample.perf")));
        assert!(plan
            .native_capture
            .processed_artifacts
            .iter()
            .any(|p| p.path.ends_with("flamegraph.html")));
        assert!(plan
            .native_capture
            .processed_artifacts
            .iter()
            .any(|p| p.path.ends_with("stacks.folded")));
        assert!(plan
            .native_capture
            .processed_artifacts
            .iter()
            .any(|p| p.path.ends_with("native-report.txt")));
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
    fn ios_backend_allocates_trace_bundle_and_export_paths() {
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

        assert!(plan
            .native_capture
            .raw_artifacts
            .iter()
            .any(|p| p.path.ends_with("time-profiler.trace")));
        assert!(plan
            .native_capture
            .processed_artifacts
            .iter()
            .any(|p| p.path.ends_with("time-profiler.xml")));
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

        cmd_profile_run(&android_args, false).expect("write first planned profile session");
        cmd_profile_run(&ios_args, false).expect("write second planned profile session");

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
                processed_artifacts: vec![ArtifactRecord {
                    label: "flamegraph".into(),
                    path: PathBuf::from("artifacts/processed/flamegraph.html"),
                }],
                symbolization: SymbolizationRecord {
                    status: CaptureStatus::Partial,
                    tool: Some("llvm-addr2line".into()),
                    resolved_frames: 3,
                    unresolved_frames: 1,
                    notes: vec!["missing symbols".into()],
                },
                viewer_hint: Some("Open flamegraph.html in a browser".into()),
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
                warmup_mode: Some("warm".into()),
                capture_method: Some("simpleperf".into()),
                warnings: vec!["missing symbols".into()],
            },
        }
    }
}
