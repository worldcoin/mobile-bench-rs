use anyhow::Result;
use std::path::Path;

use super::{
    ArtifactRecord, ProfileManifest, ProfileRunArgs, refresh_flamegraph_viewer_from_manifest,
    render_profile_markdown, write_profile_manifest,
};

pub(super) fn persist_session_outputs(
    args: &ProfileRunArgs,
    run_output_dir: &Path,
    manifest: &ProfileManifest,
) -> Result<()> {
    std::fs::create_dir_all(&args.output_dir)?;
    std::fs::create_dir_all(run_output_dir)?;
    create_selected_artifact_roots(
        &manifest.native_capture.raw_artifacts,
        &manifest.native_capture.processed_artifacts,
    )?;
    let rendered_summary = render_profile_markdown(manifest);

    let run_profile_path = run_output_dir.join("profile.json");
    let run_summary_path = run_output_dir.join("summary.md");
    write_semantic_phase_sidecar(manifest)?;
    write_harness_timeline_sidecar(manifest)?;
    refresh_flamegraph_viewer_from_manifest(run_output_dir, manifest)?;
    write_profile_manifest(&run_profile_path, manifest)?;
    std::fs::write(&run_summary_path, rendered_summary.as_bytes())?;

    let latest_profile_path = args.output_dir.join("profile.json");
    let latest_summary_path = args.output_dir.join("summary.md");
    write_profile_manifest(&latest_profile_path, manifest)?;
    std::fs::write(&latest_summary_path, rendered_summary.as_bytes())?;
    Ok(())
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

fn write_harness_timeline_sidecar(manifest: &ProfileManifest) -> Result<()> {
    let Some(path) = manifest.semantic_profile.timeline_path.as_ref() else {
        return Ok(());
    };
    if manifest.semantic_profile.harness_timeline.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&manifest.semantic_profile.harness_timeline)?,
    )?;
    Ok(())
}
