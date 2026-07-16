//! Run-request resolution and device-matrix validation.
//!
//! This module resolves CLI/config inputs into one validated RunSpec before
//! build or provider orchestration starts.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use mobench_runtime::MAX_BENCHMARK_COUNT;

use crate::{
    BenchConfig, DeviceEntry, DeviceMatrix, IosRunnerArg, IosXcuitestArtifacts, MobileTarget,
    ResolvedProjectLayout, RunSpec, browserstack_identifier_and_os_version,
    configured_android_benchmark_timeout_secs, configured_android_heartbeat_interval_secs,
    configured_ios_completion_timeout_secs, configured_ios_deployment_target,
    configured_ios_runner, default_ios_xcuitest_artifacts, ios_runner_arg_name,
    resolve_project_relative_path,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_run_spec(
    target: Option<MobileTarget>,
    function: Option<String>,
    iterations: Option<u32>,
    warmup: Option<u32>,
    devices: Vec<String>,
    layout: &ResolvedProjectLayout,
    config: Option<&Path>,
    device_matrix: Option<&Path>,
    device_tags: Vec<String>,
    ios_app: Option<PathBuf>,
    ios_test_suite: Option<PathBuf>,
    ios_completion_timeout_secs: Option<u64>,
    ios_deployment_target: Option<String>,
    ios_runner: Option<IosRunnerArg>,
    android_benchmark_timeout_secs: Option<u64>,
    android_heartbeat_interval_secs: Option<u64>,
    local_only: bool,
    _release: bool,
    dry_run: bool,
) -> Result<RunSpec> {
    if let Some(cfg_path) = config {
        let cfg = load_config(cfg_path)?;
        let resolved_target = target.unwrap_or(cfg.target);
        let configured_ios_completion_timeout_secs = ios_completion_timeout_secs
            .or(cfg.browserstack.ios_completion_timeout_secs)
            .or(layout.ios_completion_timeout_secs);
        let (configured_ios_deployment_target, configured_ios_runner) =
            if resolved_target == MobileTarget::Ios {
                let deployment_target =
                    configured_ios_deployment_target(layout, ios_deployment_target.as_deref())?;
                let runner_name = ios_runner.map(ios_runner_arg_name);
                let runner = configured_ios_runner(layout, &deployment_target, runner_name)?;
                (Some(deployment_target), Some(runner))
            } else {
                (None, None)
            };
        let configured_android_benchmark_timeout_secs = android_benchmark_timeout_secs
            .or(cfg.browserstack.android_benchmark_timeout_secs)
            .or(layout.android_benchmark_timeout_secs);
        let configured_android_heartbeat_interval_secs = android_heartbeat_interval_secs
            .or(cfg.browserstack.android_heartbeat_interval_secs)
            .or(layout.android_heartbeat_interval_secs);
        if device_matrix.is_some() && !devices.is_empty() {
            bail!(
                "--device-matrix cannot be combined with --devices; choose one source for devices"
            );
        }
        let matrix_path = device_matrix.map(Path::to_path_buf).unwrap_or_else(|| {
            resolve_project_relative_path(
                cfg_path.parent().unwrap_or_else(|| Path::new(".")),
                cfg.device_matrix.as_path(),
            )
        });
        let resolved_tags = if !device_tags.is_empty() {
            Some(device_tags)
        } else {
            cfg.device_tags.clone()
        };
        let device_names = if !devices.is_empty() {
            if resolved_target == MobileTarget::Ios {
                validate_ios_device_specs_support_deployment_target(
                    &devices,
                    configured_ios_deployment_target
                        .as_ref()
                        .expect("iOS deployment target should be resolved"),
                )?;
            }
            devices
        } else {
            let matrix = load_device_matrix(&matrix_path)?;
            match resolved_tags.as_ref() {
                Some(tags) if !tags.is_empty() => {
                    let entries = filter_device_entries_by_tags(matrix.devices, tags)?;
                    if resolved_target == MobileTarget::Ios {
                        validate_ios_device_entries_support_deployment_target(
                            &entries,
                            configured_ios_deployment_target
                                .as_ref()
                                .expect("iOS deployment target should be resolved"),
                        )?;
                    }
                    entries.into_iter().map(|d| d.name).collect()
                }
                _ => {
                    if resolved_target == MobileTarget::Ios {
                        validate_ios_device_entries_support_deployment_target(
                            &matrix.devices,
                            configured_ios_deployment_target
                                .as_ref()
                                .expect("iOS deployment target should be resolved"),
                        )?;
                    }
                    matrix.devices.into_iter().map(|d| d.name).collect()
                }
            }
        };
        let ios_xcuitest = match (ios_app, ios_test_suite) {
            (Some(app), Some(test_suite)) => Some(IosXcuitestArtifacts { app, test_suite }),
            (None, None) => cfg.ios_xcuitest,
            _ => bail!(
                "both --ios-app and --ios-test-suite must be provided together; omit both to use config-managed iOS artifacts"
            ),
        };
        let resolved_iterations = iterations.unwrap_or(cfg.iterations);
        let resolved_warmup = warmup.unwrap_or(cfg.warmup);
        validate_run_counts(resolved_iterations, resolved_warmup)?;
        return Ok(RunSpec {
            target: resolved_target,
            function: function.unwrap_or(cfg.function),
            iterations: resolved_iterations,
            warmup: resolved_warmup,
            devices: device_names,
            ios_completion_timeout_secs: configured_ios_completion_timeout_secs,
            ios_deployment_target: configured_ios_deployment_target
                .map(|target| target.to_string()),
            ios_runner: configured_ios_runner.map(|runner| runner.as_str().to_string()),
            android_benchmark_timeout_secs: configured_android_benchmark_timeout_secs,
            android_heartbeat_interval_secs: configured_android_heartbeat_interval_secs,
            browserstack: Some(cfg.browserstack),
            ios_xcuitest,
        });
    }

    let target =
        target.context("target must be provided with --target or set in the config file")?;
    let (configured_ios_deployment_target, configured_ios_runner) = if target == MobileTarget::Ios {
        let deployment_target =
            configured_ios_deployment_target(layout, ios_deployment_target.as_deref())?;
        let runner_name = ios_runner.map(ios_runner_arg_name);
        let runner = configured_ios_runner(layout, &deployment_target, runner_name)?;
        (Some(deployment_target), Some(runner))
    } else {
        (None, None)
    };
    let function = function.unwrap_or_default();
    let iterations = iterations.unwrap_or(100);
    let warmup = warmup.unwrap_or(10);
    validate_run_counts(iterations, warmup)?;

    if function.trim().is_empty() {
        bail!(
            "function must not be empty; pass --function <crate::fn> or set function in the config file"
        );
    }

    if device_matrix.is_some() && !devices.is_empty() {
        bail!("--device-matrix cannot be combined with --devices; choose one source for devices");
    }
    if device_matrix.is_none() && !device_tags.is_empty() {
        bail!("--device-tags requires --device-matrix or a config file with device tags");
    }

    let resolved_devices = if !devices.is_empty() {
        if target == MobileTarget::Ios {
            validate_ios_device_specs_support_deployment_target(
                &devices,
                configured_ios_deployment_target
                    .as_ref()
                    .expect("iOS deployment target should be resolved"),
            )?;
        }
        devices
    } else if let Some(matrix_path) = device_matrix {
        let matrix = load_device_matrix(matrix_path)?;
        if device_tags.is_empty() {
            if target == MobileTarget::Ios {
                validate_ios_device_entries_support_deployment_target(
                    &matrix.devices,
                    configured_ios_deployment_target
                        .as_ref()
                        .expect("iOS deployment target should be resolved"),
                )?;
            }
            matrix.devices.into_iter().map(|d| d.name).collect()
        } else {
            let entries = filter_device_entries_by_tags(matrix.devices, &device_tags)?;
            if target == MobileTarget::Ios {
                validate_ios_device_entries_support_deployment_target(
                    &entries,
                    configured_ios_deployment_target
                        .as_ref()
                        .expect("iOS deployment target should be resolved"),
                )?;
            }
            entries.into_iter().map(|d| d.name).collect()
        }
    } else {
        Vec::new()
    };

    let ios_xcuitest = match (ios_app, ios_test_suite) {
        (Some(app), Some(test_suite)) => Some(IosXcuitestArtifacts { app, test_suite }),
        (None, None) => None,
        _ => bail!(
            "both --ios-app and --ios-test-suite must be provided together; omit both to let mobench package iOS artifacts when running against devices"
        ),
    };

    let ios_xcuitest = if target == MobileTarget::Ios
        && !local_only
        && !resolved_devices.is_empty()
        && ios_xcuitest.is_none()
    {
        if dry_run {
            println!("📦 [dry-run] Would auto-package iOS artifacts for BrowserStack...");
        }
        Some(default_ios_xcuitest_artifacts(layout))
    } else {
        ios_xcuitest
    };

    Ok(RunSpec {
        target,
        function,
        iterations,
        warmup,
        devices: resolved_devices,
        ios_completion_timeout_secs: configured_ios_completion_timeout_secs(
            layout,
            ios_completion_timeout_secs,
        ),
        ios_deployment_target: configured_ios_deployment_target.map(|target| target.to_string()),
        ios_runner: configured_ios_runner.map(|runner| runner.as_str().to_string()),
        android_benchmark_timeout_secs: configured_android_benchmark_timeout_secs(
            layout,
            android_benchmark_timeout_secs,
        ),
        android_heartbeat_interval_secs: configured_android_heartbeat_interval_secs(
            layout,
            android_heartbeat_interval_secs,
        ),
        browserstack: None,
        ios_xcuitest,
    })
}

pub(crate) fn validate_run_counts(iterations: u32, warmup: u32) -> Result<()> {
    if iterations == 0 {
        bail!("iterations must be greater than zero");
    }
    if iterations > MAX_BENCHMARK_COUNT {
        bail!("iterations must not exceed {MAX_BENCHMARK_COUNT} (got {iterations})");
    }
    if warmup > MAX_BENCHMARK_COUNT {
        bail!("warmup must not exceed {MAX_BENCHMARK_COUNT} (got {warmup})");
    }
    Ok(())
}

pub(crate) fn load_config(path: &Path) -> Result<BenchConfig> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading config {:?}", path))?;
    toml::from_str(&contents).with_context(|| format!("parsing config {:?}", path))
}

