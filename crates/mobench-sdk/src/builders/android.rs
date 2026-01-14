//! Android build automation
//!
//! This module provides functionality to build Rust libraries for Android and
//! package them into an APK using Gradle.

use crate::types::{BenchError, BuildConfig, BuildProfile, BuildResult, Target};
use std::env;
use std::path::PathBuf;
use std::process::Command;

/// Android builder that handles the complete build pipeline
pub struct AndroidBuilder {
    /// Root directory of the project
    project_root: PathBuf,
    /// Name of the bench-mobile crate
    crate_name: String,
    /// Whether to use verbose output
    verbose: bool,
}

impl AndroidBuilder {
    /// Creates a new Android builder
    ///
    /// # Arguments
    ///
    /// * `project_root` - Root directory containing the bench-mobile crate
    /// * `crate_name` - Name of the bench-mobile crate (e.g., "my-project-bench-mobile")
    pub fn new(project_root: impl Into<PathBuf>, crate_name: impl Into<String>) -> Self {
        Self {
            project_root: project_root.into(),
            crate_name: crate_name.into(),
            verbose: false,
        }
    }

    /// Enables verbose output
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Builds the Android app with the given configuration
    ///
    /// This performs the following steps:
    /// 1. Build Rust libraries for Android ABIs using cargo-ndk
    /// 2. Generate UniFFI Kotlin bindings
    /// 3. Copy .so files to jniLibs directories
    /// 4. Run Gradle to build the APK
    ///
    /// # Returns
    ///
    /// * `Ok(BuildResult)` containing the path to the built APK
    /// * `Err(BenchError)` if the build fails
    pub fn build(&self, config: &BuildConfig) -> Result<BuildResult, BenchError> {
        // Step 1: Build Rust libraries
        println!("Building Rust libraries for Android...");
        self.build_rust_libraries(config)?;

        // Step 2: Generate UniFFI bindings
        println!("Generating UniFFI Kotlin bindings...");
        self.generate_uniffi_bindings()?;

        // Step 3: Copy .so files to jniLibs
        println!("Copying native libraries to jniLibs...");
        self.copy_native_libraries(config)?;

        // Step 4: Build APK with Gradle
        println!("Building Android APK with Gradle...");
        let apk_path = self.build_apk(config)?;

        Ok(BuildResult {
            platform: Target::Android,
            app_path: apk_path,
            test_suite_path: None, // TODO: Add support for building test APK
        })
    }

    /// Finds the benchmark crate directory (either bench-mobile/ or crates/{crate_name}/)
    fn find_crate_dir(&self) -> Result<PathBuf, BenchError> {
        // Try bench-mobile/ first (SDK projects)
        let bench_mobile_dir = self.project_root.join("bench-mobile");
        if bench_mobile_dir.exists() {
            return Ok(bench_mobile_dir);
        }

        // Try crates/{crate_name}/ (repository structure)
        let crates_dir = self.project_root.join("crates").join(&self.crate_name);
        if crates_dir.exists() {
            return Ok(crates_dir);
        }

        Err(BenchError::Build(format!(
            "Benchmark crate '{}' not found. Tried:\n  - {:?}\n  - {:?}",
            self.crate_name, bench_mobile_dir, crates_dir
        )))
    }

    /// Builds Rust libraries for Android using cargo-ndk
    fn build_rust_libraries(&self, config: &BuildConfig) -> Result<(), BenchError> {
        let crate_dir = self.find_crate_dir()?;

        // Check if cargo-ndk is installed
        self.check_cargo_ndk()?;

        // Android ABIs to build for
        let abis = vec!["arm64-v8a", "armeabi-v7a", "x86_64"];

        for abi in abis {
            if self.verbose {
                println!("  Building for {}", abi);
            }

            let mut cmd = Command::new("cargo");
            cmd.arg("ndk")
                .arg("--target")
                .arg(abi)
                .arg("--platform")
                .arg("24") // minSdk
                .arg("build");

            // Add release flag if needed
            if matches!(config.profile, BuildProfile::Release) {
                cmd.arg("--release");
            }

            // Set working directory
            cmd.current_dir(&crate_dir);

            // Execute build
            let output = cmd
                .output()
                .map_err(|e| BenchError::Build(format!("Failed to run cargo-ndk: {}", e)))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(BenchError::Build(format!(
                    "cargo-ndk build failed for {}: {}",
                    abi, stderr
                )));
            }
        }

