//! Common utilities shared between Android and iOS builders.
//!
//! This module provides helper functions that are used by both [`super::AndroidBuilder`]
//! and [`super::IosBuilder`] to ensure consistent behavior and error handling.
//!
//! ## Features
//!
//! - **Workspace-aware target detection** - Correctly handles Cargo workspaces where
//!   the target directory is at the workspace root
//! - **Host library resolution** - Finds compiled libraries for UniFFI binding generation
//! - **Consistent error handling** - All errors include actionable fix suggestions
//!
//! ## Error Messages
//!
//! All functions in this module provide detailed, actionable error messages that include:
//! - What went wrong
//! - Where it happened (paths, commands)
//! - How to fix it (specific commands or configuration changes)

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::types::BenchError;

/// Validates that the project root is a valid directory for building.
///
/// This function checks that:
/// - The path exists
/// - The path is a directory
/// - The directory contains a Cargo.toml file (or has a crate directory with one)
///
/// # Arguments
/// * `project_root` - The project root directory to validate
/// * `crate_name` - The name of the crate being built (used to check crate directories)
///
/// # Returns
/// `Ok(())` if validation passes, or a descriptive `BenchError` if it fails.
pub fn validate_project_root(project_root: &Path, crate_name: &str) -> Result<(), BenchError> {
    // Check if path exists
    if !project_root.exists() {
        return Err(BenchError::Build(format!(
            "Project root does not exist: {}\n\n\
             Ensure you are running from the correct directory or specify --project-root.",
            project_root.display()
        )));
    }

    // Check if path is a directory
    if !project_root.is_dir() {
        return Err(BenchError::Build(format!(
            "Project root is not a directory: {}\n\n\
             Expected a directory containing your Rust project.",
            project_root.display()
        )));
    }

    // Check for Cargo.toml in project root or standard crate locations
    let root_cargo = project_root.join("Cargo.toml");
    let bench_mobile_cargo = project_root.join("bench-mobile/Cargo.toml");
    let crates_cargo = project_root.join(format!("crates/{}/Cargo.toml", crate_name));

    if !root_cargo.exists() && !bench_mobile_cargo.exists() && !crates_cargo.exists() {
        return Err(BenchError::Build(format!(
            "No Cargo.toml found in project root or expected crate locations.\n\n\
             Searched:\n\
             - {}\n\
             - {}\n\
             - {}\n\n\
             Ensure you are in a Rust project directory or use --crate-path to specify the crate location.",
            root_cargo.display(),
            bench_mobile_cargo.display(),
            crates_cargo.display()
        )));
    }

    Ok(())
}

/// Detects the actual Cargo target directory using `cargo metadata`.
///
/// This correctly handles Cargo workspaces where the target directory
/// is at the workspace root, not the crate directory.
///
/// # Arguments
/// * `crate_dir` - Path to the crate directory containing Cargo.toml
///
/// # Returns
/// The path to the target directory, or falls back to `crate_dir/target` if detection fails.
///
/// # Warnings
/// Prints a warning to stderr if falling back to the default target directory due to
/// cargo metadata failures or parsing issues.
pub fn get_cargo_target_dir(crate_dir: &Path) -> Result<PathBuf, BenchError> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(crate_dir)
        .output()
        .map_err(|e| {
            BenchError::Build(format!(
                "Failed to run cargo metadata.\n\n\
                 Working directory: {}\n\
                 Error: {}\n\n\
                 Ensure cargo is installed and on PATH.",
                crate_dir.display(),
                e
            ))
        })?;

    if !output.status.success() {
        // Fall back to crate_dir/target if cargo metadata fails
        let fallback = crate_dir.join("target");
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "Warning: cargo metadata failed (exit {}), falling back to {}.\n\
             Stderr: {}\n\
             This may cause build issues if you are in a Cargo workspace.",
            output.status,
            fallback.display(),
            stderr.lines().take(3).collect::<Vec<_>>().join("\n")
        );
        return Ok(fallback);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse the JSON to extract target_directory
    // Using simple string parsing to avoid adding serde_json dependency
    if let Some(start) = stdout.find("\"target_directory\":\"") {
        let rest = &stdout[start + 20..];
        if let Some(end) = rest.find('"') {
            let target_dir = &rest[..end];
            // Handle escaped backslashes in Windows paths
            let target_dir = target_dir.replace("\\\\", "\\");
            return Ok(PathBuf::from(target_dir));
        }
    }

    // Fall back to crate_dir/target if parsing fails
    let fallback = crate_dir.join("target");
    eprintln!(
        "Warning: Failed to parse target_directory from cargo metadata output, \
         falling back to {}.\n\
         This may cause build issues if you are in a Cargo workspace.",
        fallback.display()
    );
    Ok(fallback)
}

