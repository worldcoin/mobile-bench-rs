use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::path::{Path, PathBuf};

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
    pub backend: ProfileBackend,
    pub format: ProfileFormat,
    pub capture_status: CaptureStatus,
    pub raw_artifacts: Vec<ArtifactRecord>,
    pub processed_artifacts: Vec<ArtifactRecord>,
    pub warnings: Vec<String>,
    pub viewer_hint: Option<String>,
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

pub fn cmd_profile_run(args: &ProfileRunArgs) -> Result<()> {
    std::fs::create_dir_all(&args.output_dir)?;
    let manifest = build_capture_plan(args, &args.output_dir)?;
    let profile_path = args.output_dir.join("profile.json");
    let summary_path = args.output_dir.join("summary.md");
    write_profile_manifest(&profile_path, &manifest)?;
    std::fs::write(&summary_path, render_profile_markdown(&manifest).as_bytes())?;
    println!("Profile session written to {}", profile_path.display());
    println!("Profile summary written to {}", summary_path.display());
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

fn build_capture_plan(args: &ProfileRunArgs, output_root: &Path) -> Result<ProfileManifest> {
    let backend = resolve_backend(args.target, args.backend);
    validate_profile_capabilities(args.provider, backend)?;

    let raw_root = output_root.join("artifacts/raw");
    let processed_root = output_root.join("artifacts/processed");

    let (raw_artifacts, processed_artifacts, viewer_hint) = match backend {
        ProfileBackend::AndroidNative => (
            vec![ArtifactRecord {
                label: "simpleperf".into(),
                path: raw_root.join("sample.perf"),
            }],
            vec![ArtifactRecord {
                label: "flamegraph".into(),
                path: processed_root.join("flamegraph.html"),
            }],
            Some("Open artifacts/processed/flamegraph.html in a browser".into()),
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
            Some("Open artifacts/raw/time-profiler.trace in Instruments".into()),
        ),
        ProfileBackend::RustTracing => (
            vec![ArtifactRecord {
                label: "trace-events".into(),
                path: raw_root.join("trace-events.json"),
            }],
            Vec::new(),
            Some("Open artifacts/raw/trace-events.json in a trace viewer".into()),
        ),
        ProfileBackend::Auto => unreachable!("auto backend should resolve before planning"),
    };

    Ok(ProfileManifest {
        run_id: format!(
            "{}-{}",
            args.target.as_str(),
            slugify_function_name(&args.function)
        ),
        target: args.target,
        function: args.function.clone(),
        backend,
        format: args.format,
        capture_status: CaptureStatus::Planned,
        raw_artifacts,
        processed_artifacts,
        warnings: vec![
            "capture execution is not implemented yet; this session records the planned artifact contract only"
                .into(),
        ],
        viewer_hint,
    })
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
        )
        .unwrap_err();

        assert!(error.to_string().contains("BrowserStack"));
        assert!(error.to_string().contains("unsupported"));
    }

    #[test]
    fn profile_run_writes_manifest_and_summary_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let args = ProfileRunArgs {
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
            crate_path: None,
            config: None,
            output_dir: dir.path().to_path_buf(),
            provider: ProfileProvider::Local,
            backend: ProfileBackend::AndroidNative,
            format: ProfileFormat::Both,
        };

        cmd_profile_run(&args).expect("write planned profile session");

        assert!(dir.path().join("profile.json").exists());
        assert!(dir.path().join("summary.md").exists());
    }

    fn sample_manifest() -> ProfileManifest {
        ProfileManifest {
            run_id: "run-123".into(),
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
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