        Ok(())
    }

    /// Checks if cargo-ndk is installed
    fn check_cargo_ndk(&self) -> Result<(), BenchError> {
        let output = Command::new("cargo").arg("ndk").arg("--version").output();

        match output {
            Ok(output) if output.status.success() => Ok(()),
            _ => Err(BenchError::Build(
                "cargo-ndk is not installed. Install it with: cargo install cargo-ndk".to_string(),
            )),
        }
    }

    /// Generates UniFFI Kotlin bindings
    fn generate_uniffi_bindings(&self) -> Result<(), BenchError> {
        let crate_dir = self.find_crate_dir()?;
        let crate_name_underscored = self.crate_name.replace("-", "_");

        // Check if bindings already exist (for repository testing with pre-generated bindings)
        let bindings_path = self
            .project_root
            .join("android")
            .join("app")
            .join("src")
            .join("main")
            .join("java")
            .join("uniffi")
            .join(&crate_name_underscored)
            .join(format!("{}.kt", crate_name_underscored));

        if bindings_path.exists() {
            if self.verbose {
                println!("  Using existing Kotlin bindings at {:?}", bindings_path);
            }
            return Ok(());
        }

        // Check if uniffi-bindgen is available
        let uniffi_available = Command::new("uniffi-bindgen")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !uniffi_available {
            return Err(BenchError::Build(
                "uniffi-bindgen not found and no pre-generated bindings exist.\n\
                 Install it with: cargo install uniffi-bindgen\n\
                 Or use pre-generated bindings by copying them to the expected location."
                    .to_string(),
            ));
        }

        // Build host library to feed uniffi-bindgen
        let mut build_cmd = Command::new("cargo");
        build_cmd.arg("build");
        build_cmd.current_dir(&crate_dir);
        run_command(build_cmd, "cargo build (host)")?;

        let lib_path = host_lib_path(&crate_dir, &self.crate_name)?;
        let out_dir = self
            .project_root
            .join("android")
            .join("app")
            .join("src")
            .join("main")
            .join("java");

        let mut cmd = Command::new("uniffi-bindgen");
        cmd.arg("generate")
            .arg("--library")
            .arg(&lib_path)
            .arg("--language")
            .arg("kotlin")
            .arg("--out-dir")
            .arg(&out_dir);
        run_command(cmd, "uniffi-bindgen kotlin")?;

        if self.verbose {
            println!("  Generated UniFFI Kotlin bindings at {:?}", out_dir);
        }
        Ok(())
    }

    /// Copies .so files to Android jniLibs directories
    fn copy_native_libraries(&self, config: &BuildConfig) -> Result<(), BenchError> {
        let profile_dir = match config.profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        };

        let target_dir = self.project_root.join("target");
        let jni_libs_dir = self.project_root.join("android/app/src/main/jniLibs");

        // Create jniLibs directories if they don't exist
        std::fs::create_dir_all(&jni_libs_dir)
            .map_err(|e| BenchError::Build(format!("Failed to create jniLibs directory: {}", e)))?;

        // Map cargo-ndk ABIs to Android jniLibs ABIs
        let abi_mappings = vec![
            ("aarch64-linux-android", "arm64-v8a"),
            ("armv7-linux-androideabi", "armeabi-v7a"),
            ("x86_64-linux-android", "x86_64"),
        ];

        for (rust_target, android_abi) in abi_mappings {
            let src = target_dir
                .join(rust_target)
                .join(profile_dir)
                .join(format!("lib{}.so", self.crate_name.replace("-", "_")));

            let dest_dir = jni_libs_dir.join(android_abi);
            std::fs::create_dir_all(&dest_dir).map_err(|e| {
                BenchError::Build(format!("Failed to create {} directory: {}", android_abi, e))
            })?;

            let dest = dest_dir.join(format!(
                "lib{}.so",
                self.crate_name.replace("-", "_")
            ));

            if src.exists() {
                std::fs::copy(&src, &dest).map_err(|e| {
                    BenchError::Build(format!("Failed to copy {} library: {}", android_abi, e))
                })?;

                if self.verbose {
                    println!("  Copied {} -> {}", src.display(), dest.display());
                }
            } else if self.verbose {
                println!("  Warning: {} not found, skipping", src.display());
            }
        }

        Ok(())
    }

    /// Builds the Android APK using Gradle
    fn build_apk(&self, config: &BuildConfig) -> Result<PathBuf, BenchError> {
        let android_dir = self.project_root.join("android");

        if !android_dir.exists() {
            return Err(BenchError::Build(format!(
                "Android project not found at {:?}",
                android_dir
            )));
        }

        // Determine Gradle task
        let gradle_task = match config.profile {
            BuildProfile::Debug => "assembleDebug",
            BuildProfile::Release => "assembleRelease",
        };

        // Run Gradle build
        let mut cmd = Command::new("./gradlew");
        cmd.arg(gradle_task).current_dir(&android_dir);

        if self.verbose {
            cmd.arg("--info");
        }

        let output = cmd
            .output()
            .map_err(|e| BenchError::Build(format!("Failed to run Gradle: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BenchError::Build(format!(
                "Gradle build failed: {}",
                stderr
            )));
        }

        // Determine APK path
        let profile_name = match config.profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        };

        let apk_path = android_dir
            .join("app/build/outputs/apk")
            .join(profile_name)
            .join(format!("app-{}.apk", profile_name));

        if !apk_path.exists() {
            return Err(BenchError::Build(format!(
                "APK not found at expected location: {:?}",
                apk_path
            )));
        }

        Ok(apk_path)
    }
}

// Shared helpers
fn host_lib_path(project_dir: &PathBuf, crate_name: &str) -> Result<PathBuf, BenchError> {
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
                "unsupported host OS for binding generation: {}",
                other
            )));
        }
    };
    let path = project_dir.join("target").join("debug").join(format!(
        "{}{}.{}",
        lib_prefix,
        crate_name.replace('-', "_"),
        lib_ext
    ));
    if !path.exists() {
        return Err(BenchError::Build(format!(
            "host library for UniFFI not found at {:?}",
            path
        )));
    }
    Ok(path)
}

fn run_command(mut cmd: Command, description: &str) -> Result<(), BenchError> {
    let output = cmd
        .output()
        .map_err(|e| BenchError::Build(format!("Failed to run {}: {}", description, e)))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BenchError::Build(format!(
            "{} failed: {}",
            description, stderr
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_android_builder_creation() {
        let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile");
        assert!(!builder.verbose);
    }

    #[test]
    fn test_android_builder_verbose() {
        let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile").verbose(true);
        assert!(builder.verbose);
    }
}