pub(crate) fn load_device_matrix(path: &Path) -> Result<DeviceMatrix> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading device matrix {:?}", path))?;
    serde_yaml::from_str(&contents).with_context(|| format!("parsing device matrix {:?}", path))
}

pub(crate) fn filter_devices_by_tags(
    devices: Vec<DeviceEntry>,
    tags: &[String],
) -> Result<Vec<String>> {
    Ok(filter_device_entries_by_tags(devices, tags)?
        .into_iter()
        .map(|d| d.name)
        .collect())
}

pub(crate) fn filter_device_entries_by_tags(
    devices: Vec<DeviceEntry>,
    tags: &[String],
) -> Result<Vec<DeviceEntry>> {
    let wanted: Vec<String> = tags
        .iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    if wanted.is_empty() {
        return Ok(devices);
    }

    let mut matched = Vec::new();
    let mut available_tags = BTreeSet::new();
    for device in devices {
        let Some(device_tags) = device.tags.as_ref() else {
            continue;
        };
        for tag in device_tags {
            let normalized = tag.trim().to_lowercase();
            if !normalized.is_empty() {
                available_tags.insert(normalized);
            }
        }
        let has_match = device_tags.iter().any(|tag| {
            let candidate = tag.trim().to_lowercase();
            wanted.iter().any(|wanted_tag| wanted_tag == &candidate)
        });
        if has_match {
            matched.push(device);
        }
    }

    if matched.is_empty() {
        if available_tags.is_empty() {
            bail!(
                "no devices matched tags [{}] in device matrix; no tag metadata found in the matrix",
                wanted.join(", ")
            );
        }
        let available = available_tags.into_iter().collect::<Vec<_>>().join(", ");
        bail!(
            "no devices matched tags [{}] in device matrix. Available tags: {}",
            wanted.join(", "),
            available
        );
    }
    Ok(matched)
}

