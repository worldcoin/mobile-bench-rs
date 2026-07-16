//! Fixture lifecycle commands and reproducible cache-key derivation.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::cli::{
    CheckOutputFormat, DevicePlatform, IosSigningMethodArg, MobileTarget, PlotFixture, SdkTarget,
};
use crate::devices::resolve_devices_from_matrix;
use crate::doctor::{
    PrereqCheck, collect_issues, print_check_results_json, print_check_results_text,
};
use crate::plots;
use crate::process_adapter::ToolCommand;
use crate::project_layout::{write_config_template, write_device_matrix_template};
use crate::reporting::cmd_report_summarize;
use crate::run_spec::{load_config, load_device_matrix};
use crate::{BenchConfig, cmd_build, cmd_package_ipa, cmd_package_xcuitest, repo_root};

pub(crate) fn cmd_fixture_init(
    config_path: &Path,
    device_matrix_path: &Path,
    force: bool,
) -> Result<()> {
    write_config_template(config_path, MobileTarget::Android, force)?;
    write_device_matrix_template(device_matrix_path, force)?;
    println!(
        "Initialized fixture files:\n  - {}\n  - {}",
        config_path.display(),
        device_matrix_path.display()
    );
    Ok(())
}

pub(crate) fn cmd_fixture_build(
    target: SdkTarget,
    release: bool,
    output_dir: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    progress: bool,
) -> Result<()> {
    match target {
        SdkTarget::Android => cmd_build(
            SdkTarget::Android,
            release,
            None,
            None,
            None,
            None,
            output_dir,
            crate_path,
            false,
            false,
            progress,
        )?,
        SdkTarget::Ios => {
            cmd_build(
                SdkTarget::Ios,
                release,
                None,
                None,
                None,
                None,
                output_dir.clone(),
                crate_path,
                false,
                false,
                progress,
            )?;
            cmd_package_ipa(
                "BenchRunner",
                IosSigningMethodArg::Adhoc,
                None,
                None,
                output_dir.clone(),
            )?;
            cmd_package_xcuitest("BenchRunner", None, None, output_dir)?;
        }
        SdkTarget::Both => {
            cmd_build(
                SdkTarget::Android,
                release,
                None,
                None,
                None,
                None,
                output_dir.clone(),
                crate_path.clone(),
                false,
                false,
                progress,
            )?;
            cmd_build(
                SdkTarget::Ios,
                release,
                None,
                None,
                None,
                None,
                output_dir.clone(),
                crate_path,
                false,
                false,
                progress,
            )?;
            cmd_package_ipa(
                "BenchRunner",
                IosSigningMethodArg::Adhoc,
                None,
                None,
                output_dir.clone(),
            )?;
            cmd_package_xcuitest("BenchRunner", None, None, output_dir)?;
        }
    }
    Ok(())
}

pub(crate) fn cmd_fixture_verify_plots(
    fixture: PlotFixture,
    output_dir: Option<&Path>,
) -> Result<()> {
    let (summary_path, default_output_dir, expected_plots): (&str, &str, &[&str]) = match fixture {
        PlotFixture::Basic => (
            "examples/fixtures/basic/summary.json",
            "target/mobench/plot-fixtures/basic",
            &["fibonacci.svg", "checksum.svg"],
        ),
        PlotFixture::Ffi => (
            "examples/fixtures/ffi/summary.json",
            "target/mobench/plot-fixtures/ffi",
            &["fibonacci.svg", "checksum.svg"],
        ),
    };

    let repo = repo_root()?;
    let summary_path = repo.join(summary_path);
    let output_dir = output_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join(default_output_dir));
    let markdown_path = output_dir.join("summary.md");

    if output_dir.exists() {
        fs::remove_dir_all(&output_dir)
            .with_context(|| format!("removing plot fixture output {}", output_dir.display()))?;
    }
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("creating plot fixture output {}", output_dir.display()))?;

    let markdown = cmd_report_summarize(
        &summary_path,
        Some(&markdown_path),
        plots::PlotMode::Require,
    )?;

    if !markdown.contains("## Device Comparison Plots") {
        bail!(
            "expected Device Comparison Plots section in {}",
            markdown_path.display()
        );
    }

    for plot in expected_plots {
        let plot_path = output_dir.join("plots").join(plot);
        if !plot_path.is_file() || fs::metadata(&plot_path)?.len() == 0 {
            bail!("expected rendered plot at {}", plot_path.display());
        }

        let expected_link = format!("](plots/{plot})");
        if !markdown.contains(&expected_link) {
            bail!(
                "expected markdown link {} in {}",
                expected_link,
                markdown_path.display()
            );
        }
    }

    println!(
        "Verified plot fixture {:?} in {}",
        fixture,
        output_dir.display()
    );
    Ok(())
}