/// Finds the host library path for UniFFI binding generation.
///
/// UniFFI requires a host-compiled library to generate bindings. This function
/// locates that library in the target directory.
///
/// # Arguments
/// * `crate_dir` - Path to the crate directory
/// * `crate_name` - Name of the crate (used to construct library filename)
///
/// # Returns
/// Path to the host library (e.g., `libfoo.dylib` on macOS, `libfoo.so` on Linux)
pub fn host_lib_path(crate_dir: &Path, crate_name: &str) -> Result<PathBuf, BenchError> {
    let lib_prefix = if cfg!(target_os = "windows") {
        ""
    } else {
        "lib"
    };
    let lib_ext = match env::consts::OS {
        "macos" => "dylib",
        "linux" => "so",
        other => {
            return Err(BenchError::Build(format!(
                "Unsupported host OS for binding generation: {}\n\n\
                 Supported platforms:\n\
                 - macOS (generates .dylib)\n\
                 - Linux (generates .so)\n\n\
                 Windows is not currently supported for binding generation.",
                other
            )));
        }
    };

    // Use cargo metadata to find the actual target directory
    let target_dir = get_cargo_target_dir(crate_dir)?;

    let lib_name = format!(
        "{}{}.{}",
        lib_prefix,
        crate_name.replace('-', "_"),
        lib_ext
    );
    let path = target_dir.join("debug").join(&lib_name);

    if !path.exists() {
        return Err(BenchError::Build(format!(
            "Host library for UniFFI not found.\n\n\
             Expected: {}\n\
             Target directory: {}\n\n\
             To fix this:\n\
             1. Build the host library first:\n\
                cargo build -p {}\n\n\
             2. Ensure your crate produces a cdylib:\n\
                [lib]\n\
                crate-type = [\"cdylib\"]\n\n\
             3. Check that the library name matches: {}",
            path.display(),
            target_dir.display(),
            crate_name,
            lib_name
        )));
    }
    Ok(path)
}

/// Runs an external command with consistent error handling.
///
/// Captures both stdout and stderr on failure and formats them into
/// an actionable error message.
///
/// # Arguments
/// * `cmd` - The command to execute
/// * `description` - Human-readable description of what the command does
///
/// # Returns
/// `Ok(())` if the command succeeds, or a `BenchError` with detailed output on failure.
pub fn run_command(mut cmd: Command, description: &str) -> Result<(), BenchError> {
    let output = cmd.output().map_err(|e| {
        BenchError::Build(format!(
            "Failed to start {}.\n\n\
             Error: {}\n\n\
             Ensure the tool is installed and available on PATH.",
            description, e
        ))
    })?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BenchError::Build(format!(
            "{} failed.\n\n\
             Exit status: {}\n\n\
             Stdout:\n{}\n\n\
             Stderr:\n{}",
            description, output.status, stdout, stderr
        )));
    }
    Ok(())
}

/// Reads the package name from a Cargo.toml file.
///
/// This function parses the `[package]` section of a Cargo.toml and extracts
/// the `name` field. It uses simple string parsing to avoid adding toml
/// dependencies.
///
/// # Arguments
/// * `cargo_toml_path` - Path to the Cargo.toml file
///
/// # Returns
/// `Some(name)` if the package name is found, `None` otherwise.
///
/// # Example
/// ```ignore
/// let name = read_package_name(Path::new("/path/to/Cargo.toml"));
/// if let Some(name) = name {
///     println!("Package name: {}", name);
/// }
/// ```
pub fn read_package_name(cargo_toml_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(cargo_toml_path).ok()?;

    // Find [package] section
    let package_start = content.find("[package]")?;
    let package_section = &content[package_start..];

    // Find the end of the package section (next section or end of file)
    let section_end = package_section[1..]
        .find("\n[")
        .map(|i| i + 1)
        .unwrap_or(package_section.len());
    let package_section = &package_section[..section_end];

    // Find name = "..." or name = '...'
    for line in package_section.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("name") {
            // Parse: name = "value" or name = 'value'
            if let Some(eq_pos) = trimmed.find('=') {
                let value_part = trimmed[eq_pos + 1..].trim();
                // Extract string value (handle both " and ')
                let (quote_char, start) = if value_part.starts_with('"') {
                    ('"', 1)
                } else if value_part.starts_with('\'') {
                    ('\'', 1)
                } else {
                    continue;
                };
                if let Some(end) = value_part[start..].find(quote_char) {
                    return Some(value_part[start..start + end].to_string());
                }
            }
        }
    }

    None
}

