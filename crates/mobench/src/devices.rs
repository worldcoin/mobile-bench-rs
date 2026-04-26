use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};

use crate::browserstack::{self, BrowserStackAuth, BrowserStackClient};
use crate::cli::{CheckOutputFormat, DevicePlatform};
use crate::{DeviceEntry, load_config, load_device_matrix, resolve_browserstack_credentials};

/// List available BrowserStack devices and optionally validate device specs.
pub(crate) fn cmd_devices(
    platform: Option<DevicePlatform>,
    output_json: bool,
    validate: Vec<String>,
) -> Result<()> {
    // Try to get credentials, but provide helpful error if missing
    let creds = match resolve_browserstack_credentials(None) {
        Ok(creds) => creds,
        Err(_) => {
            // Check what's missing and provide helpful guidance
            let username = env::var("BROWSERSTACK_USERNAME").ok();
            let access_key = env::var("BROWSERSTACK_ACCESS_KEY").ok();

            let missing_username = username.is_none() || username.as_deref() == Some("");
            let missing_access_key = access_key.is_none() || access_key.as_deref() == Some("");

            let error_msg =
                browserstack::format_credentials_error(missing_username, missing_access_key);
            bail!("{}", error_msg);
        }
    };

    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: creds.username,
            access_key: creds.access_key,
        },
        creds.project,
    )?;

    // If validating devices, do that and exit
    if !validate.is_empty() {
        let platform_str = platform.map(|p| match p {
            DevicePlatform::Android => "android",
            DevicePlatform::Ios => "ios",
        });

        let validation = client.validate_devices(&validate, platform_str)?;

        if output_json {
            let output = json!({
                "valid": validation.valid,
                "invalid": validation.invalid.iter().map(|e| {
                    json!({
                        "spec": e.spec,
                        "reason": e.reason,
                        "suggestions": e.suggestions
                    })
                }).collect::<Vec<_>>()
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            if !validation.valid.is_empty() {
                println!("Valid devices ({}):", validation.valid.len());
                for device in &validation.valid {
                    println!("  [OK] {}", device);
                }
            }

            if !validation.invalid.is_empty() {
                if !validation.valid.is_empty() {
                    println!();
                }
                println!("Invalid devices ({}):", validation.invalid.len());
                for error in &validation.invalid {
                    println!("  [ERROR] {}: {}", error.spec, error.reason);
                    if !error.suggestions.is_empty() {
                        println!("          Suggestions:");
                        for suggestion in &error.suggestions {
                            println!("            - {}", suggestion);
                        }
                    }
                }
            }
        }

        // Exit with error if any devices were invalid
        if !validation.invalid.is_empty() {
            bail!(
                "{} of {} device specs are invalid",
                validation.invalid.len(),
                validate.len()
            );
        }

        return Ok(());
    }

    // List devices
    println!("Fetching available BrowserStack devices...\n");

    let devices = match platform {
        Some(DevicePlatform::Android) => client.list_espresso_devices()?,
        Some(DevicePlatform::Ios) => client.list_xcuitest_devices()?,
        None => client.list_all_devices()?,
    };

    if devices.is_empty() {
        println!("No devices found.");
        return Ok(());
    }

    if output_json {
        println!("{}", serde_json::to_string_pretty(&devices)?);
        return Ok(());
    }

    // Group devices by OS
    let mut android_devices: Vec<_> = devices.iter().filter(|d| d.os == "android").collect();
    let mut ios_devices: Vec<_> = devices.iter().filter(|d| d.os == "ios").collect();

    // Sort by device name, then OS version (descending)
    android_devices.sort_by(|a, b| {
        a.device.cmp(&b.device).then_with(|| {
            // Try to compare versions numerically
            let av: f64 = a.os_version.parse().unwrap_or(0.0);
            let bv: f64 = b.os_version.parse().unwrap_or(0.0);
            bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    ios_devices.sort_by(|a, b| {
        a.device.cmp(&b.device).then_with(|| {
            let av: f64 = a.os_version.parse().unwrap_or(0.0);
            let bv: f64 = b.os_version.parse().unwrap_or(0.0);
            bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    if !android_devices.is_empty() {
        println!("Android Devices ({}):", android_devices.len());
        println!("{:-<60}", "");
        for device in &android_devices {
            println!("  {:40} OS {}", device.device, device.os_version);
            println!("    --devices \"{}\"", device.identifier());
        }
        println!();
    }

    if !ios_devices.is_empty() {
        println!("iOS Devices ({}):", ios_devices.len());
        println!("{:-<60}", "");
        for device in &ios_devices {
            println!("  {:40} iOS {}", device.device, device.os_version);
            println!("    --devices \"{}\"", device.identifier());
        }
        println!();
    }

    println!("Total: {} devices available", devices.len());
    println!("\nUsage:");
    println!("  cargo mobench run --target android --devices \"Google Pixel 7-13.0\" ...");
    println!("  cargo mobench run --target ios --devices \"iPhone 14-16\" ...");

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ResolvedMatrixDevice {
    pub(crate) name: String,
    pub(crate) os: String,
    pub(crate) os_version: String,
    pub(crate) identifier: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedDeviceProfile {
    pub(crate) profile: String,
    pub(crate) source: String,
    pub(crate) devices: Vec<ResolvedMatrixDevice>,
}

/// Built-in device profiles so `devices resolve` works without a YAML file.
pub(crate) fn builtin_device_for_profile(
    platform: DevicePlatform,
    profile: &str,
) -> Option<ResolvedMatrixDevice> {
    let (name, os, os_version) = match (platform, profile) {
        (DevicePlatform::Ios, "low-spec") => ("iPhone SE 2020", "ios", "16"),
        (DevicePlatform::Ios, "mid-spec") => ("iPhone 14", "ios", "16"),
        (DevicePlatform::Ios, "high-spec") => ("iPhone 16 Pro", "ios", "18"),
        (DevicePlatform::Android, "low-spec") => ("Motorola Moto G9 Play", "android", "10.0"),
        (DevicePlatform::Android, "mid-spec") => ("Google Pixel 7", "android", "13.0"),
        (DevicePlatform::Android, "high-spec") => ("Samsung Galaxy S24", "android", "14.0"),
        _ => return None,
    };
    Some(ResolvedMatrixDevice {
        identifier: format!("{name}-{os_version}"),
        name: name.to_string(),
        os: os.to_string(),
        os_version: os_version.to_string(),
        tags: vec![profile.to_string()],
    })
}

pub(crate) fn resolve_devices_for_profile(
    platform: DevicePlatform,
    profile: Option<&str>,
    config_path: Option<&Path>,
    device_matrix_path: Option<&Path>,
) -> Result<ResolvedDeviceProfile> {
    let profile_str = profile
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default");

    let (devices, source) = match resolve_matrix_for_cli(config_path, device_matrix_path) {
        Ok((matrix_path, config_tags)) => {
            let matrix = load_device_matrix(&matrix_path).with_context(|| {
                format!(
                    "config_error: failed to parse device matrix at {}",
                    matrix_path.display()
                )
            })?;
            let selected_tags = if profile.is_some() {
                vec![profile_str.to_string()]
            } else {
                config_tags
                    .filter(|tags| !tags.is_empty())
                    .unwrap_or_else(|| vec!["default".to_string()])
            };
            let devices = resolve_devices_from_matrix(matrix.devices, platform, &selected_tags)?;
            (devices, format!("matrix:{}", matrix_path.display()))
        }
        Err(_) => {
            if let Some(device) = builtin_device_for_profile(platform, profile_str) {
                (vec![device], "builtin".to_string())
            } else {
                bail!(
                    "No device matrix found and '{}' is not a built-in profile. \
                         Built-in profiles: low-spec, mid-spec, high-spec",
                    profile_str
                );
            }
        }
    };

    Ok(ResolvedDeviceProfile {
        profile: profile_str.to_string(),
        source,
        devices,
    })
}

pub(crate) fn cmd_devices_resolve(
    platform: DevicePlatform,
    profile: Option<String>,
    config_path: Option<&Path>,
    device_matrix_path: Option<&Path>,
    format: CheckOutputFormat,
) -> Result<()> {
    let resolved_profile = resolve_devices_for_profile(
        platform,
        profile.as_deref(),
        config_path,
        device_matrix_path,
    )?;
    let profile_str = resolved_profile.profile.as_str();
    let resolved = &resolved_profile.devices;
    let source = resolved_profile.source.as_str();

    match format {
        CheckOutputFormat::Text => {
            for device in resolved {
                println!("{}", device.identifier);
            }
        }
        CheckOutputFormat::Json => {
            let first: Option<&ResolvedMatrixDevice> = resolved.first();
            let output = json!({
                "platform": match platform {
                    DevicePlatform::Android => "android",
                    DevicePlatform::Ios => "ios",
                },
                "profile": profile_str,
                "source": source,
                "count": resolved.len(),
                "device": first.map(|d| &d.name),
                "name": first.map(|d| &d.name),
                "os_version": first.map(|d| &d.os_version),
                "devices": resolved,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
}

fn resolve_matrix_for_cli(
    config_path: Option<&Path>,
    device_matrix_path: Option<&Path>,
) -> Result<(PathBuf, Option<Vec<String>>)> {
    let mut discovered_matrix = None;
    let mut discovered_tags = None;

    if let Some(config_path) = config_path {
        let cfg = load_config(config_path)?;
        discovered_tags = cfg.device_tags.clone();
        discovered_matrix = Some(cfg.device_matrix);
    } else if device_matrix_path.is_none() {
        let default_config = PathBuf::from("bench-config.toml");
        if default_config.exists()
            && let Ok(cfg) = load_config(&default_config)
        {
            discovered_tags = cfg.device_tags.clone();
            discovered_matrix = Some(cfg.device_matrix);
        }
    }

    let matrix_path = device_matrix_path
        .map(PathBuf::from)
        .or(discovered_matrix)
        .or_else(|| {
            let fallback = PathBuf::from("device-matrix.yaml");
            if fallback.exists() {
                Some(fallback)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            anyhow!("config_error: provide --device-matrix, or provide --config with device_matrix")
        })?;

    Ok((matrix_path, discovered_tags))
}

pub(crate) fn resolve_devices_from_matrix(
    devices: Vec<DeviceEntry>,
    platform: DevicePlatform,
    tags: &[String],
) -> Result<Vec<ResolvedMatrixDevice>> {
    let wanted: Vec<String> = tags
        .iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    let platform_name = match platform {
        DevicePlatform::Android => "android",
        DevicePlatform::Ios => "ios",
    };

    let mut available_tags = BTreeSet::new();
    let mut resolved = Vec::new();

    for device in devices {
        if device.os.trim().to_lowercase() != platform_name {
            continue;
        }
        let normalized_tags: Vec<String> = device
            .tags
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|tag| tag.trim().to_lowercase())
            .filter(|tag| !tag.is_empty())
            .collect();
        for tag in &normalized_tags {
            available_tags.insert(tag.clone());
        }
        let tag_match = wanted.is_empty()
            || normalized_tags
                .iter()
                .any(|tag| wanted.iter().any(|wanted_tag| wanted_tag == tag));
        if !tag_match {
            continue;
        }
        let identifier = format!("{}-{}", device.name, device.os_version);
        resolved.push(ResolvedMatrixDevice {
            name: device.name,
            os: device.os,
            os_version: device.os_version,
            identifier,
            tags: normalized_tags,
        });
    }

    resolved.sort_by(|a, b| {
        a.identifier
            .cmp(&b.identifier)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.os_version.cmp(&b.os_version))
    });

    if resolved.is_empty() {
        if available_tags.is_empty() {
            bail!(
                "config_error: no devices matched platform `{}` and tags [{}]; no tag metadata found in matrix",
                platform_name,
                wanted.join(", ")
            );
        }
        bail!(
            "config_error: no devices matched platform `{}` and tags [{}]. Available tags: {}",
            platform_name,
            wanted.join(", "),
            available_tags.into_iter().collect::<Vec<_>>().join(", ")
        );
    }

    Ok(resolved)
}
