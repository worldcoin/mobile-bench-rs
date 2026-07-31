use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::process_adapter::ToolCommand;
use crate::{
    BenchConfig, CheckOutputFormat, ContractErrorCategory, SdkTarget, command_version_line,
    filter_devices_by_tags, load_config, load_device_matrix, resolve_browserstack_credentials,
};

/// Check prerequisites for building mobile artifacts.
///
/// This validates that all required tools and configurations are in place
/// before attempting a build.
pub(crate) fn cmd_check(target: SdkTarget, format: CheckOutputFormat) -> Result<()> {
    let checks = collect_prereq_checks(target);
    let issues = collect_issues(&checks);

    match format {
        CheckOutputFormat::Text => print_check_results_text(&checks, &issues),
        CheckOutputFormat::Json => print_check_results_json(&checks, &issues)?,
    }

    if issues.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} issue(s) found. Fix them and run 'cargo mobench check --target {:?}' again.",
            issues.len(),
            target
        )
    }
}

pub(crate) fn cmd_config_validate(config_path: &Path, format: CheckOutputFormat) -> Result<()> {
    let mut checks = Vec::new();
    let mut config: Option<BenchConfig> = None;

    match load_config(config_path) {
        Ok(cfg) => {
            checks.push(PrereqCheck {
                name: "Run config".to_string(),
                passed: true,
                detail: Some(config_path.display().to_string()),
                fix_hint: None,
            });
            config = Some(cfg);
        }
        Err(err) => {
            checks.push(PrereqCheck {
                name: "Run config".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some(format!(
                    "Fix config file syntax/fields at {}",
                    config_path.display()
                )),
            });
        }
    }

    if let Some(cfg) = &config {
        match load_device_matrix(&cfg.device_matrix) {
            Ok(matrix) => {
                if let Some(tags) = cfg.device_tags.as_ref().filter(|tags| !tags.is_empty()) {
                    if let Err(err) = filter_devices_by_tags(matrix.devices, tags) {
                        checks.push(PrereqCheck {
                            name: "Device matrix".to_string(),
                            passed: false,
                            detail: Some(err.to_string()),
                            fix_hint: Some(format!(
                                "Update tags in {} or adjust device_tags in config",
                                cfg.device_matrix.display()
                            )),
                        });
                    } else {
                        checks.push(PrereqCheck {
                            name: "Device matrix".to_string(),
                            passed: true,
                            detail: Some(format!(
                                "{} (tags: {})",
                                cfg.device_matrix.display(),
                                tags.join(", ")
                            )),
                            fix_hint: None,
                        });
                    }
                } else {
                    checks.push(PrereqCheck {
                        name: "Device matrix".to_string(),
                        passed: true,
                        detail: Some(cfg.device_matrix.display().to_string()),
                        fix_hint: None,
                    });
                }
            }
            Err(err) => {
                checks.push(PrereqCheck {
                    name: "Device matrix".to_string(),
                    passed: false,
                    detail: Some(err.to_string()),
                    fix_hint: Some(format!(
                        "Fix or regenerate device matrix at {}",
                        cfg.device_matrix.display()
                    )),
                });
            }
        }

        match resolve_browserstack_credentials(Some(&cfg.browserstack)) {
            Ok(creds) => checks.push(PrereqCheck {
                name: "BrowserStack credentials".to_string(),
                passed: true,
                detail: Some(format!("user {}", creds.username)),
                fix_hint: None,
            }),
            Err(err) => checks.push(PrereqCheck {
                name: "BrowserStack credentials".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some("Set BROWSERSTACK_USERNAME and BROWSERSTACK_ACCESS_KEY".to_string()),
            }),
        }
    }

    let issues = collect_issues(&checks);
    match format {
        CheckOutputFormat::Text => print_check_results_text(&checks, &issues),
        CheckOutputFormat::Json => print_check_results_json(&checks, &issues)?,
    }

    if issues.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} issue(s) found. Fix them and rerun 'cargo mobench config validate'.",
            issues.len()
        )
    }
}

