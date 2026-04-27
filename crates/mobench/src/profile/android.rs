use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use mobench_sdk::types::NativeLibraryArtifact;

use crate::{
    MobileTarget, ProjectLayoutOptions, RunSpec,
    benchmark_output::{
        ANDROID_BENCH_LOG_MARKER, extract_benchmark_reports_from_logs,
        select_benchmark_value_for_function,
    },
    load_dotenv_for_layout, persist_mobile_spec, resolve_project_layout, run_android_build,
    validate_benchmark_function,
};

use super::{
    ANDROID_BENCHMARK_ANCHORS, CaptureStatus, CaptureWarmupMode,
    DEFAULT_ANDROID_CAPTURE_DURATION_SECS, DEFAULT_ANDROID_WARMUP_TIMEOUT_SECS,
    DEFAULT_PROFILE_ITERATIONS, DEFAULT_PROFILE_WARMUP, FrameLocationRecord, ProfileManifest,
    ProfileRunArgs, SymbolizationRecord, semantic, split_folded_stack_line,
    write_dual_view_flamegraph_bundle,
};

#[derive(Debug, Clone)]
struct AndroidProfilerToolchain {
    sdk_root: PathBuf,
    adb_path: PathBuf,
    app_profiler_path: PathBuf,
    stackcollapse_path: PathBuf,
    python_path: PathBuf,
    llvm_addr2line_path: PathBuf,
}

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

pub(crate) fn write_android_symbolized_outputs(
    folded_stacks: &str,
    native_libraries: &[NativeLibraryArtifact],
    processed_root: &Path,
    runtime_abi: Option<&str>,
    llvm_addr2line_path: &Path,
) -> Result<SymbolizationRecord> {
    let record = write_android_symbolized_outputs_with_resolver(
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
    )?;
    write_android_frame_location_sidecar(
        folded_stacks,
        native_libraries,
        processed_root,
        runtime_abi,
        llvm_addr2line_path,
    )?;
    Ok(record)
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

    let (symbolized_stacks, mut record, report) =
        symbolize_android_folded_stacks_with_native_libraries(
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

fn write_android_frame_location_sidecar(
    folded_stacks: &str,
    native_libraries: &[NativeLibraryArtifact],
    processed_root: &Path,
    runtime_abi: Option<&str>,
    llvm_addr2line_path: &Path,
) -> Result<()> {
    let records = collect_android_frame_location_records(
        folded_stacks,
        native_libraries,
        runtime_abi,
        llvm_addr2line_path,
    )?;
    if records.is_empty() {
        return Ok(());
    }
    let sidecar_path = processed_root.join("frame-locations.json");
    std::fs::write(&sidecar_path, serde_json::to_vec_pretty(&records)?)
        .with_context(|| format!("writing {}", sidecar_path.display()))?;
    Ok(())
}

fn collect_android_frame_location_records(
    folded_stacks: &str,
    native_libraries: &[NativeLibraryArtifact],
    runtime_abi: Option<&str>,
    llvm_addr2line_path: &Path,
) -> Result<Vec<FrameLocationRecord>> {
    let mut records = BTreeMap::<String, FrameLocationRecord>::new();
    for line in folded_stacks.lines().filter(|line| !line.trim().is_empty()) {
        let Some((stack, _count)) = split_folded_stack_line(line) else {
            continue;
        };
        for frame in stack.split(';') {
            let Some((library_name, offset)) = parse_android_native_offset_frame(frame) else {
                continue;
            };
            let Some(library_path) =
                resolve_android_native_library_path(native_libraries, library_name, runtime_abi)
            else {
                continue;
            };
            let Some(record) =
                resolve_android_frame_location_with_tool(llvm_addr2line_path, library_path, offset)
            else {
                continue;
            };
            records.entry(record.frame.clone()).or_insert(record);
        }
    }
    Ok(records.into_values().collect())
}

fn resolve_android_frame_location_with_tool(
    tool_path: &Path,
    library_path: &Path,
    offset: u64,
) -> Option<FrameLocationRecord> {
    let output = Command::new(tool_path)
        .args(["-Cfpe"])
        .arg(library_path)
        .arg(format!("0x{offset:x}"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_android_addr2line_frame_location(&String::from_utf8_lossy(&output.stdout))
}

fn parse_android_addr2line_frame_location(stdout: &str) -> Option<FrameLocationRecord> {
    let mut symbol = None::<String>;
    let mut location = None::<String>;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "??" || trimmed.starts_with("?? ") {
            continue;
        }
        if let Some((parsed_symbol, parsed_location)) = trimmed.split_once(" at ") {
            symbol = Some(parsed_symbol.trim().to_owned());
            if !parsed_location.trim().is_empty() && !parsed_location.starts_with("??") {
                location = Some(parsed_location.trim().to_owned());
                break;
            }
            continue;
        }
        if symbol.is_none() {
            symbol = Some(trimmed.to_owned());
            continue;
        }
        if !trimmed.starts_with("??") {
            location = Some(trimmed.to_owned());
            break;
        }
    }
    let symbol = symbol?;
    let location = location?;
    let (source_path, line) = parse_addr2line_location(&location)?;
    Some(FrameLocationRecord {
        frame: symbol,
        source_path,
        line,
    })
}

fn parse_addr2line_location(location: &str) -> Option<(PathBuf, u32)> {
    let trimmed = location
        .split(" (discriminator ")
        .next()
        .unwrap_or(location)
        .trim();
    if trimmed.is_empty() || trimmed.starts_with("??") {
        return None;
    }
    let (path, line) = trimmed.rsplit_once(':')?;
    Some((PathBuf::from(path), line.parse().ok()?))
}

fn parse_android_native_offset_frame(frame: &str) -> Option<(&str, u64)> {
    let marker = ".so[+";
    let marker_index = frame.find(marker)?;
    let library_end = marker_index + 3;
    let library_name = frame[..library_end].rsplit('/').next()?;
    let offset_start = marker_index + marker.len();
    let offset_end = frame[offset_start..].find(']')? + offset_start;
    let offset_raw = &frame[offset_start..offset_end];
    let offset = if let Some(hex) = offset_raw.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()?
    } else {
        offset_raw.parse().ok()?
    };
    Some((library_name, offset))
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

pub(crate) fn locate_android_llvm_addr2line(
    ndk_root: &Path,
    override_path: Option<&Path>,
) -> Result<PathBuf> {
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
    candidates.into_iter().next().context(
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

pub(crate) fn execute_local_android_capture(
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
        ios_completion_timeout_secs: None,
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
                semantic::merge_from_bench_report(manifest, report)?;
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

pub(crate) fn android_log_contains_marker(logcat: &str, marker: &str) -> bool {
    logcat.lines().any(|line| line.contains(marker))
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
