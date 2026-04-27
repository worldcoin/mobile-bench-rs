use anyhow::{Result, bail};

use super::{
    CaptureStatus, ProfileBackend, ProfileManifest, ProfileProvider, ProfileRunArgs,
    ResolvedProfileTarget, execute_local_android_capture, execute_local_ios_capture,
};

pub(super) fn execute(
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
            return execute_with_local_android_executor(
                args,
                manifest,
                execute_local_android_capture,
            );
        }
        (ProfileProvider::Local, ProfileBackend::IosInstruments) => {
            return execute_with_local_ios_executor(args, manifest, execute_local_ios_capture);
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

pub(super) fn execute_with_local_android_executor<E>(
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

pub(super) fn execute_with_local_ios_executor<E>(
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