pub(crate) fn cmd_doctor(
    target: SdkTarget,
    config_path: Option<&Path>,
    device_matrix_path: Option<&Path>,
    device_tags: Vec<String>,
    browserstack: bool,
    format: CheckOutputFormat,
) -> Result<()> {
    let mut checks = collect_prereq_checks(target);

    let mut config: Option<BenchConfig> = None;
    if let Some(path) = config_path {
        match load_config(path) {
            Ok(cfg) => {
                checks.push(PrereqCheck {
                    name: "Run config".to_string(),
                    passed: true,
                    detail: Some(path.display().to_string()),
                    fix_hint: None,
                });
                config = Some(cfg);
            }
            Err(err) => {
                checks.push(PrereqCheck {
                    name: "Run config".to_string(),
                    passed: false,
                    detail: Some(err.to_string()),
                    fix_hint: Some(format!("Fix or regenerate config at {}", path.display())),
                });
            }
        }
    } else {
        checks.push(PrereqCheck {
            name: "Run config".to_string(),
            passed: true,
            detail: Some("skipped (no --config)".to_string()),
            fix_hint: None,
        });
    }

    let resolved_matrix_path = device_matrix_path
        .map(PathBuf::from)
        .or_else(|| config.as_ref().map(|cfg| cfg.device_matrix.clone()));
    let resolved_tags = if !device_tags.is_empty() {
        Some(device_tags)
    } else {
        config.as_ref().and_then(|cfg| cfg.device_tags.clone())
    };

    if resolved_matrix_path.is_none() && resolved_tags.as_ref().is_some_and(|tags| !tags.is_empty())
    {
        checks.push(PrereqCheck {
            name: "Device matrix".to_string(),
            passed: false,
            detail: Some("device tags provided without a matrix file".to_string()),
            fix_hint: Some(
                "Provide --device-matrix or set device_matrix in the config".to_string(),
            ),
        });
    } else if let Some(path) = resolved_matrix_path.as_deref() {
        match load_device_matrix(path) {
            Ok(matrix) => {
                if let Some(tags) = resolved_tags.as_ref().filter(|tags| !tags.is_empty()) {
                    if let Err(err) = filter_devices_by_tags(matrix.devices, tags) {
                        checks.push(PrereqCheck {
                            name: "Device matrix".to_string(),
                            passed: false,
                            detail: Some(err.to_string()),
                            fix_hint: Some(format!(
                                "Update tags in {} or adjust --device-tags",
                                path.display()
                            )),
                        });
                    } else {
                        checks.push(PrereqCheck {
                            name: "Device matrix".to_string(),
                            passed: true,
                            detail: Some(format!("{} (tags: {})", path.display(), tags.join(", "))),
                            fix_hint: None,
                        });
                    }
                } else {
                    checks.push(PrereqCheck {
                        name: "Device matrix".to_string(),
                        passed: true,
                        detail: Some(path.display().to_string()),
                        fix_hint: None,
                    });
                }
            }
            Err(err) => checks.push(PrereqCheck {
                name: "Device matrix".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some(format!(
                    "Fix or regenerate device matrix at {}",
                    path.display()
                )),
            }),
        }
    } else {
        checks.push(PrereqCheck {
            name: "Device matrix".to_string(),
            passed: true,
            detail: Some("skipped (no --device-matrix)".to_string()),
            fix_hint: None,
        });
    }

    if browserstack {
        let cfg_ref = config.as_ref().map(|cfg| &cfg.browserstack);
        match resolve_browserstack_credentials(cfg_ref) {
            Ok(creds) => checks.push(PrereqCheck {
                name: "BrowserStack credentials".to_string(),
                passed: true,
                detail: Some(format!("user {}", creds.username)),
                fix_hint: None,
            }),
            Err(err) => checks.push(PrereqCheck {
                name: "BrowserStack credentials".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some("Set BROWSERSTACK_USERNAME and BROWSERSTACK_ACCESS_KEY".to_string()),
            }),
        }
    } else {
        checks.push(PrereqCheck {
            name: "BrowserStack credentials".to_string(),
            passed: true,
            detail: Some("skipped (--browserstack=false)".to_string()),
            fix_hint: None,
        });
    }

    let issues = collect_issues(&checks);
    match format {
        CheckOutputFormat::Text => print_check_results_text(&checks, &issues),
        CheckOutputFormat::Json => print_check_results_json(&checks, &issues)?,
    }

    if issues.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} issue(s) found. Fix them and rerun 'cargo mobench doctor'.",
            issues.len()
        )
    }
}

