use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{
    MobileTarget, ProjectLayoutOptions, RunSpec, load_dotenv_for_layout, persist_mobile_spec,
    resolve_project_layout, run_ios_build, validate_benchmark_function,
};

use super::{
    CaptureStatus, CaptureWarmupMode, DEFAULT_IOS_BENCH_DELAY_MS,
    DEFAULT_IOS_CAPTURE_DURATION_SECS, DEFAULT_IOS_LOG_TIMEOUT_SECS,
    DEFAULT_IOS_PROFILE_REPEAT_UNTIL_MS, DEFAULT_PROFILE_ITERATIONS, DEFAULT_PROFILE_WARMUP,
    IOS_BENCHMARK_ANCHORS, ProfileManifest, ProfileRunArgs, ResolvedProfileDevice,
    SymbolizationRecord, resolve_profile_device, semantic, write_dual_view_flamegraph_bundle,
};

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

pub(crate) fn execute_local_ios_capture(
    args: &ProfileRunArgs,
    manifest: &mut ProfileManifest,
) -> Result<()> {
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
        ios_completion_timeout_secs: None,
        browserstack: None,
        ios_xcuitest: None,
    };
    persist_mobile_spec(&layout, &spec, false)?;

    let requested_device = resolve_profile_device(args)?;
    let simulator = resolve_local_ios_simulator(requested_device.as_ref())?;
    ensure_local_ios_simulator_booted(&simulator)?;
    manifest.capture_metadata.device = Some(simulator.identifier());

    run_ios_build(&layout, false, false, None)?;
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
        format!("--mobench-profile-repeat-until-ms={DEFAULT_IOS_PROFILE_REPEAT_UNTIL_MS}"),
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
            if let Some(report) = crate::benchmark_output::extract_ios_benchmark_json(&logs) {
                semantic::merge_from_bench_report(manifest, &report)?;
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
pub(crate) fn collapse_ios_sample_call_graph_to_folded_stacks(
    sample_output: &str,
) -> Result<String> {
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