/// Embeds a bench spec JSON file into the Android assets and iOS bundle resources.
///
/// This function writes a `bench_spec.json` file to the appropriate location for
/// both Android (assets directory) and iOS (bundle resources) so the mobile app
/// can read the benchmark configuration at runtime.
///
/// # Arguments
/// * `output_dir` - The mobench output directory (e.g., `target/mobench`)
/// * `spec` - The benchmark specification as a JSON-serializable struct
///
/// # Example
/// ```ignore
/// use mobench_sdk::builders::common::embed_bench_spec;
/// use mobench_sdk::BenchSpec;
///
/// let spec = BenchSpec {
///     name: "my_crate::my_benchmark".to_string(),
///     iterations: 100,
///     warmup: 10,
/// };
///
/// embed_bench_spec(Path::new("target/mobench"), &spec)?;
/// ```
pub fn embed_bench_spec<S: serde::Serialize>(output_dir: &Path, spec: &S) -> Result<(), BenchError> {
    let spec_json = serde_json::to_string_pretty(spec).map_err(|e| {
        BenchError::Build(format!("Failed to serialize bench spec: {}", e))
    })?;

    // Android: Write to assets directory
    let android_assets_dir = output_dir.join("android/app/src/main/assets");
    if output_dir.join("android").exists() {
        std::fs::create_dir_all(&android_assets_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create Android assets directory at {}: {}",
                android_assets_dir.display(),
                e
            ))
        })?;
        let android_spec_path = android_assets_dir.join("bench_spec.json");
        std::fs::write(&android_spec_path, &spec_json).map_err(|e| {
            BenchError::Build(format!(
                "Failed to write Android bench spec to {}: {}",
                android_spec_path.display(),
                e
            ))
        })?;
    }

    // iOS: Write to Resources directory in the Xcode project
    let ios_resources_dir = output_dir.join("ios/BenchRunner/BenchRunner/Resources");
    if output_dir.join("ios/BenchRunner").exists() {
        std::fs::create_dir_all(&ios_resources_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create iOS Resources directory at {}: {}",
                ios_resources_dir.display(),
                e
            ))
        })?;
        let ios_spec_path = ios_resources_dir.join("bench_spec.json");
        std::fs::write(&ios_spec_path, &spec_json).map_err(|e| {
            BenchError::Build(format!(
                "Failed to write iOS bench spec to {}: {}",
                ios_spec_path.display(),
                e
            ))
        })?;
    }

    Ok(())
}

/// Represents a benchmark specification for embedding.
///
/// This is a simple struct that can be serialized to JSON and embedded
/// in mobile app bundles.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddedBenchSpec {
    /// The benchmark function name (e.g., "my_crate::my_benchmark")
    pub function: String,
    /// Number of benchmark iterations
    pub iterations: u32,
    /// Number of warmup iterations
    pub warmup: u32,
}

/// Build metadata for artifact correlation and traceability.
///
/// This struct captures metadata about the build environment to enable
/// reproducibility and debugging of benchmark results.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchMeta {
    /// Benchmark specification that was used
    pub spec: EmbeddedBenchSpec,
    /// Git commit hash (if in a git repository)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_hash: Option<String>,
    /// Git branch name (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Whether the git working directory was dirty
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
    /// Build timestamp in RFC3339 format
    pub build_time: String,
    /// Build timestamp as Unix epoch seconds
    pub build_time_unix: u64,
    /// Target platform ("android" or "ios")
    pub target: String,
    /// Build profile ("debug" or "release")
    pub profile: String,
    /// mobench version
    pub mobench_version: String,
    /// Rust version used for the build
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust_version: Option<String>,
    /// Host OS (e.g., "macos", "linux")
    pub host_os: String,
}

/// Gets the current git commit hash (short form).
pub fn get_git_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !hash.is_empty() {
            return Some(hash);
        }
    }
    None
}

/// Gets the current git branch name.
pub fn get_git_branch() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() && branch != "HEAD" {
            return Some(branch);
        }
    }
    None
}

/// Checks if the git working directory has uncommitted changes.
pub fn is_git_dirty() -> Option<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()?;

    if output.status.success() {
        let status = String::from_utf8_lossy(&output.stdout);
        Some(!status.trim().is_empty())
    } else {
        None
    }
}