pub(crate) fn collect_prereq_checks(target: SdkTarget) -> Vec<PrereqCheck> {
    let mut checks: Vec<PrereqCheck> = Vec::new();
    checks.push(check_cargo());
    checks.push(check_rustup());
    checks.push(check_rustc_msrv());

    match target {
        SdkTarget::Android => {
            println!("Checking prerequisites for Android...\n");
            extend_android_prereq_checks(&mut checks);
        }
        SdkTarget::Ios => {
            println!("Checking prerequisites for iOS...\n");
            checks.push(check_xcode());
            checks.push(check_xcodegen());
            checks.push(check_rust_target("aarch64-apple-ios"));
            checks.push(check_rust_target("aarch64-apple-ios-sim"));
            checks.push(check_rust_target("x86_64-apple-ios"));
        }
        SdkTarget::Both => {
            println!("Checking prerequisites for Android and iOS...\n");
            extend_android_prereq_checks(&mut checks);
            checks.push(check_xcode());
            checks.push(check_xcodegen());
            checks.push(check_rust_target("aarch64-apple-ios"));
            checks.push(check_rust_target("aarch64-apple-ios-sim"));
            checks.push(check_rust_target("x86_64-apple-ios"));
        }
    }

    checks
}

pub(crate) const DEFAULT_ANDROID_DOCTOR_RUST_TARGETS: &[&str] = &["aarch64-linux-android"];
pub(crate) const WORKSPACE_MSRV: &str = "1.85";

fn extend_android_prereq_checks(checks: &mut Vec<PrereqCheck>) {
    checks.push(check_android_ndk_home());
    checks.push(check_cargo_ndk());
    for target in DEFAULT_ANDROID_DOCTOR_RUST_TARGETS {
        checks.push(check_rust_target(target));
    }
    checks.push(check_jdk());
}

pub(crate) fn collect_issues(checks: &[PrereqCheck]) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    for check in checks {
        if !check.passed
            && let Some(ref fix) = check.fix_hint
        {
            issues.push(ValidationIssue {
                category: issue_category_for_check(check),
                check: check.name.clone(),
                detail: check.detail.clone(),
                fix_hint: fix.clone(),
            });
        }
    }
    issues
}

