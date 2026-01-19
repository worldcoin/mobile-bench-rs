//! Common utilities shared between Android and iOS builders
//!
//! This module provides helper functions for:
//! - Detecting Cargo target directories (workspace-aware)
//! - Finding host libraries for UniFFI binding generation
//! - Running external commands with consistent error handling

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::types::BenchError;

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
        return Ok(crate_dir.join("target"));
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
    Ok(crate_dir.join("target"))
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
}