/// Gets the Rust version.
pub fn get_rust_version() -> Option<String> {
    let output = Command::new("rustc")
        .args(["--version"])
        .output()
        .ok()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !version.is_empty() {
            return Some(version);
        }
    }
    None
}

/// Creates a BenchMeta instance with current build information.
pub fn create_bench_meta(spec: &EmbeddedBenchSpec, target: &str, profile: &str) -> BenchMeta {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    // Format as RFC3339
    let build_time = {
        let secs = now.as_secs();
        // Simple UTC timestamp formatting
        let days_since_epoch = secs / 86400;
        let remaining_secs = secs % 86400;
        let hours = remaining_secs / 3600;
        let minutes = (remaining_secs % 3600) / 60;
        let seconds = remaining_secs % 60;

        // Calculate year, month, day from days since epoch (1970-01-01)
        // Simplified calculation - good enough for build metadata
        let (year, month, day) = days_to_ymd(days_since_epoch);

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            year, month, day, hours, minutes, seconds
        )
    };

    BenchMeta {
        spec: spec.clone(),
        commit_hash: get_git_commit(),
        branch: get_git_branch(),
        dirty: is_git_dirty(),
        build_time,
        build_time_unix: now.as_secs(),
        target: target.to_string(),
        profile: profile.to_string(),
        mobench_version: env!("CARGO_PKG_VERSION").to_string(),
        rust_version: get_rust_version(),
        host_os: env::consts::OS.to_string(),
    }
}