fn issue_category_for_check(check: &PrereqCheck) -> ContractErrorCategory {
    match check.name.as_str() {
        "Run config" | "Device matrix" => ContractErrorCategory::Config,
        "BrowserStack credentials" => ContractErrorCategory::Provider,
        _ => ContractErrorCategory::Preflight,
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PrereqCheck {
    pub(crate) name: String,
    pub(crate) passed: bool,
    pub(crate) detail: Option<String>,
    pub(crate) fix_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ValidationIssue {
    pub(crate) category: ContractErrorCategory,
    pub(crate) check: String,
    pub(crate) detail: Option<String>,
    pub(crate) fix_hint: String,
}

pub(crate) fn print_check_results_text(checks: &[PrereqCheck], issues: &[ValidationIssue]) {
    for check in checks {
        let status = if check.passed { "\u{2713}" } else { "\u{2717}" };
        let detail = check.detail.as_deref().unwrap_or("");
        let category = if check.passed {
            None
        } else {
            Some(issue_category_for_check(check))
        };
        if detail.is_empty() {
            if let Some(category) = category {
                println!("{} {} [{}]", status, check.name, category_slug(category));
            } else {
                println!("{} {}", status, check.name);
            }
        } else {
            if let Some(category) = category {
                println!(
                    "{} {} [{}] ({})",
                    status,
                    check.name,
                    category_slug(category),
                    detail
                );
            } else {
                println!("{} {} ({})", status, check.name, detail);
            }
        }
    }

    if !issues.is_empty() {
        println!("\nTo fix:");
        for issue in issues {
            println!("  * [{}] {}", category_slug(issue.category), issue.fix_hint);
        }
        println!();
        let failed_count = checks.iter().filter(|c| !c.passed).count();
        println!("{} issue(s) found.", failed_count);
    } else {
        println!("\nAll prerequisites satisfied!");
    }
}

pub(crate) fn print_check_results_json(
    checks: &[PrereqCheck],
    issues: &[ValidationIssue],
) -> Result<()> {
    let output = render_check_results_json(checks, issues);
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub(crate) fn render_check_results_json(
    checks: &[PrereqCheck],
    issues: &[ValidationIssue],
) -> Value {
    json!({
        "checks": checks,
        "issues": issues,
        "all_passed": checks.iter().all(|c| c.passed),
        "passed_count": checks.iter().filter(|c| c.passed).count(),
        "failed_count": checks.iter().filter(|c| !c.passed).count(),
    })
}

pub(crate) fn category_slug(category: ContractErrorCategory) -> &'static str {
    match category {
        ContractErrorCategory::Config => "config_error",
        ContractErrorCategory::Preflight => "preflight_error",
        ContractErrorCategory::Provider => "provider_error",
        ContractErrorCategory::Build => "build_error",
        ContractErrorCategory::Benchmark => "benchmark_error",
    }
}

fn check_cargo() -> PrereqCheck {
    let result = probe_command("cargo").arg("--version").output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            PrereqCheck {
                name: "cargo installed".to_string(),
                passed: true,
                detail: Some(version),
                fix_hint: None,
            }
        }
        _ => PrereqCheck {
            name: "cargo installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some("Install Rust: https://rustup.rs".to_string()),
        },
    }
}

fn check_rustup() -> PrereqCheck {
    let result = probe_command("rustup").arg("--version").output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            PrereqCheck {
                name: "rustup installed".to_string(),
                passed: true,
                detail: Some(version),
                fix_hint: None,
            }
        }
        _ => PrereqCheck {
            name: "rustup installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some("Install rustup: https://rustup.rs".to_string()),
        },
    }
}

fn check_rustc_msrv() -> PrereqCheck {
    match command_version_line("rustc", &["--version"]) {
        Some(version) if rustc_version_meets_msrv(&version, WORKSPACE_MSRV) => PrereqCheck {
            name: "rustc MSRV".to_string(),
            passed: true,
            detail: Some(format!("{version} (requires >= {WORKSPACE_MSRV})")),
            fix_hint: None,
        },
        Some(version) => PrereqCheck {
            name: "rustc MSRV".to_string(),
            passed: false,
            detail: Some(format!("{version} (requires >= {WORKSPACE_MSRV})")),
            fix_hint: Some(format!(
                "Update Rust: rustup update stable (MSRV {WORKSPACE_MSRV})"
            )),
        },
        None => PrereqCheck {
            name: "rustc MSRV".to_string(),
            passed: false,
            detail: Some("could not run rustc --version".to_string()),
            fix_hint: Some("Install Rust: https://rustup.rs".to_string()),
        },
    }
}

pub(crate) fn rustc_version_meets_msrv(version_line: &str, msrv: &str) -> bool {
    let Some(actual) = parse_rust_version(version_line) else {
        return false;
    };
    let Some(required) = parse_rust_version(msrv) else {
        return false;
    };
    actual >= required
}

pub(crate) fn parse_rust_version(input: &str) -> Option<(u32, u32, u32)> {
    let version = input
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|ch| ch.is_ascii_digit()))?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()
        .and_then(|part| part.split('-').next())
        .unwrap_or("0")
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