pub(crate) fn parse_ios_version_from_device_identifier(spec: &str) -> Option<&str> {
    let dash_pos = spec.rfind('-')?;
    let version = spec[dash_pos + 1..].trim();
    version
        .chars()
        .next()
        .filter(|ch| ch.is_ascii_digit())
        .map(|_| version)
}

pub(crate) fn ios_device_version_is_supported(
    device_version: &str,
    deployment_target: &mobench_sdk::codegen::IosDeploymentTarget,
) -> Result<bool> {
    let device_target =
        mobench_sdk::codegen::IosDeploymentTarget::parse(device_version).map_err(|err| {
            anyhow!("config_error: invalid iOS device version `{device_version}`: {err}")
        })?;
    Ok(&device_target >= deployment_target)
}

pub(crate) fn validate_ios_device_specs_support_deployment_target(
    devices: &[String],
    deployment_target: &mobench_sdk::codegen::IosDeploymentTarget,
) -> Result<()> {
    for device in devices {
        let Some(os_version) = parse_ios_version_from_device_identifier(device) else {
            continue;
        };
        if !ios_device_version_is_supported(os_version, deployment_target)? {
            bail!(
                "`{}` cannot run app with iOS deployment target `{}`.",
                device,
                deployment_target
            );
        }
    }
    Ok(())
}

pub(crate) fn validate_ios_device_entries_support_deployment_target(
    devices: &[DeviceEntry],
    deployment_target: &mobench_sdk::codegen::IosDeploymentTarget,
) -> Result<()> {
    for device in devices {
        if !device.os.eq_ignore_ascii_case("ios") {
            continue;
        }
        let parsed_from_name = parse_ios_version_from_device_identifier(&device.name);
        let os_version = if device.os_version.trim().is_empty() {
            parsed_from_name
        } else {
            Some(device.os_version.trim())
        };
        let Some(os_version) = os_version else {
            continue;
        };
        if !ios_device_version_is_supported(os_version, deployment_target)? {
            let (identifier, _) =
                browserstack_identifier_and_os_version(&device.name, &device.os_version);
            bail!(
                "`{}` cannot run app with iOS deployment target `{}`.",
                identifier,
                deployment_target
            );
        }
    }
    Ok(())
}