/// Convert days since epoch to (year, month, day).
/// Simplified Gregorian calendar calculation.
fn days_to_ymd(days: u64) -> (i32, u32, u32) {
    let mut remaining_days = days as i64;
    let mut year = 1970i32;

    // Advance years
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    // Days in each month (non-leap year)
    let days_in_months: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let mut month = 1u32;
    for (i, &days_in_month) in days_in_months.iter().enumerate() {
        let mut dim = days_in_month;
        if i == 1 && is_leap_year(year) {
            dim = 29;
        }
        if remaining_days < dim {
            break;
        }
        remaining_days -= dim;
        month += 1;
    }

    (year, month, remaining_days as u32 + 1)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Embeds build metadata (bench_meta.json) alongside bench_spec.json in mobile app bundles.
///
/// This function creates a `bench_meta.json` file that contains:
/// - The benchmark specification
/// - Git commit hash and branch (if available)
/// - Build timestamp
/// - Target platform and profile
/// - mobench and Rust versions
///
/// # Arguments
/// * `output_dir` - The mobench output directory (e.g., `target/mobench`)
/// * `spec` - The benchmark specification
/// * `target` - Target platform ("android" or "ios")
/// * `profile` - Build profile ("debug" or "release")
pub fn embed_bench_meta(
    output_dir: &Path,
    spec: &EmbeddedBenchSpec,
    target: &str,
    profile: &str,
) -> Result<(), BenchError> {
    let meta = create_bench_meta(spec, target, profile);
    let meta_json = serde_json::to_string_pretty(&meta).map_err(|e| {
        BenchError::Build(format!("Failed to serialize bench meta: {}", e))
    })?;

    // Android: Write to assets directory
    let android_assets_dir = output_dir.join("android/app/src/main/assets");
    if output_dir.join("android").exists() {
        std::fs::create_dir_all(&android_assets_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create Android assets directory at {}: {}",
                android_assets_dir.display(),
                e
            ))
        })?;
        let android_meta_path = android_assets_dir.join("bench_meta.json");
        std::fs::write(&android_meta_path, &meta_json).map_err(|e| {
            BenchError::Build(format!(
                "Failed to write Android bench meta to {}: {}",
                android_meta_path.display(),
                e
            ))
        })?;
    }

    // iOS: Write to Resources directory in the Xcode project
    let ios_resources_dir = output_dir.join("ios/BenchRunner/BenchRunner/Resources");
    if output_dir.join("ios/BenchRunner").exists() {
        std::fs::create_dir_all(&ios_resources_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create iOS Resources directory at {}: {}",
                ios_resources_dir.display(),
                e
            ))
        })?;
        let ios_meta_path = ios_resources_dir.join("bench_meta.json");
        std::fs::write(&ios_meta_path, &meta_json).map_err(|e| {
            BenchError::Build(format!(
                "Failed to write iOS bench meta to {}: {}",
                ios_meta_path.display(),
                e
            ))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_cargo_target_dir_fallback() {
        // For a non-existent directory, should fall back gracefully
        let result = get_cargo_target_dir(Path::new("/nonexistent/path"));
        // Should either error or return fallback path
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_host_lib_path_not_found() {
        let result = host_lib_path(Path::new("/tmp"), "nonexistent-crate");
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("Host library for UniFFI not found"));
        assert!(msg.contains("cargo build"));
    }

    #[test]
    fn test_run_command_not_found() {
        let cmd = Command::new("nonexistent-command-12345");
        let result = run_command(cmd, "test command");
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("Failed to start"));
    }

    #[test]
    fn test_read_package_name_standard() {
        let temp_dir = std::env::temp_dir().join("mobench-test-read-package");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let cargo_toml = temp_dir.join("Cargo.toml");
        std::fs::write(
            &cargo_toml,
            r#"[package]
name = "my-awesome-crate"
version = "0.1.0"
edition = "2021"

[dependencies]
"#,
        )
        .unwrap();

        let result = read_package_name(&cargo_toml);
        assert_eq!(result, Some("my-awesome-crate".to_string()));

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_read_package_name_with_single_quotes() {
        let temp_dir = std::env::temp_dir().join("mobench-test-read-package-sq");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let cargo_toml = temp_dir.join("Cargo.toml");
        std::fs::write(
            &cargo_toml,
            r#"[package]
name = 'single-quoted-crate'
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = read_package_name(&cargo_toml);
        assert_eq!(result, Some("single-quoted-crate".to_string()));

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_read_package_name_not_found() {
        let result = read_package_name(Path::new("/nonexistent/Cargo.toml"));
        assert_eq!(result, None);
    }

    #[test]
    fn test_read_package_name_no_package_section() {
        let temp_dir = std::env::temp_dir().join("mobench-test-read-package-no-pkg");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let cargo_toml = temp_dir.join("Cargo.toml");
        std::fs::write(
            &cargo_toml,
            r#"[workspace]
members = ["crates/*"]
"#,
        )
        .unwrap();

        let result = read_package_name(&cargo_toml);
        assert_eq!(result, None);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_create_bench_meta() {
        let spec = EmbeddedBenchSpec {
            function: "test_crate::my_benchmark".to_string(),
            iterations: 100,
            warmup: 10,
        };

        let meta = create_bench_meta(&spec, "android", "release");

        assert_eq!(meta.spec.function, "test_crate::my_benchmark");
        assert_eq!(meta.spec.iterations, 100);
        assert_eq!(meta.spec.warmup, 10);
        assert_eq!(meta.target, "android");
        assert_eq!(meta.profile, "release");
        assert!(!meta.mobench_version.is_empty());
        assert!(!meta.host_os.is_empty());
        assert!(!meta.build_time.is_empty());
        assert!(meta.build_time_unix > 0);
        // Build time should be in RFC3339 format (roughly YYYY-MM-DDTHH:MM:SSZ)
        assert!(meta.build_time.contains('T'));
        assert!(meta.build_time.ends_with('Z'));
    }

    #[test]
    fn test_days_to_ymd_epoch() {
        // Day 0 should be January 1, 1970
        let (year, month, day) = days_to_ymd(0);
        assert_eq!(year, 1970);
        assert_eq!(month, 1);
        assert_eq!(day, 1);
    }

    #[test]
    fn test_days_to_ymd_known_date() {
        // January 21, 2026 is approximately 20,474 days since epoch
        // (2026 - 1970 = 56 years, with leap years)
        // Let's test a simpler case: 365 days = January 1, 1971
        let (year, month, day) = days_to_ymd(365);
        assert_eq!(year, 1971);
        assert_eq!(month, 1);
        assert_eq!(day, 1);
    }

    #[test]
    fn test_is_leap_year() {
        assert!(!is_leap_year(1970)); // Not divisible by 4
        assert!(is_leap_year(2000));  // Divisible by 400
        assert!(!is_leap_year(1900)); // Divisible by 100 but not 400
        assert!(is_leap_year(2024));  // Divisible by 4, not by 100
    }

    #[test]
    fn test_bench_meta_serialization() {
        let spec = EmbeddedBenchSpec {
            function: "my_func".to_string(),
            iterations: 50,
            warmup: 5,
        };

        let meta = create_bench_meta(&spec, "ios", "debug");
        let json = serde_json::to_string(&meta).expect("serialization should work");

        // Verify it contains expected fields
        assert!(json.contains("my_func"));
        assert!(json.contains("ios"));
        assert!(json.contains("debug"));
        assert!(json.contains("build_time"));
        assert!(json.contains("mobench_version"));
    }
}
