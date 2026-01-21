//! Android build automation.
//!
//! This module provides [`AndroidBuilder`] which handles the complete pipeline for
//! building Rust libraries for Android and packaging them into an APK using Gradle.
//!
//! ## Build Pipeline
//!
//! The builder performs these steps:
//!
//! 1. **Project scaffolding** - Auto-generates Android project if missing
//! 2. **Rust compilation** - Builds native `.so` libraries for Android ABIs using `cargo-ndk`
//! 3. **Binding generation** - Generates UniFFI Kotlin bindings
//! 4. **Library packaging** - Copies `.so` files to `jniLibs/` directories
//! 5. **APK building** - Runs Gradle to build the app APK
//! 6. **Test APK building** - Builds the androidTest APK for BrowserStack Espresso
//!
//! ## Requirements
//!
//! - Android NDK (set `ANDROID_NDK_HOME` environment variable)
//! - `cargo-ndk` (`cargo install cargo-ndk`)
//! - Rust targets: `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`
//! - Java JDK (for Gradle)
//!
//! ## Example
//!
//! ```ignore
//! use mobench_sdk::builders::AndroidBuilder;
//! use mobench_sdk::{BuildConfig, BuildProfile, Target};
//!
//! let builder = AndroidBuilder::new(".", "my-bench-crate")
//!     .verbose(true)
//!     .dry_run(false);  // Set to true to preview without building
//!
//! let config = BuildConfig {
//!     target: Target::Android,
//!     profile: BuildProfile::Release,
//!     incremental: true,
//! };
//!
//! let result = builder.build(&config)?;
//! println!("APK at: {:?}", result.app_path);
//! println!("Test APK at: {:?}", result.test_suite_path);
//! # Ok::<(), mobench_sdk::BenchError>(())
//! ```
//!
//! ## Dry-Run Mode
//!
//! Use `dry_run(true)` to preview the build plan without making changes:
//!
//! ```ignore
//! let builder = AndroidBuilder::new(".", "my-bench")
//!     .dry_run(true);
//!
//! // This will print the build plan but not execute anything
//! builder.build(&config)?;
//! ```

use crate::types::{BenchError, BuildConfig, BuildProfile, BuildResult, Target};
use super::common::{get_cargo_target_dir, host_lib_path, run_command, validate_project_root};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Android builder that handles the complete build pipeline.
///
/// This builder automates the process of compiling Rust code to Android native
/// libraries, generating UniFFI Kotlin bindings, and packaging everything into
/// an APK ready for deployment.
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::builders::AndroidBuilder;
/// use mobench_sdk::{BuildConfig, BuildProfile, Target};
///
/// let builder = AndroidBuilder::new(".", "my-bench")
///     .verbose(true)
///     .output_dir("target/mobench");
///
/// let config = BuildConfig {
///     target: Target::Android,
///     profile: BuildProfile::Release,
///     incremental: true,
/// };
///
/// let result = builder.build(&config)?;
/// # Ok::<(), mobench_sdk::BenchError>(())
/// ```
pub struct AndroidBuilder {
    /// Root directory of the project
    project_root: PathBuf,
    /// Output directory for mobile artifacts (defaults to target/mobench)
    output_dir: PathBuf,
    /// Name of the bench-mobile crate
    crate_name: String,
    /// Whether to use verbose output
    verbose: bool,
    /// Optional explicit crate directory (overrides auto-detection)
    crate_dir: Option<PathBuf>,
    /// Whether to run in dry-run mode (print what would be done without making changes)
    dry_run: bool,
}

impl AndroidBuilder {
    /// Creates a new Android builder
    ///
    /// # Arguments
    ///
    /// * `project_root` - Root directory containing the bench-mobile crate
    /// * `crate_name` - Name of the bench-mobile crate (e.g., "my-project-bench-mobile")
    pub fn new(project_root: impl Into<PathBuf>, crate_name: impl Into<String>) -> Self {
        let root = project_root.into();
        Self {
            output_dir: root.join("target/mobench"),
            project_root: root,
            crate_name: crate_name.into(),
            verbose: false,
            crate_dir: None,
            dry_run: false,
        }
    }

