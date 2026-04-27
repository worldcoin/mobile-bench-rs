use anyhow::Result;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::path::{Path, PathBuf};

use crate::MobileTarget;

use super::{CaptureWarmupMode, ProfileBackend, ProfileFormat, ProfileProvider};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CaptureStatus {
    Planned,
    Captured,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SemanticCaptureStatus {
    Planned,
    Captured,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ArtifactRecord {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SymbolizationRecord {
    pub(crate) status: CaptureStatus,
    pub(crate) tool: Option<String>,
    pub(crate) resolved_frames: u64,
    pub(crate) unresolved_frames: u64,
    pub(crate) notes: Vec<String>,
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
pub(crate) struct NativeCaptureRecord {
    pub(crate) status: CaptureStatus,
    pub(crate) raw_artifacts: Vec<ArtifactRecord>,
    pub(crate) processed_artifacts: Vec<ArtifactRecord>,
    pub(crate) symbolization: SymbolizationRecord,
    pub(crate) viewer_hint: Option<String>,
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
pub(crate) struct SemanticPhaseRecord {
    pub(crate) name: String,
    pub(crate) duration_ns: Option<u64>,
    pub(crate) percent_total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HarnessTimelineSpanRecord {
    pub(crate) phase: String,
    pub(crate) start_offset_ns: u64,
    pub(crate) end_offset_ns: u64,
    pub(crate) iteration: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SemanticProfileRecord {
    pub(crate) status: SemanticCaptureStatus,
    pub(crate) phases: Vec<SemanticPhaseRecord>,
    pub(crate) spans_path: Option<PathBuf>,
    #[serde(default)]
    pub(crate) harness_timeline: Vec<HarnessTimelineSpanRecord>,
    pub(crate) timeline_path: Option<PathBuf>,
}

impl Default for SemanticProfileRecord {
    fn default() -> Self {
        Self {
            status: SemanticCaptureStatus::Planned,
            phases: Vec::new(),
            spans_path: None,
            harness_timeline: Vec::new(),
            timeline_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct CaptureMetadataRecord {
    pub(crate) device: Option<String>,
    pub(crate) os: Option<String>,
    pub(crate) sample_duration_secs: Option<u64>,
    pub(crate) benchmark_iterations: Option<u32>,
    pub(crate) benchmark_warmup: Option<u32>,
    pub(crate) warmup_mode: Option<CaptureWarmupMode>,
    pub(crate) capture_method: Option<String>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProfileManifest {
    pub(crate) run_id: String,
    pub(crate) target: MobileTarget,
    pub(crate) function: String,
    #[serde(default = "default_profile_provider")]
    pub(crate) provider: ProfileProvider,
    pub(crate) backend: ProfileBackend,
    pub(crate) format: ProfileFormat,
    pub(crate) native_capture: NativeCaptureRecord,
    pub(crate) semantic_profile: SemanticProfileRecord,
    pub(crate) capture_metadata: CaptureMetadataRecord,
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

pub(crate) fn render_profile_markdown(manifest: &ProfileManifest) -> String {
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

pub(crate) fn write_profile_manifest(path: &Path, manifest: &ProfileManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(manifest)?;
    std::fs::write(path, body)?;
    Ok(())
}

pub(crate) fn load_profile_manifest(path: &Path) -> Result<ProfileManifest> {
    let body = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&body)?)
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