pub(crate) fn cmd_fixture_verify(
    config_path: &Path,
    device_matrix_override: Option<&Path>,
    target: SdkTarget,
    profile: Option<String>,
    format: CheckOutputFormat,
) -> Result<()> {
    let mut checks = Vec::new();
    let mut cfg: Option<BenchConfig> = None;
    match load_config(config_path) {
        Ok(parsed) => {
            checks.push(PrereqCheck {
                name: "Run config".to_string(),
                passed: true,
                detail: Some(config_path.display().to_string()),
                fix_hint: None,
            });
            cfg = Some(parsed);
        }
        Err(err) => {
            checks.push(PrereqCheck {
                name: "Run config".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some(format!("Fix config at {}", config_path.display())),
            });
        }
    }

    let matrix_path = device_matrix_override
        .map(PathBuf::from)
        .or_else(|| cfg.as_ref().map(|c| c.device_matrix.clone()));
    if let Some(matrix_path) = matrix_path.as_deref() {
        match load_device_matrix(matrix_path) {
            Ok(matrix) => {
                let mut tags = profile
                    .as_ref()
                    .map(|tag| vec![tag.clone()])
                    .or_else(|| cfg.as_ref().and_then(|c| c.device_tags.clone()))
                    .unwrap_or_else(|| vec!["default".to_string()]);
                tags.retain(|tag| !tag.trim().is_empty());

                let platforms = match target {
                    SdkTarget::Android => vec![DevicePlatform::Android],
                    SdkTarget::Ios => vec![DevicePlatform::Ios],
                    SdkTarget::Both => vec![DevicePlatform::Android, DevicePlatform::Ios],
                };

                let mut unresolved = Vec::new();
                for platform in platforms {
                    if let Err(err) =
                        resolve_devices_from_matrix(matrix.devices.clone(), platform, &tags)
                    {
                        unresolved.push(err.to_string());
                    }
                }
                if unresolved.is_empty() {
                    checks.push(PrereqCheck {
                        name: "Device matrix".to_string(),
                        passed: true,
                        detail: Some(format!(
                            "{} (tags: {})",
                            matrix_path.display(),
                            tags.join(", ")
                        )),
                        fix_hint: None,
                    });
                } else {
                    checks.push(PrereqCheck {
                        name: "Device matrix".to_string(),
                        passed: false,
                        detail: Some(unresolved.join("; ")),
                        fix_hint: Some(format!(
                            "Adjust tags/profile or matrix entries in {}",
                            matrix_path.display()
                        )),
                    });
                }
            }
            Err(err) => checks.push(PrereqCheck {
                name: "Device matrix".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some(format!(
                    "Fix or regenerate device matrix at {}",
                    matrix_path.display()
                )),
            }),
        }
    } else {
        checks.push(PrereqCheck {
            name: "Device matrix".to_string(),
            passed: false,
            detail: Some("missing device matrix path".to_string()),
            fix_hint: Some(
                "Provide --device-matrix or set device_matrix in bench-config.toml".to_string(),
            ),
        });
    }

    let cargo_lock_path = repo_root()?.join("Cargo.lock");
    checks.push(PrereqCheck {
        name: "Cargo.lock".to_string(),
        passed: cargo_lock_path.exists(),
        detail: Some(cargo_lock_path.display().to_string()),
        fix_hint: if cargo_lock_path.exists() {
            None
        } else {
            Some("Run cargo generate-lockfile".to_string())
        },
    });

    let issues = collect_issues(&checks);
    match format {
        CheckOutputFormat::Text => print_check_results_text(&checks, &issues),
        CheckOutputFormat::Json => print_check_results_json(&checks, &issues)?,
    }
    if issues.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} issue(s) found. Fix them and rerun `cargo mobench fixture verify`.",
            issues.len()
        )
    }
}

pub(crate) fn cmd_fixture_cache_key(
    config_path: &Path,
    device_matrix_override: Option<&Path>,
    target: SdkTarget,
    profile: Option<String>,
    format: CheckOutputFormat,
) -> Result<()> {
    let cfg = load_config(config_path)
        .with_context(|| format!("config_error: failed to load {}", config_path.display()))?;
    let matrix_path = device_matrix_override
        .map(PathBuf::from)
        .unwrap_or_else(|| cfg.device_matrix.clone());
    let matrix_bytes = fs::read(&matrix_path).with_context(|| {
        format!(
            "config_error: failed to read device matrix {}",
            matrix_path.display()
        )
    })?;
    let config_bytes = fs::read(config_path)
        .with_context(|| format!("config_error: failed to read {}", config_path.display()))?;
    let cargo_lock_path = repo_root()?.join("Cargo.lock");
    let cargo_lock_bytes = if cargo_lock_path.exists() {
        fs::read(&cargo_lock_path)?
    } else {
        Vec::new()
    };

    let rustc_version = command_version_line("rustc", &["--version"]).unwrap_or_default();
    let cargo_version = command_version_line("cargo", &["--version"]).unwrap_or_default();
    let selected_profile = profile
        .or_else(|| {
            cfg.device_tags
                .clone()
                .and_then(|mut tags| tags.drain(..1).next())
        })
        .unwrap_or_else(|| "default".to_string());

    let mut hasher = Sha256::new();
    hasher.update(format!("mobench={}\n", env!("CARGO_PKG_VERSION")).as_bytes());
    hasher.update(format!("target={target:?}\n").as_bytes());
    hasher.update(format!("profile={selected_profile}\n").as_bytes());
    hasher.update(format!("rustc={rustc_version}\n").as_bytes());
    hasher.update(format!("cargo={cargo_version}\n").as_bytes());
    hasher.update(config_bytes);
    hasher.update(matrix_bytes);
    hasher.update(cargo_lock_bytes);
    let digest = hasher.finalize();
    let cache_key = format!("mobench-fixture-{:x}", digest);

    match format {
        CheckOutputFormat::Text => println!("{cache_key}"),
        CheckOutputFormat::Json => {
            let payload = json!({
                "cache_key": cache_key,
                "target": format!("{target:?}").to_lowercase(),
                "profile": selected_profile,
                "config": config_path.display().to_string(),
                "device_matrix": matrix_path.display().to_string(),
                "rustc": rustc_version,
                "cargo": cargo_version,
                "mobench_version": env!("CARGO_PKG_VERSION"),
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
    }
    Ok(())
}

pub(crate) fn command_version_line(cmd: &str, args: &[&str]) -> Option<String> {
    let mut command = ToolCommand::explicit(cmd).ok()?;
    command.args(args).timeout(Duration::from_secs(30));
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .next()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
}
