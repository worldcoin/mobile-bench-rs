use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::{
    MobileTarget,
    flamegraph_viewer::{
        ArtifactLink as ViewerArtifactLink, FlamegraphMode, FlamegraphViewerDoc, FrameSourceLink,
        ViewerHarnessTimelineSpan, ViewerMetadataItem, ViewerTraceEvent, ViewerTraceLane,
        count_folded_stack_lines, derive_benchmark_focused_folded_stacks,
        render_flamegraph_viewer_html, render_standalone_flamegraph_svg, summarize_folded_stacks,
    },
    repo_root,
};

mod android;
mod capture;
mod ios;
mod manifest;
mod output;
mod semantic;
mod session;
mod target;

#[cfg(test)]
pub(crate) use crate::benchmark_output::{
    ANDROID_BENCH_LOG_MARKER, extract_benchmark_reports_from_logs,
};
pub(super) use android::execute_local_android_capture;
#[cfg(test)]
pub(crate) use android::{
    android_log_contains_marker, locate_android_llvm_addr2line,
    symbolize_android_folded_stacks_with_native_libraries,
    symbolize_android_folded_stacks_with_resolver, write_android_symbolized_outputs_with_resolver,
};
#[cfg(test)]
pub(crate) use ios::collapse_ios_sample_call_graph_to_folded_stacks;
pub(super) use ios::execute_local_ios_capture;
pub(crate) use manifest::{
    ArtifactRecord, CaptureMetadataRecord, CaptureStatus, HarnessTimelineSpanRecord,
    NativeCaptureRecord, ProfileManifest, SemanticCaptureStatus, SemanticPhaseRecord,
    SemanticProfileRecord, SymbolizationRecord, load_profile_manifest, render_profile_markdown,
    write_profile_manifest,
};
#[cfg(test)]
pub(crate) use mobench_sdk::types::NativeLibraryArtifact;
pub(super) use target::{
    ResolvedProfileDevice, ResolvedProfileTarget, build_run_id, resolve_profile_device,
    resolve_profile_target, validate_format_capabilities,
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

#[derive(Debug, Clone, Args)]
pub struct ProfileDiffArgs {
    #[arg(long, help = "Path to the baseline profile.json manifest")]
    pub baseline: PathBuf,
    #[arg(long, help = "Path to the candidate profile.json manifest")]
    pub candidate: PathBuf,
    #[arg(
        long,
        default_value = "target/mobench/profile/diff",
        help = "Output directory for differential profile artifacts"
    )]
    pub output_dir: PathBuf,
    #[arg(
        long,
        help = "Normalize baseline sample counts to candidate totals before diffing"
    )]
    pub normalize: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChronologicalTraceSourceRecord {
    kind: String,
    profiler: String,
    origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChronologicalTraceRecord {
    source: ChronologicalTraceSourceRecord,
    total_duration_ns: u64,
    lanes: Vec<ViewerTraceLane>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FrameLocationRecord {
    frame: String,
    source_path: PathBuf,
    line: u32,
}

#[derive(Debug, Clone)]
struct TimelinePayload {
    lanes: Vec<ViewerTraceLane>,
    total_duration_ns: Option<u64>,
    note: Option<String>,
    trace_path: Option<PathBuf>,
}

pub fn cmd_profile_run(args: &ProfileRunArgs, dry_run: bool) -> Result<()> {
    session::run_profile_session_with_executor(args, dry_run, capture::execute)
}

fn prepare_viewer_timeline_payload(
    run_output_dir: &Path,
    processed_root: &Path,
    manifest: &ProfileManifest,
) -> Result<TimelinePayload> {
    let trace_path = artifact_path_by_label(
        &manifest.native_capture.processed_artifacts,
        "chronological-trace",
    )
    .map(|path| resolve_run_relative_path(run_output_dir, path))
    .unwrap_or_else(|| processed_root.join("chronological-trace.json"));
    if trace_path.exists()
        && let Ok(record) = load_chronological_trace_record(&trace_path)
    {
        let lanes = sanitize_trace_lanes(&record);
        return Ok(TimelinePayload {
            total_duration_ns: Some(record.total_duration_ns),
            note: build_timeline_note(&lanes),
            lanes,
            trace_path: Some(trace_path),
        });
    }

    let lanes = build_harness_only_viewer_timeline_lanes(manifest);
    let total_duration_ns = compute_timeline_total_duration_ns(
        &build_viewer_harness_timeline(manifest),
        manifest.capture_metadata.sample_duration_secs,
    );
    let trace_path = write_chronological_trace_sidecar(
        &trace_path,
        manifest,
        &lanes,
        total_duration_ns,
        "mobench-harness-timeline",
    )?;
    Ok(TimelinePayload {
        note: build_timeline_note(&lanes),
        lanes,
        total_duration_ns,
        trace_path,
    })
}

fn load_chronological_trace_record(path: &Path) -> Result<ChronologicalTraceRecord> {
    let body = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&body).with_context(|| format!("parsing {}", path.display()))
}

fn sanitize_trace_lanes(trace: &ChronologicalTraceRecord) -> Vec<ViewerTraceLane> {
    if trace.source.kind != "mobench-harness-timeline" {
        return trace.lanes.clone();
    }
    trace
        .lanes
        .iter()
        .map(|lane| ViewerTraceLane {
            id: lane.id.clone(),
            label: lane.label.clone(),
            events: lane
                .events
                .iter()
                .filter(|event| event.event_kind != "sample")
                .cloned()
                .collect(),
        })
        .filter(|lane| !lane.events.is_empty())
        .collect()
}

fn refresh_flamegraph_viewer_from_manifest(
    run_output_dir: &Path,
    manifest: &ProfileManifest,
) -> Result<()> {
    let Some(viewer_path) = artifact_path_by_label(
        &manifest.native_capture.processed_artifacts,
        "flamegraph-viewer",
    )
    .map(|path| resolve_run_relative_path(run_output_dir, path)) else {
        return Ok(());
    };
    let Some(processed_root) = viewer_path.parent() else {
        return Ok(());
    };

    let Some(full_svg_path) = artifact_path_by_label(
        &manifest.native_capture.processed_artifacts,
        "flamegraph-full-svg",
    )
    .map(|path| resolve_run_relative_path(run_output_dir, path)) else {
        return Ok(());
    };
    let Some(focused_svg_path) = artifact_path_by_label(
        &manifest.native_capture.processed_artifacts,
        "flamegraph-focused-svg",
    )
    .map(|path| resolve_run_relative_path(run_output_dir, path)) else {
        return Ok(());
    };
    let Some(full_folded_path) = artifact_path_by_label(
        &manifest.native_capture.processed_artifacts,
        "collapsed-stacks",
    )
    .map(|path| resolve_run_relative_path(run_output_dir, path)) else {
        return Ok(());
    };

    if !full_svg_path.exists() || !focused_svg_path.exists() || !full_folded_path.exists() {
        return Ok(());
    }

    let full_svg = std::fs::read_to_string(&full_svg_path)
        .with_context(|| format!("reading {}", full_svg_path.display()))?;
    let focused_svg = std::fs::read_to_string(&focused_svg_path)
        .with_context(|| format!("reading {}", focused_svg_path.display()))?;
    let full_folded = std::fs::read_to_string(&full_folded_path)
        .with_context(|| format!("reading {}", full_folded_path.display()))?;

    let focused = derive_benchmark_focused_folded_stacks(
        &full_folded,
        benchmark_anchors_for_backend(manifest.backend),
    );
    let focused_warning = if focused.folded.trim().is_empty() {
        Some(
            "No benchmark anchor frames were detected; the benchmark-only view is falling back to the full-process flamegraph."
                .to_string(),
        )
    } else {
        None
    };
    let focused_folded = if focused.folded.trim().is_empty() {
        full_folded.as_str()
    } else {
        focused.folded.as_str()
    };

    let harness_timeline = build_viewer_harness_timeline(manifest);
    let timeline_payload =
        prepare_viewer_timeline_payload(run_output_dir, processed_root, manifest)?;
    let source_links = load_viewer_source_links(run_output_dir, processed_root, manifest)?;
    let browser_title =
        flamegraph_browser_title(project_name_from_workspace_path(run_output_dir).as_deref());
    let viewer_html = render_flamegraph_viewer_html(FlamegraphViewerDoc {
        title: flamegraph_title_for_manifest(manifest),
        browser_title,
        full_svg_document: full_svg,
        focused_svg_document: focused_svg,
        full_summary: summarize_folded_stacks(
            &full_folded,
            count_folded_stack_lines(&full_folded),
            0,
            None,
        ),
        focused_summary: summarize_folded_stacks(
            focused_folded,
            focused.matched_stack_count,
            focused.excluded_stack_count,
            focused_warning,
        ),
        sampled_duration_secs: manifest
            .capture_metadata
            .sample_duration_secs
            .map(|value| value as f64),
        run_metadata: build_viewer_run_metadata(manifest),
        harness_timeline,
        timeline_lanes: timeline_payload.lanes.clone(),
        timeline_total_duration_ns: timeline_payload.total_duration_ns,
        timeline_note: timeline_payload.note,
        default_mode: FlamegraphMode::Focused,
        artifact_links: build_viewer_artifact_links(
            run_output_dir,
            processed_root,
            manifest,
            timeline_payload.trace_path.as_deref(),
        ),
        source_links: source_links.clone(),
        source_link_note: viewer_source_link_note(manifest, &source_links),
    });

    std::fs::write(&viewer_path, viewer_html)
        .with_context(|| format!("writing {}", viewer_path.display()))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DifferentialViewerManifest {
    run_id: String,
    baseline: String,
    candidate: String,
    #[serde(default)]
    target: Option<MobileTarget>,
    #[serde(default)]
    function: Option<String>,
    #[serde(default)]
    backend: Option<ProfileBackend>,
    #[serde(default)]
    normalize: bool,
    viewer_path: String,
    #[serde(default)]
    summary_path: Option<String>,
    warnings: Vec<String>,
    modes: Vec<DifferentialViewerModeRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DifferentialViewerModeRecord {
    mode: String,
    #[serde(default)]
    baseline_folded: Option<String>,
    #[serde(default)]
    candidate_folded: Option<String>,
    diff_folded: String,
    flamegraph_svg: String,
    #[serde(default)]
    baseline_samples: Option<u64>,
    #[serde(default)]
    candidate_samples: Option<u64>,
}

fn refresh_differential_flamegraph_viewer_from_manifest_path(
    diff_manifest_path: &Path,
) -> Result<()> {
    let diff_manifest_dir = diff_manifest_path
        .parent()
        .context("differential manifest path must have a parent directory")?;
    let diff_manifest: DifferentialViewerManifest = serde_json::from_slice(
        &std::fs::read(diff_manifest_path)
            .with_context(|| format!("reading {}", diff_manifest_path.display()))?,
    )
    .with_context(|| format!("parsing {}", diff_manifest_path.display()))?;

    let baseline_path = resolve_external_manifest_path(diff_manifest_dir, &diff_manifest.baseline);
    let candidate_path =
        resolve_external_manifest_path(diff_manifest_dir, &diff_manifest.candidate);
    let viewer_path = resolve_external_manifest_path(diff_manifest_dir, &diff_manifest.viewer_path);
    let processed_root = viewer_path
        .parent()
        .context("differential viewer path must have a parent directory")?;

    let baseline_manifest = load_profile_manifest(&baseline_path)?;
    let candidate_manifest = load_profile_manifest(&candidate_path)?;
    let candidate_run_dir = candidate_path
        .parent()
        .context("candidate manifest path must have a parent directory")?;

    let full_mode = differential_mode_record(&diff_manifest, "full")?;
    let focused_mode = differential_mode_record(&diff_manifest, "focused")?;

    let full_folded_path =
        resolve_external_manifest_path(diff_manifest_dir, &full_mode.diff_folded);
    let focused_folded_path =
        resolve_external_manifest_path(diff_manifest_dir, &focused_mode.diff_folded);
    let full_svg_path =
        resolve_external_manifest_path(diff_manifest_dir, &full_mode.flamegraph_svg);
    let focused_svg_path =
        resolve_external_manifest_path(diff_manifest_dir, &focused_mode.flamegraph_svg);

    let full_folded = std::fs::read_to_string(&full_folded_path)
        .with_context(|| format!("reading {}", full_folded_path.display()))?;
    let focused_folded = std::fs::read_to_string(&focused_folded_path)
        .with_context(|| format!("reading {}", focused_folded_path.display()))?;
    let full_svg = std::fs::read_to_string(&full_svg_path)
        .with_context(|| format!("reading {}", full_svg_path.display()))?;
    let focused_svg = std::fs::read_to_string(&focused_svg_path)
        .with_context(|| format!("reading {}", focused_svg_path.display()))?;

    let shared_warning = diff_manifest.warnings.first().cloned();
    let full_summary = summarize_folded_stacks(
        &full_folded,
        count_folded_stack_lines(&full_folded),
        0,
        shared_warning.clone(),
    );
    let focused_summary = summarize_folded_stacks(
        &focused_folded,
        count_folded_stack_lines(&focused_folded),
        0,
        shared_warning,
    );

    let harness_timeline = build_viewer_harness_timeline(&candidate_manifest);
    let timeline_payload =
        prepare_viewer_timeline_payload(candidate_run_dir, processed_root, &candidate_manifest)?;
    let source_links =
        load_viewer_source_links(candidate_run_dir, processed_root, &candidate_manifest)?;
    let browser_title =
        flamegraph_browser_title(project_name_from_workspace_path(&candidate_path).as_deref());

    let viewer_html = render_flamegraph_viewer_html(FlamegraphViewerDoc {
        title: "Differential Flamegraph".into(),
        browser_title,
        full_svg_document: full_svg,
        focused_svg_document: focused_svg,
        full_summary,
        focused_summary,
        sampled_duration_secs: candidate_manifest
            .capture_metadata
            .sample_duration_secs
            .map(|value| value as f64),
        run_metadata: build_differential_viewer_run_metadata(
            &diff_manifest.run_id,
            &baseline_manifest,
            &candidate_manifest,
        ),
        harness_timeline,
        timeline_lanes: timeline_payload.lanes.clone(),
        timeline_total_duration_ns: timeline_payload.total_duration_ns,
        timeline_note: timeline_payload.note,
        default_mode: FlamegraphMode::Focused,
        artifact_links: build_differential_viewer_artifact_links(
            processed_root,
            candidate_run_dir,
            &candidate_manifest,
            &full_folded_path,
            &focused_folded_path,
            &full_svg_path,
            &focused_svg_path,
            timeline_payload.trace_path.as_deref(),
        ),
        source_links: source_links.clone(),
        source_link_note: viewer_source_link_note(&candidate_manifest, &source_links),
    });

    std::fs::write(&viewer_path, viewer_html)
        .with_context(|| format!("writing {}", viewer_path.display()))?;
    Ok(())
}

fn differential_mode_record<'a>(
    manifest: &'a DifferentialViewerManifest,
    mode: &str,
) -> Result<&'a DifferentialViewerModeRecord> {
    manifest
        .modes
        .iter()
        .find(|record| record.mode == mode)
        .with_context(|| format!("missing `{mode}` mode in differential viewer manifest"))
}

fn resolve_external_manifest_path(base_dir: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        let manifest_relative = base_dir.join(path);
        if manifest_relative.exists() {
            return manifest_relative;
        }
        if let Some(workspace_root) = find_workspace_root(base_dir) {
            let workspace_relative = workspace_root.join(path);
            if workspace_relative.exists() || !manifest_relative.exists() {
                return workspace_relative;
            }
        }
        manifest_relative
    }
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        if ancestor.join("Cargo.toml").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn build_differential_viewer_run_metadata(
    diff_run_id: &str,
    baseline_manifest: &ProfileManifest,
    candidate_manifest: &ProfileManifest,
) -> Vec<ViewerMetadataItem> {
    let mut metadata = vec![
        ViewerMetadataItem {
            label: "Baseline Run".into(),
            value: baseline_manifest.run_id.clone(),
        },
        ViewerMetadataItem {
            label: "Candidate Run".into(),
            value: candidate_manifest.run_id.clone(),
        },
        ViewerMetadataItem {
            label: "Target".into(),
            value: match candidate_manifest.target {
                MobileTarget::Android => "android".into(),
                MobileTarget::Ios => "ios".into(),
            },
        },
        ViewerMetadataItem {
            label: "Backend".into(),
            value: match candidate_manifest.backend {
                ProfileBackend::AndroidNative => "android-native".into(),
                ProfileBackend::IosInstruments => "ios-instruments".into(),
                ProfileBackend::RustTracing => "rust-tracing".into(),
                ProfileBackend::Auto => "auto".into(),
            },
        },
        ViewerMetadataItem {
            label: "Benchmark".into(),
            value: candidate_manifest.function.clone(),
        },
    ];
    if let Some(device) = &candidate_manifest.capture_metadata.device {
        metadata.push(ViewerMetadataItem {
            label: "Device".into(),
            value: device.clone(),
        });
    }
    if let Some(os) = &candidate_manifest.capture_metadata.os {
        metadata.push(ViewerMetadataItem {
            label: "OS".into(),
            value: os.clone(),
        });
    }
    if candidate_manifest
        .capture_metadata
        .benchmark_iterations
        .is_some()
        || candidate_manifest
            .capture_metadata
            .benchmark_warmup
            .is_some()
    {
        let measured = candidate_manifest
            .capture_metadata
            .benchmark_iterations
            .unwrap_or(0);
        let warmup = candidate_manifest
            .capture_metadata
            .benchmark_warmup
            .unwrap_or(0);
        metadata.push(ViewerMetadataItem {
            label: "Iterations".into(),
            value: format!("{measured} measured / {warmup} warmup"),
        });
    }
    let mut capture_parts = Vec::new();
    if let Some(method) = &candidate_manifest.capture_metadata.capture_method {
        capture_parts.push(method.clone());
    }
    if let Some(mode) = candidate_manifest.capture_metadata.warmup_mode {
        capture_parts.push(mode.as_str().to_string());
    }
    if let Some(duration) = candidate_manifest.capture_metadata.sample_duration_secs {
        capture_parts.push(format!("{duration}s sample"));
    }
    if !capture_parts.is_empty() {
        metadata.push(ViewerMetadataItem {
            label: "Capture".into(),
            value: capture_parts.join(" · "),
        });
    }
    metadata.push(ViewerMetadataItem {
        label: "Run ID".into(),
        value: diff_run_id.to_string(),
    });
    metadata
}

#[allow(clippy::too_many_arguments)]
fn build_differential_viewer_artifact_links(
    processed_root: &Path,
    candidate_run_dir: &Path,
    candidate_manifest: &ProfileManifest,
    full_folded_path: &Path,
    focused_folded_path: &Path,
    full_svg_path: &Path,
    focused_svg_path: &Path,
    trace_path: Option<&Path>,
) -> Vec<ViewerArtifactLink> {
    let mut links = Vec::new();

    for label in [
        "sample",
        "simpleperf",
        "trace-events",
        "native-report",
        "frame-locations",
    ] {
        if let Some(path) =
            artifact_path_by_label(&candidate_manifest.native_capture.raw_artifacts, label)
                .or_else(|| {
                    artifact_path_by_label(
                        &candidate_manifest.native_capture.processed_artifacts,
                        label,
                    )
                })
                .map(|path| resolve_run_relative_path(candidate_run_dir, path))
                .filter(|path| path.exists())
        {
            links.push(ViewerArtifactLink::new(
                artifact_display_label(label),
                relative_path_from(processed_root, &path),
            ));
        }
    }

    links.push(ViewerArtifactLink::new(
        "Full folded stacks",
        relative_path_from(processed_root, full_folded_path),
    ));
    links.push(ViewerArtifactLink::new(
        "Benchmark-focused folded stacks",
        relative_path_from(processed_root, focused_folded_path),
    ));
    links.push(ViewerArtifactLink::new(
        "Full-process SVG",
        relative_path_from(processed_root, full_svg_path),
    ));
    links.push(ViewerArtifactLink::new(
        "Benchmark-only SVG",
        relative_path_from(processed_root, focused_svg_path),
    ));

    if let Some(path) = trace_path {
        links.push(ViewerArtifactLink::new(
            "Chronological trace",
            relative_path_from(processed_root, path),
        ));
    }

    if let Some(path) = candidate_manifest.semantic_profile.spans_path.as_deref() {
        links.push(ViewerArtifactLink::new(
            "Semantic phases",
            relative_path_from(
                processed_root,
                &resolve_run_relative_path(candidate_run_dir, path),
            ),
        ));
    }
    if let Some(path) = candidate_manifest.semantic_profile.timeline_path.as_deref() {
        links.push(ViewerArtifactLink::new(
            "Harness timeline",
            relative_path_from(
                processed_root,
                &resolve_run_relative_path(candidate_run_dir, path),
            ),
        ));
    }

    links
}

fn artifact_path_by_label<'a>(artifacts: &'a [ArtifactRecord], label: &str) -> Option<&'a Path> {
    artifacts
        .iter()
        .find(|artifact| artifact.label == label)
        .map(|artifact| artifact.path.as_path())
}