fn check_android_ndk_home() -> PrereqCheck {
    match env::var("ANDROID_NDK_HOME") {
        Ok(path) if !path.is_empty() => {
            let path_exists = Path::new(&path).exists();
            if path_exists {
                PrereqCheck {
                    name: "ANDROID_NDK_HOME set".to_string(),
                    passed: true,
                    detail: Some(path),
                    fix_hint: None,
                }
            } else {
                PrereqCheck {
                    name: "ANDROID_NDK_HOME set".to_string(),
                    passed: false,
                    detail: Some(format!("path does not exist: {}", path)),
                    fix_hint: Some("Set ANDROID_NDK_HOME to a valid NDK path: export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/<version>".to_string()),
                }
            }
        }
        _ => PrereqCheck {
            name: "ANDROID_NDK_HOME set".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some(
                "Set ANDROID_NDK_HOME: export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/<version>"
                    .to_string(),
            ),
        },
    }
}

fn check_cargo_ndk() -> PrereqCheck {
    let result = probe_command("cargo").args(["ndk", "--version"]).output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            PrereqCheck {
                name: "cargo-ndk installed".to_string(),
                passed: true,
                detail: Some(version),
                fix_hint: None,
            }
        }
        _ => PrereqCheck {
            name: "cargo-ndk installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some("Install cargo-ndk: cargo install cargo-ndk".to_string()),
        },
    }
}

fn check_rust_target(target: &str) -> PrereqCheck {
    let result = probe_command("rustup")
        .args(["target", "list", "--installed"])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let installed = String::from_utf8_lossy(&output.stdout);
            let has_target = installed.lines().any(|line| line.trim() == target);
            if has_target {
                PrereqCheck {
                    name: format!("Rust target: {}", target),
                    passed: true,
                    detail: None,
                    fix_hint: None,
                }
            } else {
                PrereqCheck {
                    name: format!("Rust target: {}", target),
                    passed: false,
                    detail: Some("not installed".to_string()),
                    fix_hint: Some(format!("Install target: rustup target add {}", target)),
                }
            }
        }
        _ => PrereqCheck {
            name: format!("Rust target: {}", target),
            passed: false,
            detail: Some("could not check".to_string()),
            fix_hint: Some(format!("Install target: rustup target add {}", target)),
        },
    }
}

fn check_jdk() -> PrereqCheck {
    // Try java -version
    let result = probe_command("java").arg("-version").output();

    match result {
        Ok(output) => {
            // Java outputs version to stderr
            let version_output = String::from_utf8_lossy(&output.stderr);
            let version_line = version_output.lines().next().unwrap_or("");

            if output.status.success() || !version_line.is_empty() {
                PrereqCheck {
                    name: "JDK installed".to_string(),
                    passed: true,
                    detail: Some(version_line.trim().to_string()),
                    fix_hint: None,
                }
            } else {
                PrereqCheck {
                    name: "JDK installed".to_string(),
                    passed: false,
                    detail: None,
                    fix_hint: Some("Install JDK 17+: brew install openjdk@17".to_string()),
                }
            }
        }
        Err(_) => PrereqCheck {
            name: "JDK installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some("Install JDK 17+: brew install openjdk@17".to_string()),
        },
    }
}

fn check_xcode() -> PrereqCheck {
    let result = probe_command("xcodebuild").arg("-version").output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            PrereqCheck {
                name: "Xcode installed".to_string(),
                passed: true,
                detail: Some(version),
                fix_hint: None,
            }
        }
        _ => PrereqCheck {
            name: "Xcode installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some(
                "Install Xcode from the App Store or run: xcode-select --install".to_string(),
            ),
        },
    }
}

fn check_xcodegen() -> PrereqCheck {
    let result = probe_command("xcodegen").arg("--version").output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            PrereqCheck {
                name: "xcodegen installed".to_string(),
                passed: true,
                detail: Some(version),
                fix_hint: None,
            }
        }
        _ => PrereqCheck {
            name: "xcodegen installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some("Install xcodegen: brew install xcodegen".to_string()),
        },
    }
}

fn probe_command(name: &'static str) -> ToolCommand {
    let mut command = ToolCommand::path_search(name);
    command.timeout(Duration::from_secs(30));
    command
}