    /// Sets the output directory for mobile artifacts
    ///
    /// By default, artifacts are written to `{project_root}/target/mobench/`.
    /// Use this to customize the output location.
    pub fn output_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.output_dir = dir.into();
        self
    }

    /// Sets the explicit crate directory
    ///
    /// By default, the builder searches for the crate in this order:
    /// 1. `{project_root}/Cargo.toml` - if it exists and has `[package] name` matching `crate_name`
    /// 2. `{project_root}/bench-mobile/` - SDK-generated projects
    /// 3. `{project_root}/crates/{crate_name}/` - workspace structure
    /// 4. `{project_root}/{crate_name}/` - simple nested structure
    ///
    /// Use this to override auto-detection and point directly to the crate.
    pub fn crate_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.crate_dir = Some(dir.into());
        self
    }

    /// Enables verbose output
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Enables dry-run mode
    ///
    /// In dry-run mode, the builder prints what would be done without actually
    /// making any changes. Useful for previewing the build process.
    pub fn dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Builds the Android app with the given configuration
    ///
    /// This performs the following steps:
    /// 0. Auto-generate project scaffolding if missing
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
        // Validate project root before starting build
        if self.crate_dir.is_none() {
            validate_project_root(&self.project_root, &self.crate_name)?;
        }

        let android_dir = self.output_dir.join("android");
        let profile_name = match config.profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        };

        if self.dry_run {
            println!("\n[dry-run] Android build plan:");
            println!("  Step 0: Check/generate Android project scaffolding at {:?}", android_dir);
            println!("  Step 0.5: Ensure Gradle wrapper exists (run 'gradle wrapper' if needed)");
            println!("  Step 1: Build Rust libraries for Android ABIs (arm64-v8a, armeabi-v7a, x86_64)");
            println!("    Command: cargo ndk --target <abi> --platform 24 build {}",
                if matches!(config.profile, BuildProfile::Release) { "--release" } else { "" });
            println!("  Step 2: Generate UniFFI Kotlin bindings");
            println!("    Output: {:?}", android_dir.join("app/src/main/java/uniffi"));
            println!("  Step 3: Copy .so files to jniLibs directories");
            println!("    Destination: {:?}", android_dir.join("app/src/main/jniLibs"));
            println!("  Step 4: Build Android APK with Gradle");
            println!("    Command: ./gradlew assemble{}", if profile_name == "release" { "Release" } else { "Debug" });
            println!("    Output: {:?}", android_dir.join(format!("app/build/outputs/apk/{}/app-{}.apk", profile_name, profile_name)));
            println!("  Step 5: Build Android test APK");
            println!("    Command: ./gradlew assemble{}AndroidTest", if profile_name == "release" { "Release" } else { "Debug" });

            // Return a placeholder result for dry-run
            return Ok(BuildResult {
                platform: Target::Android,
                app_path: android_dir.join(format!("app/build/outputs/apk/{}/app-{}.apk", profile_name, profile_name)),
                test_suite_path: Some(android_dir.join(format!("app/build/outputs/apk/androidTest/{}/app-{}-androidTest.apk", profile_name, profile_name))),
            });
        }

        // Step 0: Ensure Android project scaffolding exists
        // Pass project_root and crate_dir for better benchmark function detection
        crate::codegen::ensure_android_project_with_options(
            &self.output_dir,
            &self.crate_name,
            Some(&self.project_root),
            self.crate_dir.as_deref(),
        )?;

        // Step 0.5: Ensure Gradle wrapper exists
        self.ensure_gradle_wrapper(&android_dir)?;

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

        // Step 5: Build Android test APK for BrowserStack
        println!("Building Android test APK...");
        let test_suite_path = self.build_test_apk(config)?;

        // Step 6: Validate all expected artifacts exist
        let result = BuildResult {
            platform: Target::Android,
            app_path: apk_path,
            test_suite_path: Some(test_suite_path),
        };
        self.validate_build_artifacts(&result, config)?;

        Ok(result)
    }

    /// Validates that all expected build artifacts exist after a successful build
    fn validate_build_artifacts(&self, result: &BuildResult, config: &BuildConfig) -> Result<(), BenchError> {
        let mut missing = Vec::new();
        let profile_dir = match config.profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        };

        // Check main APK
        if !result.app_path.exists() {
            missing.push(format!("Main APK: {}", result.app_path.display()));
        }

        // Check test APK
        if let Some(ref test_path) = result.test_suite_path {
            if !test_path.exists() {
                missing.push(format!("Test APK: {}", test_path.display()));
            }
        }

        // Check that at least one native library exists in jniLibs
        let jni_libs_dir = self.output_dir.join("android/app/src/main/jniLibs");
        let lib_name = format!("lib{}.so", self.crate_name.replace("-", "_"));
        let required_abis = ["arm64-v8a", "armeabi-v7a", "x86_64"];
        let mut found_libs = 0;
        for abi in &required_abis {
            let lib_path = jni_libs_dir.join(abi).join(&lib_name);
            if lib_path.exists() {
                found_libs += 1;
            } else {
                missing.push(format!("Native library ({} {}): {}", abi, profile_dir, lib_path.display()));
            }
        }

        if found_libs == 0 {
            return Err(BenchError::Build(format!(
                "Build validation failed: No native libraries found.\n\n\
                 Expected at least one .so file in jniLibs directories.\n\
                 Missing artifacts:\n{}\n\n\
                 This usually means the Rust build step failed. Check the cargo-ndk output above.",
                missing.iter().map(|s| format!("  - {}", s)).collect::<Vec<_>>().join("\n")
            )));
        }

        if !missing.is_empty() {
            eprintln!(
                "Warning: Some build artifacts are missing:\n{}\n\
                 The build may still work but some features might be unavailable.",
                missing.iter().map(|s| format!("  - {}", s)).collect::<Vec<_>>().join("\n")
            );
        }

        Ok(())
    }

    /// Finds the benchmark crate directory.
    ///
    /// Search order:
    /// 1. Explicit `crate_dir` if set via `.crate_dir()` builder method
    /// 2. Current directory (`project_root`) if its Cargo.toml has a matching package name
    /// 3. `{project_root}/bench-mobile/` (SDK projects)
    /// 4. `{project_root}/crates/{crate_name}/` (repository structure)
    fn find_crate_dir(&self) -> Result<PathBuf, BenchError> {
        // If explicit crate_dir was provided, use it
        if let Some(ref dir) = self.crate_dir {
            if dir.exists() {
                return Ok(dir.clone());
            }
            return Err(BenchError::Build(format!(
                "Specified crate path does not exist: {:?}.\n\n\
                 Tip: pass --crate-path pointing at a directory containing Cargo.toml.",
                dir
            )));
        }

        // Check if the current directory (project_root) IS the crate
        // This handles the case where user runs `cargo mobench build` from within the crate directory
        let root_cargo_toml = self.project_root.join("Cargo.toml");
        if root_cargo_toml.exists() {
            if let Some(pkg_name) = super::common::read_package_name(&root_cargo_toml) {
                if pkg_name == self.crate_name {
                    return Ok(self.project_root.clone());
                }
            }
        }

        // Try bench-mobile/ (SDK projects)
        let bench_mobile_dir = self.project_root.join("bench-mobile");
        if bench_mobile_dir.exists() {
            return Ok(bench_mobile_dir);
        }

        // Try crates/{crate_name}/ (repository structure)
        let crates_dir = self.project_root.join("crates").join(&self.crate_name);
        if crates_dir.exists() {
            return Ok(crates_dir);
        }

        // Also try {crate_name}/ in project root (common pattern)
        let named_dir = self.project_root.join(&self.crate_name);
        if named_dir.exists() {
            return Ok(named_dir);
        }

        let root_manifest = root_cargo_toml;
        let bench_mobile_manifest = bench_mobile_dir.join("Cargo.toml");
        let crates_manifest = crates_dir.join("Cargo.toml");
        let named_manifest = named_dir.join("Cargo.toml");
        Err(BenchError::Build(format!(
            "Benchmark crate '{}' not found.\n\n\
             Searched locations:\n\
             - {} (checked [package] name)\n\
             - {}\n\
             - {}\n\
             - {}\n\n\
             To fix this:\n\
             1. Run from the crate directory (where Cargo.toml has name = \"{}\")\n\
             2. Create a bench-mobile/ directory with your benchmark crate, or\n\
             3. Use --crate-path to specify the benchmark crate location:\n\
                cargo mobench build --target android --crate-path ./my-benchmarks\n\n\
             Common issues:\n\
             - Typo in crate name (check Cargo.toml [package] name)\n\
             - Wrong working directory (run from project root)\n\
             - Missing Cargo.toml in the crate directory\n\n\
             Run 'cargo mobench init --help' to generate a new benchmark project.",
            self.crate_name,
            root_manifest.display(),
            bench_mobile_manifest.display(),
            crates_manifest.display(),
            named_manifest.display(),
            self.crate_name,
        )))
    }

    /// Builds Rust libraries for Android using cargo-ndk
    fn build_rust_libraries(&self, config: &BuildConfig) -> Result<(), BenchError> {
        let crate_dir = self.find_crate_dir()?;

        // Check if cargo-ndk is installed
        self.check_cargo_ndk()?;

        // Android ABIs to build for
        let abis = vec!["arm64-v8a", "armeabi-v7a", "x86_64"];
        let release_flag = if matches!(config.profile, BuildProfile::Release) {
            "--release"
        } else {
            ""
        };

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
            if !release_flag.is_empty() {
                cmd.arg(release_flag);
            }

            // Set working directory
            cmd.current_dir(&crate_dir);

            // Execute build
            let command_hint = if release_flag.is_empty() {
                format!("cargo ndk --target {} --platform 24 build", abi)
            } else {
                format!("cargo ndk --target {} --platform 24 build {}", abi, release_flag)
            };
            let output = cmd
                .output()
                .map_err(|e| BenchError::Build(format!(
                    "Failed to start cargo-ndk for {}.\n\n\
                     Command: {}\n\
                     Crate directory: {}\n\
                     System error: {}\n\n\
                     Tips:\n\
                     - Install cargo-ndk: cargo install cargo-ndk\n\
                     - Ensure cargo is on PATH",
                    abi,
                    command_hint,
                    crate_dir.display(),
                    e
                )))?;

            if !output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let profile = if matches!(config.profile, BuildProfile::Release) {
                    "release"
                } else {
                    "debug"
                };
                let rust_target = match abi {
                    "arm64-v8a" => "aarch64-linux-android",
                    "armeabi-v7a" => "armv7-linux-androideabi",
                    "x86_64" => "x86_64-linux-android",
                    _ => abi,
                };
                return Err(BenchError::Build(format!(
                    "cargo-ndk build failed for {} ({} profile).\n\n\
                     Command: {}\n\
                     Crate directory: {}\n\
                     Exit status: {}\n\n\
                     Stdout:\n{}\n\n\
                     Stderr:\n{}\n\n\
                     Common causes:\n\
                     - Missing Rust target: rustup target add {}\n\
                     - NDK not found: set ANDROID_NDK_HOME\n\
                     - Compilation error in Rust code (see output above)\n\
                     - Incompatible native dependencies (some C libraries do not support Android)",
                    abi,
                    profile,
                    command_hint,
                    crate_dir.display(),
                    output.status,
                    stdout,
                    stderr,
                    rust_target,
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
                "cargo-ndk is not installed or not in PATH.\n\n\
                 cargo-ndk is required to cross-compile Rust for Android.\n\n\
                 To install:\n\
                   cargo install cargo-ndk\n\
                 Verify with:\n\
                   cargo ndk --version\n\n\
                 You also need the Android NDK. Set ANDROID_NDK_HOME or install via Android Studio.\n\
                 See: https://github.com/nickelc/cargo-ndk"
                    .to_string(),
            )),
        }
    }

    /// Generates UniFFI Kotlin bindings
    fn generate_uniffi_bindings(&self) -> Result<(), BenchError> {
        let crate_dir = self.find_crate_dir()?;
        let crate_name_underscored = self.crate_name.replace("-", "_");

        // Check if bindings already exist (for repository testing with pre-generated bindings)
        let bindings_path = self
            .output_dir
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

        // Build host library to feed uniffi-bindgen
        let mut build_cmd = Command::new("cargo");
        build_cmd.arg("build");
        build_cmd.current_dir(&crate_dir);
        run_command(build_cmd, "cargo build (host)")?;

        let lib_path = host_lib_path(&crate_dir, &self.crate_name)?;
        let out_dir = self
            .output_dir
            .join("android")
            .join("app")
            .join("src")
            .join("main")
            .join("java");

        // Try cargo run first (works if crate has uniffi-bindgen binary target)
        let cargo_run_result = Command::new("cargo")
            .args(["run", "-p", &self.crate_name, "--bin", "uniffi-bindgen", "--"])
            .arg("generate")
            .arg("--library")
            .arg(&lib_path)
            .arg("--language")
            .arg("kotlin")
            .arg("--out-dir")
            .arg(&out_dir)
            .current_dir(&crate_dir)
            .output();

        let use_cargo_run = cargo_run_result
            .as_ref()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if use_cargo_run {
            if self.verbose {
                println!("  Generated bindings using cargo run uniffi-bindgen");
            }
        } else {
            // Fall back to global uniffi-bindgen
            let uniffi_available = Command::new("uniffi-bindgen")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            if !uniffi_available {
                return Err(BenchError::Build(
                    "uniffi-bindgen not found and no pre-generated bindings exist.\n\n\
                     To fix this, either:\n\
                     1. Add a uniffi-bindgen binary to your crate:\n\
                        [[bin]]\n\
                        name = \"uniffi-bindgen\"\n\
                        path = \"src/bin/uniffi-bindgen.rs\"\n\n\
                     2. Or install uniffi-bindgen globally:\n\
                        cargo install uniffi-bindgen\n\n\
                     3. Or pre-generate bindings and commit them."
                        .to_string(),
                ));
            }

            let mut cmd = Command::new("uniffi-bindgen");
            cmd.arg("generate")
                .arg("--library")
                .arg(&lib_path)
                .arg("--language")
                .arg("kotlin")
                .arg("--out-dir")
                .arg(&out_dir);
            run_command(cmd, "uniffi-bindgen kotlin")?;
        }

        if self.verbose {
            println!("  Generated UniFFI Kotlin bindings at {:?}", out_dir);
        }
        Ok(())
    }

    /// Copies .so files to Android jniLibs directories
    fn copy_native_libraries(&self, config: &BuildConfig) -> Result<(), BenchError> {
        let crate_dir = self.find_crate_dir()?;
        let profile_dir = match config.profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        };

        // Use cargo metadata to find the actual target directory (handles workspaces)
        let target_dir = get_cargo_target_dir(&crate_dir)?;
        let jni_libs_dir = self.output_dir.join("android/app/src/main/jniLibs");

        // Create jniLibs directories if they don't exist
        std::fs::create_dir_all(&jni_libs_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create jniLibs directory at {}: {}. Check output directory permissions.",
                jni_libs_dir.display(),
                e
            ))
        })?;

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
                BenchError::Build(format!(
                    "Failed to create ABI directory {} at {}: {}. Check output directory permissions.",
                    android_abi,
                    dest_dir.display(),
                    e
                ))
            })?;

            let dest = dest_dir.join(format!("lib{}.so", self.crate_name.replace("-", "_")));

            if src.exists() {
                std::fs::copy(&src, &dest).map_err(|e| {
                    BenchError::Build(format!(
                        "Failed to copy {} library from {} to {}: {}. Ensure cargo-ndk completed successfully.",
                        android_abi,
                        src.display(),
                        dest.display(),
                        e
                    ))
                })?;

                if self.verbose {
                    println!("  Copied {} -> {}", src.display(), dest.display());
                }
            } else {
                // Always warn about missing native libraries - this will cause runtime crashes
                eprintln!(
                    "Warning: Native library for {} not found at {}.\n\
                     This will cause a runtime crash when the app tries to load the library.\n\
                     Ensure cargo-ndk build completed successfully for this ABI.",
                    android_abi,
                    src.display()
                );
            }
        }

        Ok(())
    }

    /// Ensures local.properties exists with sdk.dir set
    ///
    /// Gradle requires this file to know where the Android SDK is located.
    /// This function only generates the file if ANDROID_HOME or ANDROID_SDK_ROOT
    /// environment variables are set. We intentionally avoid probing filesystem
    /// paths to prevent writing machine-specific paths that would break builds
    /// on other machines.
    ///
    /// If neither environment variable is set, we skip generating the file and
    /// let Android Studio or Gradle handle SDK detection.
    fn ensure_local_properties(&self, android_dir: &Path) -> Result<(), BenchError> {
        let local_props = android_dir.join("local.properties");

        // If local.properties already exists, leave it alone
        if local_props.exists() {
            return Ok(());
        }

        // Only generate local.properties if an environment variable is set.
        // This avoids writing machine-specific paths that break on other machines.
        let sdk_dir = self.find_android_sdk_from_env();

        match sdk_dir {
            Some(path) => {
                // Write local.properties with the SDK path from env var
                let content = format!("sdk.dir={}\n", path.display());
                fs::write(&local_props, content).map_err(|e| {
                    BenchError::Build(format!(
                        "Failed to write local.properties at {:?}: {}. Check output directory permissions.",
                        local_props, e
                    ))
                })?;

                if self.verbose {
                    println!("  Generated local.properties with sdk.dir={}", path.display());
                }
            }
            None => {
                // No env var set - skip generating local.properties
                // Gradle/Android Studio will auto-detect the SDK or prompt the user
                if self.verbose {
                    println!("  Skipping local.properties generation (ANDROID_HOME/ANDROID_SDK_ROOT not set)");
                    println!("  Gradle will auto-detect SDK or you can create local.properties manually");
                }
            }
        }

        Ok(())
    }

    /// Finds the Android SDK installation path from environment variables only
    ///
    /// Returns Some(path) if ANDROID_HOME or ANDROID_SDK_ROOT is set and the path exists.
    /// Returns None if neither is set or the paths don't exist.
    ///
    /// We intentionally avoid probing common filesystem locations to prevent
    /// writing machine-specific paths that would break builds on other machines.
    fn find_android_sdk_from_env(&self) -> Option<PathBuf> {
        // Check ANDROID_HOME first (standard)
        if let Ok(path) = env::var("ANDROID_HOME") {
            let sdk_path = PathBuf::from(&path);
            if sdk_path.exists() {
                return Some(sdk_path);
            }
        }

        // Check ANDROID_SDK_ROOT (alternative)
        if let Ok(path) = env::var("ANDROID_SDK_ROOT") {
            let sdk_path = PathBuf::from(&path);
            if sdk_path.exists() {
                return Some(sdk_path);
            }
        }

        None
    }

    /// Ensures the Gradle wrapper (gradlew) exists in the Android project
    ///
    /// If gradlew doesn't exist, this runs `gradle wrapper --gradle-version 8.5`
    /// to generate the wrapper files.
    fn ensure_gradle_wrapper(&self, android_dir: &Path) -> Result<(), BenchError> {
        let gradlew = android_dir.join("gradlew");

        // If gradlew already exists, we're good
        if gradlew.exists() {
            return Ok(());
        }

        println!("Gradle wrapper not found, generating...");

        // Check if gradle is available
        let gradle_available = Command::new("gradle")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !gradle_available {
            return Err(BenchError::Build(
                "Gradle wrapper (gradlew) not found and 'gradle' command is not available.\n\n\
                 The Android project requires Gradle to build. You have two options:\n\n\
                 1. Install Gradle globally and run the build again (it will auto-generate the wrapper):\n\
                    - macOS: brew install gradle\n\
                    - Linux: sudo apt install gradle\n\
                    - Or download from https://gradle.org/install/\n\n\
                 2. Or generate the wrapper manually in the Android project directory:\n\
                    cd target/mobench/android && gradle wrapper --gradle-version 8.5"
                    .to_string(),
            ));
        }

        // Run gradle wrapper to generate gradlew
        let mut cmd = Command::new("gradle");
        cmd.arg("wrapper")
            .arg("--gradle-version")
            .arg("8.5")
            .current_dir(android_dir);

        let output = cmd.output().map_err(|e| {
            BenchError::Build(format!(
                "Failed to run 'gradle wrapper' command: {}\n\n\
                 Ensure Gradle is installed and on your PATH.",
                e
            ))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BenchError::Build(format!(
                "Failed to generate Gradle wrapper.\n\n\
                 Command: gradle wrapper --gradle-version 8.5\n\
                 Working directory: {}\n\
                 Exit status: {}\n\
                 Stderr: {}\n\n\
                 Try running this command manually in the Android project directory.",
                android_dir.display(),
                output.status,
                stderr
            )));
        }

        // Make gradlew executable on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(&gradlew) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&gradlew, perms);
            }
        }

        if self.verbose {
            println!("  Generated Gradle wrapper at {:?}", gradlew);
        }

        Ok(())
    }

    /// Builds the Android APK using Gradle
    fn build_apk(&self, config: &BuildConfig) -> Result<PathBuf, BenchError> {
        let android_dir = self.output_dir.join("android");

        if !android_dir.exists() {
            return Err(BenchError::Build(format!(
                "Android project not found at {}.\n\n\
                 Expected a Gradle project under the output directory.\n\
                 Run `cargo mobench init --target android` or `cargo mobench build --target android` from the project root to generate it.",
                android_dir.display()
            )));
        }

        // Ensure local.properties exists with sdk.dir
        self.ensure_local_properties(&android_dir)?;

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
            .map_err(|e| BenchError::Build(format!(
                "Failed to run Gradle wrapper.\n\n\
                 Command: ./gradlew {}\n\
                 Working directory: {}\n\
                 Error: {}\n\n\
                 Tips:\n\
                 - Ensure ./gradlew is executable (chmod +x ./gradlew)\n\
                 - Run ./gradlew --version in that directory to verify the wrapper",
                gradle_task,
                android_dir.display(),
                e
            )))?;

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BenchError::Build(format!(
                "Gradle build failed.\n\n\
                 Command: ./gradlew {}\n\
                 Working directory: {}\n\
                 Exit status: {}\n\n\
                 Stdout:\n{}\n\n\
                 Stderr:\n{}\n\n\
                 Tips:\n\
                 - Re-run with verbose mode to pass --info to Gradle\n\
                 - Run ./gradlew {} --stacktrace for a full stack trace",
                gradle_task,
                android_dir.display(),
                output.status,
                stdout,
                stderr,
                gradle_task,
            )));
        }

        // Determine APK path
        let profile_name = match config.profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        };

        let apk_dir = android_dir.join("app/build/outputs/apk").join(profile_name);

        // Try to find APK - check multiple possible filenames
        // Gradle produces different names depending on signing configuration:
        // - app-release.apk (signed)
        // - app-release-unsigned.apk (unsigned release)
        // - app-debug.apk (debug)
        let apk_path = self.find_apk(&apk_dir, profile_name, gradle_task)?;

        Ok(apk_path)
    }

    /// Finds the APK file in the build output directory
    ///
    /// Gradle produces different APK filenames depending on signing configuration:
    /// - `app-release.apk` - signed release build
    /// - `app-release-unsigned.apk` - unsigned release build
    /// - `app-debug.apk` - debug build
    ///
    /// This method also checks for `output-metadata.json` which contains the actual
    /// output filename when present.
    fn find_apk(&self, apk_dir: &Path, profile_name: &str, gradle_task: &str) -> Result<PathBuf, BenchError> {
        // First, try to read output-metadata.json for the actual APK name
        let metadata_path = apk_dir.join("output-metadata.json");
        if metadata_path.exists() {
            if let Ok(metadata_content) = fs::read_to_string(&metadata_path) {
                // Parse the JSON to find the outputFile
                // Format: {"elements":[{"outputFile":"app-release-unsigned.apk",...}]}
                if let Some(apk_name) = self.parse_output_metadata(&metadata_content) {
                    let apk_path = apk_dir.join(&apk_name);
                    if apk_path.exists() {
                        if self.verbose {
                            println!("  Found APK from output-metadata.json: {}", apk_path.display());
                        }
                        return Ok(apk_path);
                    }
                }
            }
        }

        // Define candidates in order of preference
        let candidates = if profile_name == "release" {
            vec![
                format!("app-{}.apk", profile_name),           // Signed release
                format!("app-{}-unsigned.apk", profile_name),  // Unsigned release
            ]
        } else {
            vec![
                format!("app-{}.apk", profile_name),           // Debug
            ]
        };

        // Check each candidate
        for candidate in &candidates {
            let apk_path = apk_dir.join(candidate);
            if apk_path.exists() {
                if self.verbose {
                    println!("  Found APK: {}", apk_path.display());
                }
                return Ok(apk_path);
            }
        }

        // No APK found - provide helpful error message
        Err(BenchError::Build(format!(
            "APK not found in {}.\n\n\
             Gradle task {} reported success but no APK was produced.\n\
             Searched for:\n{}\n\n\
             Check the build output directory and rerun ./gradlew {} if needed.",
            apk_dir.display(),
            gradle_task,
            candidates.iter().map(|c| format!("  - {}", c)).collect::<Vec<_>>().join("\n"),
            gradle_task
        )))
    }

    /// Parses output-metadata.json to extract the APK filename
    ///
    /// The JSON format is:
    /// ```json
    /// {
    ///   "elements": [
    ///     {
    ///       "outputFile": "app-release-unsigned.apk",
    ///       ...
    ///     }
    ///   ]
    /// }
    /// ```
    fn parse_output_metadata(&self, content: &str) -> Option<String> {
        // Simple JSON parsing without external dependencies
        // Look for "outputFile":"<filename>"
        let pattern = "\"outputFile\"";
        if let Some(pos) = content.find(pattern) {
            let after_key = &content[pos + pattern.len()..];
            // Skip whitespace and colon
            let after_colon = after_key.trim_start().strip_prefix(':')?;
            let after_ws = after_colon.trim_start();
            // Extract the string value
            if after_ws.starts_with('"') {
                let value_start = &after_ws[1..];
                if let Some(end_quote) = value_start.find('"') {
                    let filename = &value_start[..end_quote];
                    if filename.ends_with(".apk") {
                        return Some(filename.to_string());
                    }
                }
            }
        }
        None
    }

    /// Builds the Android test APK using Gradle
    fn build_test_apk(&self, config: &BuildConfig) -> Result<PathBuf, BenchError> {
        let android_dir = self.output_dir.join("android");

        if !android_dir.exists() {
            return Err(BenchError::Build(format!(
                "Android project not found at {}.\n\n\
                 Expected a Gradle project under the output directory.\n\
                 Run `cargo mobench init --target android` or `cargo mobench build --target android` from the project root to generate it.",
                android_dir.display()
            )));
        }

        let gradle_task = match config.profile {
            BuildProfile::Debug => "assembleDebugAndroidTest",
            BuildProfile::Release => "assembleReleaseAndroidTest",
        };

        let mut cmd = Command::new("./gradlew");
        cmd.arg(gradle_task).current_dir(&android_dir);

        if self.verbose {
            cmd.arg("--info");
        }

        let output = cmd
            .output()
            .map_err(|e| BenchError::Build(format!(
                "Failed to run Gradle wrapper.\n\n\
                 Command: ./gradlew {}\n\
                 Working directory: {}\n\
                 Error: {}\n\n\
                 Tips:\n\
                 - Ensure ./gradlew is executable (chmod +x ./gradlew)\n\
                 - Run ./gradlew --version in that directory to verify the wrapper",
                gradle_task,
                android_dir.display(),
                e
            )))?;

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BenchError::Build(format!(
                "Gradle test APK build failed.\n\n\
                 Command: ./gradlew {}\n\
                 Working directory: {}\n\
                 Exit status: {}\n\n\
                 Stdout:\n{}\n\n\
                 Stderr:\n{}\n\n\
                 Tips:\n\
                 - Re-run with verbose mode to pass --info to Gradle\n\
                 - Run ./gradlew {} --stacktrace for a full stack trace",
                gradle_task,
                android_dir.display(),
                output.status,
                stdout,
                stderr,
                gradle_task,
            )));
        }

        let profile_name = match config.profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        };

        let test_apk_dir = android_dir
            .join("app/build/outputs/apk/androidTest")
            .join(profile_name);

        // Find the test APK - use similar logic to main APK
        let apk_path = self.find_test_apk(&test_apk_dir, profile_name, gradle_task)?;

        Ok(apk_path)
    }

    /// Finds the test APK file in the build output directory
    ///
    /// Test APKs can have different naming patterns depending on the build:
    /// - `app-debug-androidTest.apk`
    /// - `app-release-androidTest.apk`
    fn find_test_apk(&self, apk_dir: &Path, profile_name: &str, gradle_task: &str) -> Result<PathBuf, BenchError> {
        // First, try to read output-metadata.json for the actual APK name
        let metadata_path = apk_dir.join("output-metadata.json");
        if metadata_path.exists() {
            if let Ok(metadata_content) = fs::read_to_string(&metadata_path) {
                if let Some(apk_name) = self.parse_output_metadata(&metadata_content) {
                    let apk_path = apk_dir.join(&apk_name);
                    if apk_path.exists() {
                        if self.verbose {
                            println!("  Found test APK from output-metadata.json: {}", apk_path.display());
                        }
                        return Ok(apk_path);
                    }
                }
            }
        }

        // Check standard naming pattern
        let apk_path = apk_dir.join(format!("app-{}-androidTest.apk", profile_name));
        if apk_path.exists() {
            if self.verbose {
                println!("  Found test APK: {}", apk_path.display());
            }
            return Ok(apk_path);
        }

        // No test APK found
        Err(BenchError::Build(format!(
            "Android test APK not found in {}.\n\n\
             Gradle task {} reported success but no test APK was produced.\n\
             Expected: app-{}-androidTest.apk\n\n\
             Check app/build/outputs/apk/androidTest/{} and rerun ./gradlew {} if needed.",
            apk_dir.display(),
            gradle_task,
            profile_name,
            profile_name,
            gradle_task
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_android_builder_creation() {
        let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile");
        assert!(!builder.verbose);
        assert_eq!(
            builder.output_dir,
            PathBuf::from("/tmp/test-project/target/mobench")
        );
    }

    #[test]
    fn test_android_builder_verbose() {
        let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile").verbose(true);
        assert!(builder.verbose);
    }

    #[test]
    fn test_android_builder_custom_output_dir() {
        let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile")
            .output_dir("/custom/output");
        assert_eq!(builder.output_dir, PathBuf::from("/custom/output"));
    }

    #[test]
    fn test_parse_output_metadata_unsigned() {
        let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile");
        let metadata = r#"{"version":3,"artifactType":{"type":"APK","kind":"Directory"},"applicationId":"dev.world.bench","variantName":"release","elements":[{"type":"SINGLE","filters":[],"attributes":[],"versionCode":1,"versionName":"0.1","outputFile":"app-release-unsigned.apk"}],"elementType":"File"}"#;
        let result = builder.parse_output_metadata(metadata);
        assert_eq!(result, Some("app-release-unsigned.apk".to_string()));
    }

    #[test]
    fn test_parse_output_metadata_signed() {
        let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile");
        let metadata = r#"{"version":3,"elements":[{"outputFile":"app-release.apk"}]}"#;
        let result = builder.parse_output_metadata(metadata);
        assert_eq!(result, Some("app-release.apk".to_string()));
    }

    #[test]
    fn test_parse_output_metadata_no_apk() {
        let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile");
        let metadata = r#"{"version":3,"elements":[]}"#;
        let result = builder.parse_output_metadata(metadata);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_output_metadata_invalid_json() {
        let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile");
        let metadata = "not valid json";
        let result = builder.parse_output_metadata(metadata);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_crate_dir_current_directory_is_crate() {
        // Test case 1: Current directory IS the crate with matching package name
        let temp_dir = std::env::temp_dir().join("mobench-test-find-crate-current");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Create Cargo.toml with matching package name
        std::fs::write(
            temp_dir.join("Cargo.toml"),
            r#"[package]
name = "bench-mobile"
version = "0.1.0"
"#,
        )
        .unwrap();

        let builder = AndroidBuilder::new(&temp_dir, "bench-mobile");
        let result = builder.find_crate_dir();
        assert!(result.is_ok(), "Should find crate in current directory");
        assert_eq!(result.unwrap(), temp_dir);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_find_crate_dir_nested_bench_mobile() {
        // Test case 2: Crate is in bench-mobile/ subdirectory
        let temp_dir = std::env::temp_dir().join("mobench-test-find-crate-nested");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("bench-mobile")).unwrap();

        // Create parent Cargo.toml (workspace or different crate)
        std::fs::write(
            temp_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["bench-mobile"]
"#,
        )
        .unwrap();

        // Create bench-mobile/Cargo.toml
        std::fs::write(
            temp_dir.join("bench-mobile/Cargo.toml"),
            r#"[package]
name = "bench-mobile"
version = "0.1.0"
"#,
        )
        .unwrap();

        let builder = AndroidBuilder::new(&temp_dir, "bench-mobile");
        let result = builder.find_crate_dir();
        assert!(result.is_ok(), "Should find crate in bench-mobile/ directory");
        assert_eq!(result.unwrap(), temp_dir.join("bench-mobile"));

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_find_crate_dir_crates_subdir() {
        // Test case 3: Crate is in crates/{name}/ subdirectory
        let temp_dir = std::env::temp_dir().join("mobench-test-find-crate-crates");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("crates/my-bench")).unwrap();

        // Create workspace Cargo.toml
        std::fs::write(
            temp_dir.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/*"]
"#,
        )
        .unwrap();

        // Create crates/my-bench/Cargo.toml
        std::fs::write(
            temp_dir.join("crates/my-bench/Cargo.toml"),
            r#"[package]
name = "my-bench"
version = "0.1.0"
"#,
        )
        .unwrap();

        let builder = AndroidBuilder::new(&temp_dir, "my-bench");
        let result = builder.find_crate_dir();
        assert!(result.is_ok(), "Should find crate in crates/ directory");
        assert_eq!(result.unwrap(), temp_dir.join("crates/my-bench"));

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_find_crate_dir_not_found() {
        // Test case 4: Crate doesn't exist anywhere
        let temp_dir = std::env::temp_dir().join("mobench-test-find-crate-notfound");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Create Cargo.toml with DIFFERENT package name
        std::fs::write(
            temp_dir.join("Cargo.toml"),
            r#"[package]
name = "some-other-crate"
version = "0.1.0"
"#,
        )
        .unwrap();

        let builder = AndroidBuilder::new(&temp_dir, "nonexistent-crate");
        let result = builder.find_crate_dir();
        assert!(result.is_err(), "Should fail to find nonexistent crate");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Benchmark crate 'nonexistent-crate' not found"));
        assert!(err_msg.contains("Searched locations"));

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_find_crate_dir_explicit_crate_path() {
        // Test case 5: Explicit crate_dir overrides auto-detection
        let temp_dir = std::env::temp_dir().join("mobench-test-find-crate-explicit");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("custom-location")).unwrap();

        let builder = AndroidBuilder::new(&temp_dir, "any-name")
            .crate_dir(temp_dir.join("custom-location"));
        let result = builder.find_crate_dir();
        assert!(result.is_ok(), "Should use explicit crate_dir");
        assert_eq!(result.unwrap(), temp_dir.join("custom-location"));

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }
}
