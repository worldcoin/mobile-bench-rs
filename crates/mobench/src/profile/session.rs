use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::info;

use super::{
    ArtifactRecord, CaptureMetadataRecord, CaptureStatus, CaptureWarmupMode, NativeCaptureRecord,
    ProfileBackend, ProfileFormat, ProfileManifest, ProfileProvider, ProfileRunArgs,
    ResolvedProfileTarget, SemanticCaptureStatus, SemanticProfileRecord, SymbolizationRecord,
    build_run_id, resolve_profile_target, validate_format_capabilities,
    write_profile_session_outputs,
};

pub(super) struct ProfileSession {
    run_output_dir: PathBuf,
    manifest: ProfileManifest,
}

impl ProfileSession {
    pub(super) fn plan(args: &ProfileRunArgs, target: &ResolvedProfileTarget) -> Result<Self> {
        let run_id = build_run_id(args.target, &args.function);
        let run_output_dir = args.output_dir.join(&run_id);
        let manifest = build_capture_plan(args, target, &run_output_dir)?;

        Ok(Self {
            run_output_dir,
            manifest,
        })
    }

    pub(super) fn run_output_dir(&self) -> &Path {
        &self.run_output_dir
    }

    pub(super) fn profile_path(&self) -> PathBuf {
        self.run_output_dir.join("profile.json")
    }

    pub(super) fn summary_path(&self) -> PathBuf {
        self.run_output_dir.join("summary.md")
    }

    #[cfg(test)]
    pub(super) fn manifest(&self) -> &ProfileManifest {
        &self.manifest
    }

    pub(super) fn manifest_mut(&mut self) -> &mut ProfileManifest {
        &mut self.manifest
    }

    pub(super) fn should_persist_outputs(&self, dry_run: bool, execution_succeeded: bool) -> bool {
        dry_run
            || execution_succeeded
            || self.manifest.native_capture.status != CaptureStatus::Planned
            || self.manifest.native_capture.symbolization.status != CaptureStatus::Planned
            || self.manifest.semantic_profile.status != SemanticCaptureStatus::Planned
    }

    pub(super) fn persist(&self, args: &ProfileRunArgs) -> Result<()> {
        write_profile_session_outputs(args, &self.run_output_dir, &self.manifest)
    }
}

pub(super) fn run_profile_session_with_executor<E>(
    args: &ProfileRunArgs,
    dry_run: bool,
    execute: E,
) -> Result<()>
where
    E: FnOnce(&ProfileRunArgs, &ResolvedProfileTarget, &mut ProfileManifest) -> Result<()>,
{
    let target = resolve_profile_target(args)?;
    let mut session = ProfileSession::plan(args, &target)?;
    let profile_span = tracing::info_span!(
        "profile_run",
        target = ?args.target,
        provider = ?args.provider,
        backend = ?target.backend,
        function = %args.function,
        dry_run
    );
    let _profile_span = profile_span.enter();
    info!(
        output_dir = %session.run_output_dir().display(),
        "resolved profile run"
    );
    let execution_result = if dry_run {
        info!("planning profile capture only");
        session.manifest_mut().capture_metadata.warnings.push(
            "dry-run enabled; capture planning stopped before execution and recorded the planned artifact contract only"
                .into(),
        );
        Ok(())
    } else {
        info!("executing profile capture");
        execute(args, &target, session.manifest_mut())
    };

    if session.should_persist_outputs(dry_run, execution_result.is_ok()) {
        info!("writing profile session outputs");
        session.persist(args)?;
    }
    execution_result?;

    println!(
        "Profile session written to {}",
        session.profile_path().display()
    );
    println!(
        "Profile summary written to {}",
        session.summary_path().display()
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

pub(super) fn build_capture_plan(
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
                    label: "frame-locations".into(),
                    path: processed_root.join("frame-locations.json"),
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
                ArtifactRecord {
                    label: "chronological-trace".into(),
                    path: processed_root.join("chronological-trace.json"),
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
            timeline_path: Some(output_root.join("artifacts/semantic/timeline.json")),
            ..SemanticProfileRecord::default()
        },
        capture_metadata: CaptureMetadataRecord {
            device: target
                .device
                .as_ref()
                .map(|device| device.identifier.clone()),
            os: target
                .device
                .as_ref()
                .map(|device| format!("{} {}", device.os, device.os_version)),
            sample_duration_secs: None,
            benchmark_iterations: None,
            benchmark_warmup: None,
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