fn benchmark_anchors_for_backend(backend: ProfileBackend) -> &'static [&'static str] {
    match backend {
        ProfileBackend::AndroidNative => ANDROID_BENCHMARK_ANCHORS,
        ProfileBackend::IosInstruments => IOS_BENCHMARK_ANCHORS,
        ProfileBackend::RustTracing | ProfileBackend::Auto => &[],
    }
}

fn flamegraph_browser_title(project_name: Option<&str>) -> String {
    match project_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("Mobench Flamegraph - {name}"),
        None => "Mobench Flamegraph".into(),
    }
}

fn project_name_from_workspace_path(path: &Path) -> Option<String> {
    find_workspace_root(path)
        .and_then(|root| {
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .or_else(|| {
            repo_root().ok().and_then(|root| {
                root.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .filter(|name| !name.is_empty())
            })
        })
}

fn flamegraph_title_for_manifest(manifest: &ProfileManifest) -> String {
    match manifest.backend {
        ProfileBackend::AndroidNative => "Android Native Profile".into(),
        ProfileBackend::IosInstruments => "iOS Native Profile".into(),
        ProfileBackend::RustTracing => "Rust Tracing Profile".into(),
        ProfileBackend::Auto => "Native Profile".into(),
    }
}

fn build_viewer_run_metadata(manifest: &ProfileManifest) -> Vec<ViewerMetadataItem> {
    let mut metadata = vec![
        ViewerMetadataItem {
            label: "Run ID".into(),
            value: manifest.run_id.clone(),
        },
        ViewerMetadataItem {
            label: "Target".into(),
            value: match manifest.target {
                MobileTarget::Android => "android".into(),
                MobileTarget::Ios => "ios".into(),
            },
        },
        ViewerMetadataItem {
            label: "Backend".into(),
            value: match manifest.backend {
                ProfileBackend::AndroidNative => "android-native".into(),
                ProfileBackend::IosInstruments => "ios-instruments".into(),
                ProfileBackend::RustTracing => "rust-tracing".into(),
                ProfileBackend::Auto => "auto".into(),
            },
        },
        ViewerMetadataItem {
            label: "Benchmark".into(),
            value: manifest.function.clone(),
        },
    ];
    if let Some(device) = &manifest.capture_metadata.device {
        metadata.push(ViewerMetadataItem {
            label: "Device".into(),
            value: device.clone(),
        });
    }
    if let Some(os) = &manifest.capture_metadata.os {
        metadata.push(ViewerMetadataItem {
            label: "OS".into(),
            value: os.clone(),
        });
    }
    if manifest.capture_metadata.benchmark_iterations.is_some()
        || manifest.capture_metadata.benchmark_warmup.is_some()
    {
        let measured = manifest.capture_metadata.benchmark_iterations.unwrap_or(0);
        let warmup = manifest.capture_metadata.benchmark_warmup.unwrap_or(0);
        metadata.push(ViewerMetadataItem {
            label: "Iterations".into(),
            value: format!("{measured} measured / {warmup} warmup"),
        });
    }
    let mut capture_parts = Vec::new();
    if let Some(method) = &manifest.capture_metadata.capture_method {
        capture_parts.push(method.clone());
    }
    if let Some(mode) = manifest.capture_metadata.warmup_mode {
        capture_parts.push(mode.as_str().to_string());
    }
    if let Some(duration) = manifest.capture_metadata.sample_duration_secs {
        capture_parts.push(format!("{duration}s sample"));
    }
    if !capture_parts.is_empty() {
        metadata.push(ViewerMetadataItem {
            label: "Capture".into(),
            value: capture_parts.join(" · "),
        });
    }
    metadata
}

fn build_viewer_harness_timeline(manifest: &ProfileManifest) -> Vec<ViewerHarnessTimelineSpan> {
    manifest
        .semantic_profile
        .harness_timeline
        .iter()
        .map(|span| ViewerHarnessTimelineSpan {
            phase: span.phase.clone(),
            start_offset_ns: span.start_offset_ns,
            end_offset_ns: span.end_offset_ns,
            iteration: span.iteration,
        })
        .collect()
}

fn build_harness_only_viewer_timeline_lanes(manifest: &ProfileManifest) -> Vec<ViewerTraceLane> {
    let harness_events: Vec<ViewerTraceEvent> = manifest
        .semantic_profile
        .harness_timeline
        .iter()
        .map(|span| ViewerTraceEvent {
            event_kind: "span".into(),
            start_offset_ns: span.start_offset_ns,
            end_offset_ns: Some(span.end_offset_ns),
            frames: Vec::new(),
            phase: Some(span.phase.clone()),
            iteration: span.iteration,
        })
        .collect();

    let mut lanes = Vec::new();
    if !harness_events.is_empty() {
        lanes.push(ViewerTraceLane {
            id: "harness".into(),
            label: "Harness".into(),
            events: harness_events,
        });
    }
    lanes
}

fn compute_timeline_total_duration_ns(
    harness_timeline: &[ViewerHarnessTimelineSpan],
    sampled_duration_secs: Option<u64>,
) -> Option<u64> {
    harness_timeline
        .iter()
        .map(|span| span.end_offset_ns)
        .max()
        .or_else(|| sampled_duration_secs.map(|value| value.saturating_mul(1_000_000_000)))
}

fn trace_lanes_have_sample_events(lanes: &[ViewerTraceLane]) -> bool {
    lanes.iter().any(|lane| {
        lane.events
            .iter()
            .any(|event| event.event_kind == "sample" && !event.frames.is_empty())
    })
}

fn build_timeline_note(lanes: &[ViewerTraceLane]) -> Option<String> {
    if lanes.is_empty() {
        return Some(
            "Timeline mode becomes available once exact harness intervals or chronological trace events are recorded."
                .into(),
        );
    }
    if trace_lanes_have_sample_events(lanes) {
        Some(
            "Timeline mode shows exact harness chronology plus recorded stack samples. Aggregate flamegraph views remain full-session hotspot summaries."
                .into(),
        )
    } else {
        Some(
            "Harness-only timeline. This capture recorded exact phase timing, but it does not include time-ordered stack samples for the selected interval."
                .into(),
        )
    }
}

fn write_chronological_trace_sidecar(
    trace_path: &Path,
    manifest: &ProfileManifest,
    lanes: &[ViewerTraceLane],
    total_duration_ns: Option<u64>,
    source_kind: &str,
) -> Result<Option<PathBuf>> {
    let Some(total_duration_ns) = total_duration_ns else {
        return Ok(None);
    };
    if lanes.is_empty() {
        return Ok(None);
    }

    let trace = ChronologicalTraceRecord {
        source: ChronologicalTraceSourceRecord {
            kind: source_kind.into(),
            profiler: manifest
                .capture_metadata
                .capture_method
                .clone()
                .unwrap_or_else(|| match manifest.backend {
                    ProfileBackend::AndroidNative => "simpleperf".into(),
                    ProfileBackend::IosInstruments => "sample".into(),
                    ProfileBackend::RustTracing => "trace-events".into(),
                    ProfileBackend::Auto => "unknown".into(),
                }),
            origin: match manifest.provider {
                ProfileProvider::Local => "local".into(),
                ProfileProvider::Browserstack => "browserstack".into(),
            },
        },
        total_duration_ns,
        lanes: lanes.to_vec(),
    };
    if let Some(parent) = trace_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(trace_path, serde_json::to_vec_pretty(&trace)?)
        .with_context(|| format!("writing {}", trace_path.display()))?;
    Ok(Some(trace_path.to_path_buf()))
}

fn build_viewer_artifact_links(
    run_output_dir: &Path,
    processed_root: &Path,
    manifest: &ProfileManifest,
    trace_path: Option<&Path>,
) -> Vec<ViewerArtifactLink> {
    let mut links = Vec::new();
    let artifact_order = [
        "simpleperf",
        "sample",
        "trace-events",
        "native-report",
        "frame-locations",
        "collapsed-stacks",
        "benchmark-focused-stacks",
        "flamegraph-full-svg",
        "flamegraph-focused-svg",
    ];

    for label in artifact_order {
        if let Some(path) = artifact_path_by_label(&manifest.native_capture.raw_artifacts, label)
            .or_else(|| artifact_path_by_label(&manifest.native_capture.processed_artifacts, label))
            .map(|path| resolve_run_relative_path(run_output_dir, path))
            .filter(|path| path.exists())
        {
            links.push(ViewerArtifactLink::new(
                artifact_display_label(label),
                relative_path_from(processed_root, &path),
            ));
        }
    }

    if let Some(path) = trace_path {
        links.push(ViewerArtifactLink::new(
            "Chronological trace",
            relative_path_from(processed_root, path),
        ));
    }

    if let Some(path) = manifest.semantic_profile.spans_path.as_deref() {
        links.push(ViewerArtifactLink::new(
            "Semantic phases",
            relative_path_from(
                processed_root,
                &resolve_run_relative_path(run_output_dir, path),
            ),
        ));
    }
    if let Some(path) = manifest.semantic_profile.timeline_path.as_deref() {
        links.push(ViewerArtifactLink::new(
            "Harness timeline",
            relative_path_from(
                processed_root,
                &resolve_run_relative_path(run_output_dir, path),
            ),
        ));
    }

    links
}

fn artifact_display_label(label: &str) -> String {
    match label {
        "simpleperf" => "Raw sample.perf".into(),
        "sample" => "Raw sample.txt".into(),
        "trace-events" => "Raw trace-events.json".into(),
        "native-report" => "Native report".into(),
        "frame-locations" => "Frame locations".into(),
        "collapsed-stacks" => "Full folded stacks".into(),
        "benchmark-focused-stacks" => "Benchmark-focused folded stacks".into(),
        "flamegraph-full-svg" => "Full-process SVG".into(),
        "flamegraph-focused-svg" => "Benchmark-only SVG".into(),
        _ => label.to_string(),
    }
}

fn resolve_run_relative_path(run_output_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() || path.starts_with(run_output_dir) {
        path.to_path_buf()
    } else {
        run_output_dir.join(path)
    }
}

fn relative_path_from(base_dir: &Path, target: &Path) -> String {
    let base_components: Vec<_> = base_dir.components().collect();
    let target_components: Vec<_> = target.components().collect();
    let mut shared = 0;
    while shared < base_components.len()
        && shared < target_components.len()
        && base_components[shared] == target_components[shared]
    {
        shared += 1;
    }

    let mut relative = PathBuf::new();
    for _ in shared..base_components.len() {
        relative.push("..");
    }
    for component in &target_components[shared..] {
        relative.push(component.as_os_str());
    }

    if relative.as_os_str().is_empty() {
        ".".into()
    } else {
        relative.to_string_lossy().replace('\\', "/")
    }
}

fn default_source_link_note(manifest: &ProfileManifest) -> Option<String> {
    match manifest.backend {
        ProfileBackend::IosInstruments => Some(
            "Source links are unavailable for simulator-host `sample` sessions in this release."
                .into(),
        ),
        ProfileBackend::AndroidNative => Some(
            "Source links are unavailable because this capture did not record Android frame location metadata.".into(),
        ),
        ProfileBackend::RustTracing => Some(
            "Source links are unavailable for trace-events output in this release.".into(),
        ),
        ProfileBackend::Auto => None,
    }
}

fn viewer_source_link_note(
    manifest: &ProfileManifest,
    source_links: &[FrameSourceLink],
) -> Option<String> {
    if source_links.is_empty() {
        default_source_link_note(manifest)
    } else {
        None
    }
}

fn load_viewer_source_links(
    run_output_dir: &Path,
    processed_root: &Path,
    manifest: &ProfileManifest,
) -> Result<Vec<FrameSourceLink>> {
    let Some(sidecar_path) = artifact_path_by_label(
        &manifest.native_capture.processed_artifacts,
        "frame-locations",
    )
    .map(|path| resolve_run_relative_path(run_output_dir, path)) else {
        return Ok(Vec::new());
    };
    if !sidecar_path.exists() {
        return Ok(Vec::new());
    }
    let records: Vec<FrameLocationRecord> = serde_json::from_slice(
        &std::fs::read(&sidecar_path)
            .with_context(|| format!("reading {}", sidecar_path.display()))?,
    )
    .with_context(|| format!("parsing {}", sidecar_path.display()))?;
    let repo_root = repo_root().ok();
    Ok(records
        .into_iter()
        .filter_map(|record| {
            frame_location_record_to_source_link(processed_root, repo_root.as_deref(), record)
        })
        .collect())
}

fn frame_location_record_to_source_link(
    processed_root: &Path,
    repo_root: Option<&Path>,
    record: FrameLocationRecord,
) -> Option<FrameSourceLink> {
    let absolute_path = if record.source_path.is_absolute() {
        record.source_path.clone()
    } else if let Some(root) = repo_root {
        root.join(&record.source_path)
    } else {
        record.source_path.clone()
    };
    let display_path = if let Some(root) = repo_root {
        absolute_path
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| absolute_path.clone())
    } else {
        absolute_path.clone()
    };
    let href = format!(
        "{}#L{}",
        relative_path_from(processed_root, &absolute_path),
        record.line
    );
    Some(FrameSourceLink {
        frame: record.frame,
        location: format!("{}:{}", display_path.display(), record.line),
        href,
    })
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

pub fn cmd_profile_diff(args: &ProfileDiffArgs) -> Result<()> {
    let baseline_manifest = load_profile_manifest(&args.baseline)?;
    let candidate_manifest = load_profile_manifest(&args.candidate)?;
    validate_profile_diff_inputs(
        &args.baseline,
        &baseline_manifest,
        &args.candidate,
        &candidate_manifest,
    )?;

    let baseline_run_dir = args
        .baseline
        .parent()
        .context("baseline manifest path must have a parent directory")?;
    let candidate_run_dir = args
        .candidate
        .parent()
        .context("candidate manifest path must have a parent directory")?;
    let diff_run_id = format!(
        "{}--vs--{}",
        baseline_manifest.run_id, candidate_manifest.run_id
    );
    let diff_run_dir = args.output_dir.join(&diff_run_id);
    let processed_root = diff_run_dir.join("artifacts/processed");
    std::fs::create_dir_all(&processed_root)?;
    info!(
        output_dir = %diff_run_dir.display(),
        normalize = args.normalize,
        "building profile diff"
    );

    let full_mode = build_profile_diff_mode(
        baseline_run_dir,
        &baseline_manifest,
        candidate_run_dir,
        &candidate_manifest,
        "full",
        "collapsed-stacks",
        processed_root.join("diff.full.folded"),
        processed_root.join("flamegraph.full.svg"),
        args.normalize,
    )?;
    let focused_mode = build_profile_diff_mode(
        baseline_run_dir,
        &baseline_manifest,
        candidate_run_dir,
        &candidate_manifest,
        "focused",
        "benchmark-focused-stacks",
        processed_root.join("diff.focused.folded"),
        processed_root.join("flamegraph.focused.svg"),
        args.normalize,
    )?;

    let viewer_path = processed_root.join("flamegraph.html");
    let summary_path = diff_run_dir.join("summary.md");
    let diff_manifest = DifferentialViewerManifest {
        run_id: diff_run_id,
        baseline: path_string(&args.baseline),
        candidate: path_string(&args.candidate),
        target: Some(candidate_manifest.target),
        function: Some(candidate_manifest.function.clone()),
        backend: Some(candidate_manifest.backend),
        normalize: args.normalize,
        viewer_path: path_string(&viewer_path),
        summary_path: Some(path_string(&summary_path)),
        warnings: vec![
            "Differential flamegraph colors: red = hotter in candidate, blue = hotter in baseline. Frame widths follow candidate sample counts."
                .into(),
        ],
        modes: vec![full_mode, focused_mode],
    };

    let diff_manifest_path = diff_run_dir.join("profile-diff.json");
    write_differential_manifest(&diff_manifest_path, &diff_manifest)?;
    refresh_differential_flamegraph_viewer_from_manifest_path(&diff_manifest_path)?;

    let summary = render_profile_diff_markdown(&diff_manifest);
    std::fs::write(&summary_path, summary.as_bytes())
        .with_context(|| format!("writing {}", summary_path.display()))?;

    std::fs::create_dir_all(&args.output_dir)?;
    write_differential_manifest(&args.output_dir.join("profile-diff.json"), &diff_manifest)?;
    std::fs::write(args.output_dir.join("summary.md"), summary.as_bytes())?;

    println!(
        "Differential profile written to {}",
        diff_manifest_path.display()
    );
    println!("Differential summary written to {}", summary_path.display());
    println!("Differential viewer written to {}", viewer_path.display());
    Ok(())
}

fn validate_profile_diff_inputs(
    baseline_path: &Path,
    baseline_manifest: &ProfileManifest,
    candidate_path: &Path,
    candidate_manifest: &ProfileManifest,
) -> Result<()> {
    if baseline_manifest.target != candidate_manifest.target {
        bail!(
            "profile diff requires the same target on both sides, got `{}` from {} and `{}` from {}",
            baseline_manifest.target.as_str(),
            baseline_path.display(),
            candidate_manifest.target.as_str(),
            candidate_path.display()
        );
    }
    if baseline_manifest.backend != candidate_manifest.backend {
        bail!(
            "profile diff requires the same backend on both sides, got `{:?}` from {} and `{:?}` from {}",
            baseline_manifest.backend,
            baseline_path.display(),
            candidate_manifest.backend,
            candidate_path.display()
        );
    }
    if baseline_manifest.function != candidate_manifest.function {
        bail!(
            "profile diff requires the same benchmark function on both sides, got `{}` from {} and `{}` from {}",
            baseline_manifest.function,
            baseline_path.display(),
            candidate_manifest.function,
            candidate_path.display()
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_profile_diff_mode(
    baseline_run_dir: &Path,
    baseline_manifest: &ProfileManifest,
    candidate_run_dir: &Path,
    candidate_manifest: &ProfileManifest,
    mode: &str,
    artifact_label: &str,
    diff_folded_path: PathBuf,
    flamegraph_svg_path: PathBuf,
    normalize: bool,
) -> Result<DifferentialViewerModeRecord> {
    let baseline_folded_path =
        resolve_required_processed_artifact(baseline_run_dir, baseline_manifest, artifact_label)?;
    let candidate_folded_path =
        resolve_required_processed_artifact(candidate_run_dir, candidate_manifest, artifact_label)?;

    if let Some(parent) = diff_folded_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::new(
        File::create(&diff_folded_path)
            .with_context(|| format!("creating {}", diff_folded_path.display()))?,
    );
    inferno::differential::from_files(
        inferno::differential::Options {
            normalize,
            strip_hex: false,
        },
        &baseline_folded_path,
        &candidate_folded_path,
        &mut writer,
    )
    .with_context(|| format!("diffing folded stacks for `{mode}` mode"))?;

    let diff_folded = std::fs::read_to_string(&diff_folded_path)
        .with_context(|| format!("reading {}", diff_folded_path.display()))?;
    let svg = render_standalone_flamegraph_svg(&diff_folded, "Differential Flamegraph")?;
    std::fs::write(&flamegraph_svg_path, svg.as_bytes())
        .with_context(|| format!("writing {}", flamegraph_svg_path.display()))?;

    Ok(DifferentialViewerModeRecord {
        mode: mode.into(),
        baseline_folded: Some(path_string(&baseline_folded_path)),
        candidate_folded: Some(path_string(&candidate_folded_path)),
        diff_folded: path_string(&diff_folded_path),
        flamegraph_svg: path_string(&flamegraph_svg_path),
        baseline_samples: Some(total_samples_in_folded_path(&baseline_folded_path)?),
        candidate_samples: Some(total_samples_in_folded_path(&candidate_folded_path)?),
    })
}

fn resolve_required_processed_artifact(
    run_output_dir: &Path,
    manifest: &ProfileManifest,
    label: &str,
) -> Result<PathBuf> {
    artifact_path_by_label(&manifest.native_capture.processed_artifacts, label)
        .map(|path| resolve_run_relative_path(run_output_dir, path))
        .filter(|path| path.exists())
        .with_context(|| {
            format!(
                "profile manifest `{}` is missing processed artifact `{label}`",
                manifest.run_id
            )
        })
}

fn total_samples_in_folded_path(path: &Path) -> Result<u64> {
    let folded =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(folded
        .lines()
        .filter_map(split_folded_stack_line)
        .map(|(_, count)| count)
        .sum())
}

fn split_folded_stack_line(line: &str) -> Option<(&str, u64)> {
    let (stack, count) = line.rsplit_once(' ')?;
    if stack.is_empty() || !count.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((stack, count.parse().ok()?))
}

fn write_differential_manifest(path: &Path, manifest: &DifferentialViewerManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(manifest)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn render_profile_diff_markdown(manifest: &DifferentialViewerManifest) -> String {
    let mut markdown = String::new();
    let _ = writeln!(markdown, "# Differential Flamegraph Summary");
    let _ = writeln!(markdown);
    let _ = writeln!(markdown, "- Run ID: `{}`", manifest.run_id);
    if let Some(target) = manifest.target {
        let _ = writeln!(markdown, "- Target: `{}`", target.as_str());
    }
    if let Some(function) = &manifest.function {
        let _ = writeln!(markdown, "- Function: `{function}`");
    }
    let _ = writeln!(markdown, "- Baseline: `{}`", manifest.baseline);
    let _ = writeln!(markdown, "- Candidate: `{}`", manifest.candidate);
    let _ = writeln!(markdown, "- Normalize: `{}`", manifest.normalize);
    let _ = writeln!(markdown, "- Viewer: `{}`", manifest.viewer_path);
    let _ = writeln!(markdown);
    let _ = writeln!(
        markdown,
        "- Differential semantics: `red = hotter in candidate, blue = hotter in baseline, widths = candidate samples`"
    );
    if !manifest.warnings.is_empty() {
        let _ = writeln!(markdown);
        let _ = writeln!(markdown, "## Notes");
        let _ = writeln!(markdown);
        for warning in &manifest.warnings {
            let _ = writeln!(markdown, "- {}", warning);
        }
    }
    let _ = writeln!(markdown);
    let _ = writeln!(markdown, "## Modes");
    let _ = writeln!(markdown);
    for mode in &manifest.modes {
        let _ = writeln!(markdown, "### {}", mode.mode);
        if let Some(path) = &mode.baseline_folded {
            let _ = writeln!(markdown, "- Baseline folded: `{}`", path);
        }
        if let Some(path) = &mode.candidate_folded {
            let _ = writeln!(markdown, "- Candidate folded: `{}`", path);
        }
        let _ = writeln!(markdown, "- Diff folded: `{}`", mode.diff_folded);
        let _ = writeln!(markdown, "- SVG: `{}`", mode.flamegraph_svg);
        if let (Some(before), Some(after)) = (mode.baseline_samples, mode.candidate_samples) {
            let _ = writeln!(
                markdown,
                "- Samples: baseline `{}` -> candidate `{}`",
                before, after
            );
        }
        let _ = writeln!(markdown);
    }
    markdown
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
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
        browser_title: flamegraph_browser_title(
            project_name_from_workspace_path(processed_root).as_deref(),
        ),
        full_svg_document: full_svg,
        focused_svg_document: focused_svg,
        full_summary,
        focused_summary,
        sampled_duration_secs: None,
        run_metadata: Vec::new(),
        harness_timeline: Vec::new(),
        timeline_lanes: Vec::new(),
        timeline_total_duration_ns: None,
        timeline_note: None,
        default_mode: FlamegraphMode::Focused,
        artifact_links: vec![
            ViewerArtifactLink::new(raw_artifact_label, raw_artifact_path),
            ViewerArtifactLink::new("Native report", "native-report.txt"),
            ViewerArtifactLink::new("Full folded stacks", "stacks.folded"),
            ViewerArtifactLink::new(
                "Benchmark-focused folded stacks",
                "benchmark.focused.folded",
            ),
            ViewerArtifactLink::new("Full-process SVG", "flamegraph.full.svg"),
            ViewerArtifactLink::new("Benchmark-only SVG", "flamegraph.focused.svg"),
        ],
        source_links: Vec::new(),
        source_link_note: None,
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

pub fn cmd_profile_summarize_for_test(args: &ProfileSummarizeArgs) -> Result<String> {
    let manifest = load_profile_manifest(&args.profile)?;
    match args.output_format {
        ProfileSummaryFormat::Markdown => Ok(render_profile_markdown(&manifest)),
        ProfileSummaryFormat::Json => Ok(serde_json::to_string_pretty(&manifest)?),
    }
}

#[cfg(test)]
mod tests {
    use super::session::{build_capture_plan, run_profile_session_with_executor};
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
            crate::benchmark_output::benchmark_value_function(&reports[0]),
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

        semantic::populate_from_benchmark_value(
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

        semantic::merge_from_bench_report(
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
    fn semantic_processor_ingests_phase_timings_and_harness_timeline() {
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

        semantic::merge_from_bench_report(
            &mut manifest,
            &serde_json::json!({
                "spec": {
                    "iterations": 3,
                    "warmup": 1
                },
                "samples": [
                    {"duration_ns": 200},
                    {"duration_ns": 300}
                ],
                "phases": [
                    {"name": "prepare", "duration_ns": 100},
                    {"name": "execute", "duration_ns": 400}
                ],
                "timeline": [
                    {
                        "phase": "prepare",
                        "start_offset_ns": 0,
                        "end_offset_ns": 100,
                        "iteration": null
                    },
                    {
                        "phase": "execute",
                        "start_offset_ns": 100,
                        "end_offset_ns": 500,
                        "iteration": 2
                    }
                ]
            }),
        )
        .expect("merge semantic profile");

        assert_eq!(
            manifest.semantic_profile.status,
            SemanticCaptureStatus::Captured
        );
        assert_eq!(manifest.capture_metadata.benchmark_iterations, Some(3));
        assert_eq!(manifest.capture_metadata.benchmark_warmup, Some(1));
        assert_eq!(manifest.semantic_profile.phases[0].percent_total, Some(20));
        assert_eq!(manifest.semantic_profile.phases[1].percent_total, Some(80));
        assert_eq!(manifest.semantic_profile.harness_timeline.len(), 2);
        assert_eq!(
            manifest.semantic_profile.harness_timeline[1].iteration,
            Some(2)
        );
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

        output::persist_session_outputs(&args, &run_output_dir, &manifest)
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
    fn profile_output_persistence_writes_run_and_latest_outputs() {
        let dir = tempfile::tempdir().expect("temp dir");
        let manifest = sample_manifest();
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

        output::persist_session_outputs(&args, &run_output_dir, &manifest)
            .expect("persist profile outputs");

        assert!(run_output_dir.join("profile.json").exists());
        assert!(run_output_dir.join("summary.md").exists());
        assert!(dir.path().join("profile.json").exists());
        assert!(dir.path().join("summary.md").exists());
    }

    fn write_timeline_demo_session(
        output_dir: &Path,
        run_output_dir: &Path,
    ) -> Result<ProfileManifest> {
        let raw_root = run_output_dir.join("artifacts/raw");
        let processed_root = run_output_dir.join("artifacts/processed");
        let semantic_root = run_output_dir.join("artifacts/semantic");

        std::fs::create_dir_all(&raw_root)?;
        std::fs::create_dir_all(&processed_root)?;
        std::fs::create_dir_all(&semantic_root)?;

        let mut manifest = sample_manifest();
        manifest.run_id = "ios-demo".into();
        manifest.target = MobileTarget::Ios;
        manifest.backend = ProfileBackend::IosInstruments;
        manifest.native_capture.status = CaptureStatus::Captured;
        manifest.native_capture.symbolization.status = CaptureStatus::Captured;
        manifest.native_capture.symbolization.tool = Some("sample".into());
        manifest.capture_metadata.device = Some("iPhone 17 Pro-26.2".into());
        manifest.capture_metadata.os = Some("iOS 26.2".into());
        manifest.capture_metadata.capture_method = Some("sample/simctl".into());
        manifest.capture_metadata.sample_duration_secs = Some(15);
        manifest.capture_metadata.benchmark_iterations = Some(20);
        manifest.capture_metadata.benchmark_warmup = Some(3);
        manifest.capture_metadata.warmup_mode = Some(CaptureWarmupMode::Warm);
        manifest.semantic_profile.spans_path = Some(semantic_root.join("phases.json"));
        manifest.semantic_profile.timeline_path = Some(semantic_root.join("timeline.json"));
        manifest.semantic_profile.harness_timeline = vec![
            HarnessTimelineSpanRecord {
                phase: "setup".into(),
                start_offset_ns: 0,
                end_offset_ns: 500_000_000,
                iteration: None,
            },
            HarnessTimelineSpanRecord {
                phase: "warmup-benchmark".into(),
                start_offset_ns: 500_000_000,
                end_offset_ns: 1_000_000_000,
                iteration: Some(0),
            },
            HarnessTimelineSpanRecord {
                phase: "measured-benchmark".into(),
                start_offset_ns: 1_000_000_000,
                end_offset_ns: 1_400_000_000,
                iteration: Some(0),
            },
            HarnessTimelineSpanRecord {
                phase: "measured-benchmark".into(),
                start_offset_ns: 1_400_000_000,
                end_offset_ns: 1_800_000_000,
                iteration: Some(1),
            },
            HarnessTimelineSpanRecord {
                phase: "teardown".into(),
                start_offset_ns: 1_800_000_000,
                end_offset_ns: 2_100_000_000,
                iteration: None,
            },
        ];
        manifest.native_capture.raw_artifacts = vec![ArtifactRecord {
            label: "sample".into(),
            path: raw_root.join("sample.txt"),
        }];
        manifest.native_capture.processed_artifacts = vec![
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
            ArtifactRecord {
                label: "chronological-trace".into(),
                path: processed_root.join("chronological-trace.json"),
            },
        ];

        std::fs::write(raw_root.join("sample.txt"), "synthetic sample output")?;
        let folded = concat!(
            "UIKitMain;runBenchmark(spec:);sample_fns::run_benchmark;sample_fns::fibonacci 5\n",
            "UIKitMain;runBenchmark(spec:);sample_fns::run_benchmark;sample_fns::checksum 2\n",
        );
        write_dual_view_flamegraph_bundle(
            folded,
            &processed_root,
            "iOS Native Profile",
            IOS_BENCHMARK_ANCHORS,
            "../raw/sample.txt",
            "Raw sample.txt",
        )?;
        std::fs::write(
            processed_root.join("chronological-trace.json"),
            serde_json::to_vec_pretty(&ChronologicalTraceRecord {
                source: ChronologicalTraceSourceRecord {
                    kind: "mobench-demo-trace".into(),
                    profiler: "sample/simctl".into(),
                    origin: "local".into(),
                },
                total_duration_ns: 2_100_000_000,
                lanes: vec![ViewerTraceLane {
                    id: "main-thread".into(),
                    label: "Main Thread".into(),
                    events: vec![
                        ViewerTraceEvent {
                            event_kind: "sample".into(),
                            start_offset_ns: 1_050_000_000,
                            end_offset_ns: Some(1_180_000_000),
                            frames: vec![
                                "sample_fns::run_benchmark".into(),
                                "sample_fns::fibonacci".into(),
                            ],
                            phase: Some("measured-benchmark".into()),
                            iteration: Some(0),
                        },
                        ViewerTraceEvent {
                            event_kind: "sample".into(),
                            start_offset_ns: 1_430_000_000,
                            end_offset_ns: Some(1_560_000_000),
                            frames: vec![
                                "sample_fns::run_benchmark".into(),
                                "sample_fns::checksum".into(),
                            ],
                            phase: Some("measured-benchmark".into()),
                            iteration: Some(1),
                        },
                    ],
                }],
            })
            .expect("serialize demo trace"),
        )?;

        let args = ProfileRunArgs {
            target: MobileTarget::Ios,
            function: "sample_fns::fibonacci".into(),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::IosInstruments,
            format: ProfileFormat::Both,
            output_dir: output_dir.to_path_buf(),
            crate_path: None,
            device: None,
            os_version: None,
            profile: None,
            device_matrix: None,
            config: None,
            warmup_mode: Some(CaptureWarmupMode::Warm),
        };
        output::persist_session_outputs(&args, run_output_dir, &manifest)?;
        Ok(manifest)
    }

    #[test]
    fn write_profile_session_outputs_rewrites_flamegraph_with_timeline_payload() {
        let dir = tempfile::tempdir().expect("temp dir");
        let run_output_dir = dir.path().join("ios-demo");

        let manifest =
            write_timeline_demo_session(dir.path(), &run_output_dir).expect("write demo session");

        let viewer_html =
            std::fs::read_to_string(run_output_dir.join("artifacts/processed/flamegraph.html"))
                .expect("read flamegraph viewer");
        let trace_json = std::fs::read_to_string(
            run_output_dir.join("artifacts/processed/chronological-trace.json"),
        )
        .expect("read chronological trace");

        assert!(viewer_html.contains("Timeline"));
        assert!(viewer_html.contains("iPhone 17 Pro-26.2"));
        assert!(viewer_html.contains("20 measured / 3 warmup"));
        assert!(viewer_html.contains("sample/simctl"));
        assert!(viewer_html.contains("\"Main Thread\""));
        assert!(viewer_html.contains("\"warmup-benchmark\""));
        assert!(trace_json.contains("\"mobench-demo-trace\""));
        assert!(trace_json.contains("\"Main Thread\""));
        assert!(trace_json.contains(&manifest.function));
    }

    #[test]
    #[ignore]
    fn generate_flamegraph_timeline_demo_artifact() {
        let output_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/mobench/flamegraph-timeline-demo");
        let _ = std::fs::remove_dir_all(&output_dir);
        let run_output_dir = output_dir.join("ios-demo");

        write_timeline_demo_session(&output_dir, &run_output_dir).expect("generate demo artifact");
    }

    #[test]
    fn refresh_differential_viewer_manifest_writes_timeline_capable_html() {
        let dir = tempfile::tempdir().expect("temp dir");
        let baseline_run_dir = dir.path().join("baseline-run");
        let candidate_run_dir = dir.path().join("candidate-run");
        let diff_run_dir = dir.path().join("diff-run");
        let diff_processed_dir = diff_run_dir.join("artifacts/processed");
        std::fs::create_dir_all(&baseline_run_dir).expect("create baseline run dir");
        std::fs::create_dir_all(&candidate_run_dir).expect("create candidate run dir");
        std::fs::create_dir_all(&diff_processed_dir).expect("create diff processed dir");

        let mut baseline_manifest = sample_manifest();
        baseline_manifest.run_id = "baseline-run".into();
        baseline_manifest.target = MobileTarget::Ios;
        baseline_manifest.backend = ProfileBackend::IosInstruments;
        baseline_manifest.capture_metadata.device = Some("iPhone 17 Pro-26.2".into());
        baseline_manifest.capture_metadata.os = Some("iOS 26.2".into());
        baseline_manifest.capture_metadata.capture_method = Some("sample/simctl".into());

        let mut candidate_manifest = baseline_manifest.clone();
        candidate_manifest.run_id = "candidate-run".into();

        std::fs::write(
            baseline_run_dir.join("profile.json"),
            serde_json::to_vec_pretty(&baseline_manifest).expect("serialize baseline manifest"),
        )
        .expect("write baseline manifest");
        std::fs::write(
            candidate_run_dir.join("profile.json"),
            serde_json::to_vec_pretty(&candidate_manifest).expect("serialize candidate manifest"),
        )
        .expect("write candidate manifest");

        std::fs::write(
            diff_processed_dir.join("diff.full.folded"),
            "root;main 12\n",
        )
        .expect("write diff full folded");
        std::fs::write(
            diff_processed_dir.join("diff.focused.folded"),
            "bench;sample_fns::fibonacci 7\n",
        )
        .expect("write diff focused folded");
        std::fs::write(
            diff_processed_dir.join("flamegraph.full.svg"),
            "<svg id=\"full\"></svg>",
        )
        .expect("write diff full svg");
        std::fs::write(
            diff_processed_dir.join("flamegraph.focused.svg"),
            "<svg id=\"focused\"></svg>",
        )
        .expect("write diff focused svg");

        let diff_manifest_path = dir.path().join("profile-diff.json");
        std::fs::write(
            &diff_manifest_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "run_id": "baseline-run--vs--candidate-run",
                "baseline": "baseline-run/profile.json",
                "candidate": "candidate-run/profile.json",
                "viewer_path": "diff-run/artifacts/processed/flamegraph.html",
                "warnings": [
                    "Differential flamegraph colors: red = hotter in candidate, blue = hotter in baseline. Frame widths follow candidate sample counts."
                ],
                "modes": [
                    {
                        "mode": "full",
                        "diff_folded": "diff-run/artifacts/processed/diff.full.folded",
                        "flamegraph_svg": "diff-run/artifacts/processed/flamegraph.full.svg"
                    },
                    {
                        "mode": "focused",
                        "diff_folded": "diff-run/artifacts/processed/diff.focused.folded",
                        "flamegraph_svg": "diff-run/artifacts/processed/flamegraph.focused.svg"
                    }
                ]
            }))
            .expect("serialize diff manifest"),
        )
        .expect("write diff manifest");

        refresh_differential_flamegraph_viewer_from_manifest_path(&diff_manifest_path)
            .expect("refresh differential viewer");

        let html = std::fs::read_to_string(diff_processed_dir.join("flamegraph.html"))
            .expect("read differential flamegraph html");
        assert!(html.contains("data-mode=\"timeline\""));
        assert!(html.contains("Baseline Run"));
        assert!(html.contains("Candidate Run"));
        assert!(html.contains("Chronological trace"));
        assert!(html.contains("Exact harness time"));
    }

    #[test]
    fn cmd_profile_diff_writes_runtime_bundle() {
        let dir = tempfile::tempdir().expect("temp dir");
        let baseline_run_dir = dir.path().join("baseline-run");
        let candidate_run_dir = dir.path().join("candidate-run");
        std::fs::create_dir_all(baseline_run_dir.join("artifacts/processed"))
            .expect("create baseline processed");
        std::fs::create_dir_all(candidate_run_dir.join("artifacts/processed"))
            .expect("create candidate processed");

        let mut baseline_manifest = sample_manifest();
        baseline_manifest.run_id = "baseline-run".into();
        baseline_manifest.native_capture.status = CaptureStatus::Captured;
        baseline_manifest.native_capture.symbolization.status = CaptureStatus::Captured;
        let mut candidate_manifest = baseline_manifest.clone();
        candidate_manifest.run_id = "candidate-run".into();

        std::fs::write(
            baseline_run_dir.join("artifacts/processed/stacks.folded"),
            "root;sample_fns::fibonacci 4\n",
        )
        .expect("write baseline full");
        std::fs::write(
            baseline_run_dir.join("artifacts/processed/benchmark.focused.folded"),
            "sample_fns::run_benchmark;sample_fns::fibonacci 4\n",
        )
        .expect("write baseline focused");
        std::fs::write(
            candidate_run_dir.join("artifacts/processed/stacks.folded"),
            "root;sample_fns::fibonacci 7\nroot;sample_fns::checksum 1\n",
        )
        .expect("write candidate full");
        std::fs::write(
            candidate_run_dir.join("artifacts/processed/benchmark.focused.folded"),
            "sample_fns::run_benchmark;sample_fns::fibonacci 7\n",
        )
        .expect("write candidate focused");

        write_profile_manifest(&baseline_run_dir.join("profile.json"), &baseline_manifest)
            .expect("write baseline manifest");
        write_profile_manifest(&candidate_run_dir.join("profile.json"), &candidate_manifest)
            .expect("write candidate manifest");

        let output_dir = dir.path().join("diff");
        cmd_profile_diff(&ProfileDiffArgs {
            baseline: baseline_run_dir.join("profile.json"),
            candidate: candidate_run_dir.join("profile.json"),
            output_dir: output_dir.clone(),
            normalize: true,
        })
        .expect("run profile diff");

        let diff_dir = output_dir.join("baseline-run--vs--candidate-run");
        assert!(diff_dir.join("profile-diff.json").exists());
        assert!(diff_dir.join("summary.md").exists());
        assert!(
            diff_dir
                .join("artifacts/processed/flamegraph.html")
                .exists()
        );
        let summary = std::fs::read_to_string(diff_dir.join("summary.md")).expect("read summary");
        assert!(summary.contains("Differential Flamegraph Summary"));
        assert!(summary.contains("Normalize: `true`"));
    }

    #[test]
    fn refresh_flamegraph_viewer_includes_android_source_links_when_sidecar_exists() {
        let dir = tempfile::tempdir().expect("temp dir");
        let run_output_dir = dir.path().join("android-source-demo");
        let processed_root = run_output_dir.join("artifacts/processed");
        std::fs::create_dir_all(&processed_root).expect("create processed root");

        let mut manifest = sample_manifest();
        manifest.run_id = "android-source-demo".into();
        manifest.native_capture.status = CaptureStatus::Captured;
        manifest.native_capture.symbolization.status = CaptureStatus::Captured;
        manifest
            .native_capture
            .processed_artifacts
            .push(ArtifactRecord {
                label: "frame-locations".into(),
                path: PathBuf::from("artifacts/processed/frame-locations.json"),
            });

        std::fs::write(
            processed_root.join("stacks.folded"),
            "root;sample_fns::fibonacci 5\n",
        )
        .expect("write full folded");
        std::fs::write(
            processed_root.join("benchmark.focused.folded"),
            "sample_fns::run_benchmark;sample_fns::fibonacci 5\n",
        )
        .expect("write focused folded");
        std::fs::write(
            processed_root.join("flamegraph.full.svg"),
            "<svg id=\"full\"></svg>",
        )
        .expect("write full svg");
        std::fs::write(
            processed_root.join("flamegraph.focused.svg"),
            "<svg id=\"focused\"></svg>",
        )
        .expect("write focused svg");
        std::fs::write(
            processed_root.join("frame-locations.json"),
            serde_json::to_vec_pretty(&vec![FrameLocationRecord {
                frame: "sample_fns::fibonacci".into(),
                source_path: PathBuf::from("crates/sample-fns/src/lib.rs"),
                line: 42,
            }])
            .expect("serialize frame locations"),
        )
        .expect("write frame locations");

        refresh_flamegraph_viewer_from_manifest(&run_output_dir, &manifest)
            .expect("refresh flamegraph viewer");

        let html = std::fs::read_to_string(processed_root.join("flamegraph.html"))
            .expect("read viewer html");
        assert!(html.contains("sample_fns::fibonacci"));
        assert!(html.contains("crates/sample-fns/src/lib.rs:42"));
    }

    #[test]
    #[ignore]
    fn refresh_profile_diff_demo_viewer_artifact() {
        let diff_manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target/mobench/profile-diff-demo/profile-diff.json");
        refresh_differential_flamegraph_viewer_from_manifest_path(&diff_manifest_path)
            .expect("refresh profile diff demo viewer");
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

        semantic::populate_from_benchmark_value(&mut manifest, &bench_report);

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
    fn semantic_profile_ingests_exact_harness_timeline_and_run_counts_from_bench_report_json() {
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
            ],
            "timeline": [
                {
                    "phase": "setup",
                    "start_offset_ns": 0,
                    "end_offset_ns": 10,
                    "iteration": null
                },
                {
                    "phase": "measured-benchmark",
                    "start_offset_ns": 10,
                    "end_offset_ns": 30,
                    "iteration": 0
                }
            ]
        });

        semantic::populate_from_benchmark_value(&mut manifest, &bench_report);

        let json = serde_json::to_value(&manifest).expect("serialize manifest");
        assert_eq!(
            json["capture_metadata"]["benchmark_iterations"],
            serde_json::json!(2)
        );
        assert_eq!(
            json["capture_metadata"]["benchmark_warmup"],
            serde_json::json!(1)
        );
        assert_eq!(
            json["semantic_profile"]["harness_timeline"][0]["phase"],
            "setup"
        );
        assert_eq!(
            json["semantic_profile"]["harness_timeline"][1]["phase"],
            "measured-benchmark"
        );
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

        let error = capture::execute_with_local_android_executor(
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
    fn local_ios_attempted_capture_marks_failed_state() {
        let args = sample_run_args(
            MobileTarget::Ios,
            ProfileProvider::Local,
            ProfileBackend::IosInstruments,
            ProfileFormat::Both,
        );
        let target = resolve_profile_target(&args).expect("resolve target");
        let mut manifest =
            build_capture_plan(&args, &target, &PathBuf::from("target/mobench/profile"))
                .expect("build capture plan");

        let error =
            capture::execute_with_local_ios_executor(&args, &mut manifest, |_args, _manifest| {
                anyhow::bail!("simulated ios capture failure")
            })
            .expect_err("simulated capture failure");

        assert!(error.to_string().contains("simulated ios capture failure"));
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
                .any(|warning| warning.contains("simulated ios capture failure"))
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
            capture::execute_with_local_android_executor(args, manifest, |_args, _manifest| {
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
        let error = capture::execute(&args, &target, &mut manifest).unwrap_err();

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
        let error = capture::execute(&args, &target, &mut manifest).unwrap_err();

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
    fn profile_session_plan_owns_run_output_dir_and_manifest() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut args = sample_run_args(
            MobileTarget::Android,
            ProfileProvider::Local,
            ProfileBackend::AndroidNative,
            ProfileFormat::Both,
        );
        args.output_dir = dir.path().to_path_buf();
        let target = resolve_profile_target(&args).expect("resolve target");

        let session = session::ProfileSession::plan(&args, &target).expect("plan profile session");

        assert_eq!(
            session.run_output_dir(),
            dir.path().join("android-sample_fns--fibonacci").as_path()
        );
        assert_eq!(session.manifest().run_id, "android-sample_fns--fibonacci");
        assert_eq!(session.manifest().backend, ProfileBackend::AndroidNative);
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
                harness_timeline: vec![
                    HarnessTimelineSpanRecord {
                        phase: "setup".into(),
                        start_offset_ns: 0,
                        end_offset_ns: 100,
                        iteration: None,
                    },
                    HarnessTimelineSpanRecord {
                        phase: "measured-benchmark".into(),
                        start_offset_ns: 100,
                        end_offset_ns: 300,
                        iteration: Some(0),
                    },
                ],
                timeline_path: Some(PathBuf::from("artifacts/semantic/timeline.json")),
            },
            capture_metadata: CaptureMetadataRecord {
                device: target
                    .device
                    .as_ref()
                    .map(|device| device.identifier.clone()),
                os: Some("android 13".into()),
                sample_duration_secs: Some(15),
                benchmark_iterations: Some(20),
                benchmark_warmup: Some(3),
                warmup_mode: Some(CaptureWarmupMode::Warm),
                capture_method: Some("simpleperf".into()),
                warnings: vec!["missing symbols".into()],
            },
        }
    }
}
