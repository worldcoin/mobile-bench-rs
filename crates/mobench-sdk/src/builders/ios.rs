//! iOS build automation.
//!
//! This module provides [`IosBuilder`] which handles the complete pipeline for
//! building Rust libraries for iOS and packaging them into an xcframework that
//! can be used in Xcode projects.
//!
//! ## Build Pipeline
//!
//! The builder performs these steps:
//!
//! 1. **Project scaffolding** - Auto-generates iOS project if missing
//! 2. **Rust compilation** - Builds static libraries for device and simulator targets
//! 3. **Binding generation** - Generates UniFFI Swift bindings and C headers
//! 4. **XCFramework creation** - Creates properly structured xcframework with slices
//! 5. **Code signing** - Signs the xcframework for Xcode acceptance
//! 6. **Xcode project generation** - Runs xcodegen if `project.yml` exists
//!
//! ## Requirements
//!
//! - Xcode with command line tools (`xcode-select --install`)
//! - Rust targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`
//! - `uniffi-bindgen` for Swift binding generation
//! - `xcodegen` (optional, `brew install xcodegen`)
//!
//! ## Example
//!
//! ```ignore
//! use mobench_sdk::builders::{IosBuilder, SigningMethod};
//! use mobench_sdk::{BuildConfig, BuildProfile, Target};
//!
//! let builder = IosBuilder::new(".", "my-bench-crate")
//!     .verbose(true)
//!     .dry_run(false);
//!
//! let config = BuildConfig {
//!     target: Target::Ios,
//!     profile: BuildProfile::Release,
//!     incremental: true,
//! };
//!
//! let result = builder.build(&config)?;
//! println!("XCFramework at: {:?}", result.app_path);
//!
//! // Package IPA for BrowserStack or device testing
//! let ipa_path = builder.package_ipa("BenchRunner", SigningMethod::AdHoc)?;
//! # Ok::<(), mobench_sdk::BenchError>(())
//! ```
//!
//! ## Dry-Run Mode
//!
//! Use `dry_run(true)` to preview the build plan without making changes:
//!
//! ```ignore
//! let builder = IosBuilder::new(".", "my-bench")
//!     .dry_run(true);
//!
//! // This will print the build plan but not execute anything
//! builder.build(&config)?;
//! ```
//!
//! ## IPA Packaging
//!
//! After building the xcframework, you can package an IPA for device testing:
//!
//! ```ignore
//! // Ad-hoc signing (works for BrowserStack, no Apple ID needed)
//! let ipa = builder.package_ipa("BenchRunner", SigningMethod::AdHoc)?;
//!
//! // Development signing (requires Apple Developer account)
//! let ipa = builder.package_ipa("BenchRunner", SigningMethod::Development)?;
//! ```

use crate::types::{BenchError, BuildConfig, BuildProfile, BuildResult, Target};
use super::common::{get_cargo_target_dir, host_lib_path, run_command};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// iOS builder that handles the complete build pipeline.
///
/// This builder automates the process of compiling Rust code to iOS static
/// libraries, generating UniFFI Swift bindings, creating an xcframework,
/// and optionally packaging an IPA for device deployment.
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::builders::{IosBuilder, SigningMethod};
/// use mobench_sdk::{BuildConfig, BuildProfile, Target};
///
/// let builder = IosBuilder::new(".", "my-bench")
///     .verbose(true)
///     .output_dir("target/mobench");
///
/// let config = BuildConfig {
///     target: Target::Ios,
///     profile: BuildProfile::Release,
///     incremental: true,
/// };
///
/// let result = builder.build(&config)?;
///
/// // Optional: Package IPA for device testing
/// let ipa = builder.package_ipa("BenchRunner", SigningMethod::AdHoc)?;
/// # Ok::<(), mobench_sdk::BenchError>(())
/// ```
pub struct IosBuilder {
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

impl IosBuilder {
    /// Creates a new iOS builder
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
    /// By default, the builder searches for the crate in:
    /// - `{project_root}/bench-mobile/`
    /// - `{project_root}/crates/{crate_name}/`
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

    /// Builds the iOS app with the given configuration
    ///
    /// This performs the following steps:
    /// 0. Auto-generate project scaffolding if missing
    /// 1. Build Rust libraries for iOS targets (device + simulator)
    /// 2. Generate UniFFI Swift bindings and C headers
    /// 3. Create xcframework with proper structure
    /// 4. Code-sign the xcframework
    /// 5. Generate Xcode project with xcodegen (if project.yml exists)
    ///
    /// # Returns
    ///
    /// * `Ok(BuildResult)` containing the path to the xcframework
    /// * `Err(BenchError)` if the build fails
    pub fn build(&self, config: &BuildConfig) -> Result<BuildResult, BenchError> {
        let framework_name = self.crate_name.replace("-", "_");
        let ios_dir = self.output_dir.join("ios");
        let xcframework_path = ios_dir.join(format!("{}.xcframework", framework_name));

        if self.dry_run {
            println!("\n[dry-run] iOS build plan:");
            println!("  Step 0: Check/generate iOS project scaffolding at {:?}", ios_dir.join("BenchRunner"));
            println!("  Step 1: Build Rust libraries for iOS targets");
            println!("    Command: cargo build --target aarch64-apple-ios --lib {}",
                if matches!(config.profile, BuildProfile::Release) { "--release" } else { "" });
            println!("    Command: cargo build --target aarch64-apple-ios-sim --lib {}",
                if matches!(config.profile, BuildProfile::Release) { "--release" } else { "" });
            println!("  Step 2: Generate UniFFI Swift bindings");
            println!("    Output: {:?}", ios_dir.join("BenchRunner/BenchRunner/Generated"));
            println!("  Step 3: Create xcframework at {:?}", xcframework_path);
            println!("    - ios-arm64/{}.framework (device)", framework_name);
            println!("    - ios-simulator-arm64/{}.framework (simulator)", framework_name);
            println!("  Step 4: Code-sign xcframework");
            println!("    Command: codesign --force --deep --sign - {:?}", xcframework_path);
            println!("  Step 5: Generate Xcode project with xcodegen (if project.yml exists)");
            println!("    Command: xcodegen generate");

            // Return a placeholder result for dry-run
            return Ok(BuildResult {
                platform: Target::Ios,
                app_path: xcframework_path,
                test_suite_path: None,
            });
        }

        // Step 0: Ensure iOS project scaffolding exists
        crate::codegen::ensure_ios_project(&self.output_dir, &self.crate_name)?;

        // Step 1: Build Rust libraries
        println!("Building Rust libraries for iOS...");
        self.build_rust_libraries(config)?;

        // Step 2: Generate UniFFI bindings
        println!("Generating UniFFI Swift bindings...");
        self.generate_uniffi_bindings()?;

        // Step 3: Create xcframework
        println!("Creating xcframework...");
        let xcframework_path = self.create_xcframework(config)?;

        // Step 4: Code-sign xcframework
        println!("Code-signing xcframework...");
        self.codesign_xcframework(&xcframework_path)?;

        // Copy header to include/ for consumers (handy for CLI uploads)
        let header_src = self
            .find_uniffi_header(&format!("{}FFI.h", framework_name))
            .ok_or_else(|| {
                BenchError::Build(format!(
                    "UniFFI header {}FFI.h not found after generation",
                    framework_name
                ))
            })?;
        let include_dir = self.output_dir.join("ios/include");
        fs::create_dir_all(&include_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create include dir at {}: {}. Check output directory permissions.",
                include_dir.display(),
                e
            ))
        })?;
        let header_dest = include_dir.join(format!("{}.h", framework_name));
        fs::copy(&header_src, &header_dest).map_err(|e| {
            BenchError::Build(format!(
                "Failed to copy UniFFI header to {:?}: {}. Check output directory permissions.",
                header_dest, e
            ))
        })?;

        // Step 5: Generate Xcode project if needed
        self.generate_xcode_project()?;

        Ok(BuildResult {
            platform: Target::Ios,
            app_path: xcframework_path,
            test_suite_path: None,
        })
    }

    /// Finds the benchmark crate directory (either bench-mobile/ or crates/{crate_name}/)
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

        let bench_mobile_manifest = bench_mobile_dir.join("Cargo.toml");
        let crates_manifest = crates_dir.join("Cargo.toml");
        Err(BenchError::Build(format!(
            "Benchmark crate '{}' not found.\n\n\
             Searched locations:\n\
             - {}\n\
             - {}\n\n\
             To fix this:\n\
             1. Create a bench-mobile/ directory with your benchmark crate, or\n\
             2. Use --crate-path to specify the benchmark crate location:\n\
                cargo mobench build --target ios --crate-path ./my-benchmarks\n\n\
             Common issues:\n\
             - Typo in crate name (check Cargo.toml [package] name)\n\
             - Wrong working directory (run from project root)\n\
             - Missing Cargo.toml in the crate directory\n\n\
             Run 'cargo mobench init-sdk --help' to generate a new benchmark project.",
            self.crate_name,
            bench_mobile_manifest.display(),
            crates_manifest.display()
        )))
    }

    /// Builds Rust libraries for iOS targets
    fn build_rust_libraries(&self, config: &BuildConfig) -> Result<(), BenchError> {
        let crate_dir = self.find_crate_dir()?;

        // iOS targets: device and simulator
        let targets = vec![
            "aarch64-apple-ios",     // Device (ARM64)
            "aarch64-apple-ios-sim", // Simulator (M1+ Macs)
        ];

        // Check if targets are installed
        self.check_rust_targets(&targets)?;
        let release_flag = if matches!(config.profile, BuildProfile::Release) {
            "--release"
        } else {
            ""
        };

        for target in targets {
            if self.verbose {
                println!("  Building for {}", target);
            }

            let mut cmd = Command::new("cargo");
            cmd.arg("build").arg("--target").arg(target).arg("--lib");

            // Add release flag if needed
            if !release_flag.is_empty() {
                cmd.arg(release_flag);
            }

            // Set working directory
            cmd.current_dir(&crate_dir);

            // Execute build
            let command_hint = if release_flag.is_empty() {
                format!("cargo build --target {} --lib", target)
            } else {
                format!("cargo build --target {} --lib {}", target, release_flag)
            };
            let output = cmd
                .output()
                .map_err(|e| BenchError::Build(format!(
                    "Failed to run cargo for {}.\n\n\
                     Command: {}\n\
                     Crate directory: {}\n\
                     Error: {}\n\n\
                     Tip: ensure cargo is installed and on PATH.",
                    target,
                    command_hint,
                    crate_dir.display(),
                    e
                )))?;

            if !output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(BenchError::Build(format!(
                    "cargo build failed for {}.\n\n\
                     Command: {}\n\
                     Crate directory: {}\n\
                     Exit status: {}\n\n\
                     Stdout:\n{}\n\n\
                     Stderr:\n{}\n\n\
                     Tips:\n\
                     - Ensure Xcode command line tools are installed (xcode-select --install)\n\
                     - Confirm Rust targets are installed (rustup target add {})",
                    target,
                    command_hint,
                    crate_dir.display(),
                    output.status,
                    stdout,
                    stderr,
                    target
                )));
            }
        }

        Ok(())
    }

    /// Checks if required Rust targets are installed
    fn check_rust_targets(&self, targets: &[&str]) -> Result<(), BenchError> {
        let output = Command::new("rustup")
            .arg("target")
            .arg("list")
            .arg("--installed")
            .output()
            .map_err(|e| {
                BenchError::Build(format!(
                    "Failed to check rustup targets: {}. Ensure rustup is installed and on PATH.",
                    e
                ))
            })?;

        let installed = String::from_utf8_lossy(&output.stdout);

        for target in targets {
            if !installed.contains(target) {
                return Err(BenchError::Build(format!(
                    "Rust target '{}' is not installed.\n\n\
                     This target is required to compile for iOS.\n\n\
                     To install:\n\
                       rustup target add {}\n\n\
                     For a complete iOS setup, you need both:\n\
                       rustup target add aarch64-apple-ios        # Device\n\
                       rustup target add aarch64-apple-ios-sim    # Simulator (Apple Silicon)",
                    target, target
                )));
            }
        }

        Ok(())
    }

    /// Generates UniFFI Swift bindings
    fn generate_uniffi_bindings(&self) -> Result<(), BenchError> {
        let crate_dir = self.find_crate_dir()?;
        let crate_name_underscored = self.crate_name.replace("-", "_");

        // Check if bindings already exist (for repository testing with pre-generated bindings)
        let bindings_path = self
            .output_dir
            .join("ios")
            .join("BenchRunner")
            .join("BenchRunner")
            .join("Generated")
            .join(format!("{}.swift", crate_name_underscored));

        if bindings_path.exists() {
            if self.verbose {
                println!("  Using existing Swift bindings at {:?}", bindings_path);
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
            .join("ios")
            .join("BenchRunner")
            .join("BenchRunner")
            .join("Generated");
        fs::create_dir_all(&out_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create Swift bindings dir at {}: {}. Check output directory permissions.",
                out_dir.display(),
                e
            ))
        })?;

        // Try cargo run first (works if crate has uniffi-bindgen binary target)
        let cargo_run_result = Command::new("cargo")
            .args(["run", "-p", &self.crate_name, "--bin", "uniffi-bindgen", "--"])
            .arg("generate")
            .arg("--library")
            .arg(&lib_path)
            .arg("--language")
            .arg("swift")
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
                .arg("swift")
                .arg("--out-dir")
                .arg(&out_dir);
            run_command(cmd, "uniffi-bindgen swift")?;
        }

        if self.verbose {
            println!("  Generated UniFFI Swift bindings at {:?}", out_dir);
        }

        Ok(())
    }

    /// Creates an xcframework from the built libraries
    fn create_xcframework(&self, config: &BuildConfig) -> Result<PathBuf, BenchError> {
        let profile_dir = match config.profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        };

        let crate_dir = self.find_crate_dir()?;
        let target_dir = get_cargo_target_dir(&crate_dir)?;
        let xcframework_dir = self.output_dir.join("ios");
        let framework_name = &self.crate_name.replace("-", "_");
        let xcframework_path = xcframework_dir.join(format!("{}.xcframework", framework_name));

        // Remove existing xcframework if it exists
        if xcframework_path.exists() {
            fs::remove_dir_all(&xcframework_path).map_err(|e| {
                BenchError::Build(format!(
                    "Failed to remove old xcframework at {}: {}. Close any tools using it and retry.",
                    xcframework_path.display(),
                    e
                ))
            })?;
        }

        // Create xcframework directory
        fs::create_dir_all(&xcframework_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create xcframework directory at {}: {}. Check output directory permissions.",
                xcframework_dir.display(),
                e
            ))
        })?;

        // Build framework structure for each platform
        self.create_framework_slice(
            &target_dir.join("aarch64-apple-ios").join(profile_dir),
            &xcframework_path.join("ios-arm64"),
            framework_name,
            "ios",
        )?;

        self.create_framework_slice(
            &target_dir.join("aarch64-apple-ios-sim").join(profile_dir),
            &xcframework_path.join("ios-simulator-arm64"),
            framework_name,
            "ios-simulator",
        )?;

        // Create xcframework Info.plist
        self.create_xcframework_plist(&xcframework_path, framework_name)?;

        Ok(xcframework_path)
    }

    /// Creates a framework slice for a specific platform
    fn create_framework_slice(
        &self,
        lib_path: &Path,
        output_dir: &Path,
        framework_name: &str,
        platform: &str,
    ) -> Result<(), BenchError> {
        let framework_dir = output_dir.join(format!("{}.framework", framework_name));
        let headers_dir = framework_dir.join("Headers");

        // Create directories
        fs::create_dir_all(&headers_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create framework directories at {}: {}. Check output directory permissions.",
                headers_dir.display(),
                e
            ))
        })?;

        // Copy static library
        let src_lib = lib_path.join(format!("lib{}.a", framework_name));
        let dest_lib = framework_dir.join(framework_name);

        if !src_lib.exists() {
            return Err(BenchError::Build(format!(
                "Static library not found at {}.\n\n\
                 Expected output from cargo build --target <target> --lib.\n\
                 Ensure your crate has [lib] crate-type = [\"staticlib\"].",
                src_lib.display()
            )));
        }

        fs::copy(&src_lib, &dest_lib).map_err(|e| {
            BenchError::Build(format!(
                "Failed to copy static library from {} to {}: {}. Check output directory permissions.",
                src_lib.display(),
                dest_lib.display(),
                e
            ))
        })?;

        // Copy UniFFI-generated header into the framework
        let header_name = format!("{}FFI.h", framework_name);
        let header_path = self.find_uniffi_header(&header_name).ok_or_else(|| {
            BenchError::Build(format!(
                "UniFFI header {} not found; run binding generation before building",
                header_name
            ))
        })?;
        let dest_header = headers_dir.join(&header_name);
        fs::copy(&header_path, &dest_header).map_err(|e| {
            BenchError::Build(format!(
                "Failed to copy UniFFI header from {} to {}: {}. Check output directory permissions.",
                header_path.display(),
                dest_header.display(),
                e
            ))
        })?;

        // Create module.modulemap
        let modulemap_content = format!(
            "framework module {} {{\n  umbrella header \"{}FFI.h\"\n  export *\n  module * {{ export * }}\n}}",
            framework_name, framework_name
        );
        let modulemap_path = headers_dir.join("module.modulemap");
        fs::write(&modulemap_path, modulemap_content).map_err(|e| {
            BenchError::Build(format!(
                "Failed to write module.modulemap at {}: {}. Check output directory permissions.",
                modulemap_path.display(),
                e
            ))
        })?;

        // Create framework Info.plist
        self.create_framework_plist(&framework_dir, framework_name, platform)?;

        Ok(())
    }

    /// Creates Info.plist for a framework slice
    fn create_framework_plist(
        &self,
        framework_dir: &Path,
        framework_name: &str,
        platform: &str,
    ) -> Result<(), BenchError> {
        let bundle_id = framework_name.replace('_', "-");
        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>{}</string>
    <key>CFBundleIdentifier</key>
    <string>dev.world.{}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>{}</string>
    <key>CFBundlePackageType</key>
    <string>FMWK</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>CFBundleSupportedPlatforms</key>
    <array>
        <string>{}</string>
    </array>
</dict>
</plist>"#,
            framework_name,
            bundle_id,
            framework_name,
            if platform == "ios" {
                "iPhoneOS"
            } else {
                "iPhoneSimulator"
            }
        );

        let plist_path = framework_dir.join("Info.plist");
        fs::write(&plist_path, plist_content).map_err(|e| {
            BenchError::Build(format!(
                "Failed to write framework Info.plist at {}: {}. Check output directory permissions.",
                plist_path.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Creates xcframework Info.plist
    fn create_xcframework_plist(
        &self,
        xcframework_path: &Path,
        framework_name: &str,
    ) -> Result<(), BenchError> {
        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>AvailableLibraries</key>
    <array>
        <dict>
            <key>LibraryIdentifier</key>
            <string>ios-arm64</string>
            <key>LibraryPath</key>
            <string>{}.framework</string>
            <key>SupportedArchitectures</key>
            <array>
                <string>arm64</string>
            </array>
            <key>SupportedPlatform</key>
            <string>ios</string>
        </dict>
        <dict>
            <key>LibraryIdentifier</key>
            <string>ios-simulator-arm64</string>
            <key>LibraryPath</key>
            <string>{}.framework</string>
            <key>SupportedArchitectures</key>
            <array>
                <string>arm64</string>
            </array>
            <key>SupportedPlatform</key>
            <string>ios</string>
            <key>SupportedPlatformVariant</key>
            <string>simulator</string>
        </dict>
    </array>
    <key>CFBundlePackageType</key>
    <string>XFWK</string>
    <key>XCFrameworkFormatVersion</key>
    <string>1.0</string>
</dict>
</plist>"#,
            framework_name, framework_name
        );

        let plist_path = xcframework_path.join("Info.plist");
        fs::write(&plist_path, plist_content).map_err(|e| {
            BenchError::Build(format!(
                "Failed to write xcframework Info.plist at {}: {}. Check output directory permissions.",
                plist_path.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Code-signs the xcframework
    fn codesign_xcframework(&self, xcframework_path: &Path) -> Result<(), BenchError> {
        let output = Command::new("codesign")
            .arg("--force")
            .arg("--deep")
            .arg("--sign")
            .arg("-")
            .arg(xcframework_path)
            .output();

        match output {
            Ok(output) if output.status.success() => {
                if self.verbose {
                    println!("  Successfully code-signed xcframework");
                }
                Ok(())
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                println!("Warning: Code signing failed: {}", stderr);
                println!("You may need to manually sign the xcframework");
                Ok(()) // Don't fail the build for signing issues
            }
            Err(e) => {
                println!("Warning: Could not run codesign: {}", e);
                println!("You may need to manually sign the xcframework");
                Ok(()) // Don't fail the build if codesign is not available
            }
        }
    }

    /// Generates Xcode project using xcodegen if project.yml exists
    fn generate_xcode_project(&self) -> Result<(), BenchError> {
        let ios_dir = self.output_dir.join("ios");
        let project_yml = ios_dir.join("BenchRunner/project.yml");

        if !project_yml.exists() {
            if self.verbose {
                println!("  No project.yml found, skipping xcodegen");
            }
            return Ok(());
        }

        if self.verbose {
            println!("  Generating Xcode project with xcodegen");
        }

        let project_dir = ios_dir.join("BenchRunner");
        let output = Command::new("xcodegen")
            .arg("generate")
            .current_dir(&project_dir)
            .output();

        match output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(BenchError::Build(format!(
                    "xcodegen failed.\n\n\
                     Command: xcodegen generate\n\
                     Working directory: {}\n\
                     Exit status: {}\n\n\
                     Stdout:\n{}\n\n\
                     Stderr:\n{}\n\n\
                     Tip: install xcodegen with: brew install xcodegen",
                    project_dir.display(),
                    output.status,
                    stdout,
                    stderr
                )))
            }
            Err(e) => {
                println!("Warning: xcodegen not found or failed: {}", e);
                println!("Install xcodegen with: brew install xcodegen");
                Ok(()) // Don't fail if xcodegen is not available
            }
        }
    }

    /// Locate the generated UniFFI header for the crate
    fn find_uniffi_header(&self, header_name: &str) -> Option<PathBuf> {
        // Check generated Swift bindings directory first
        let swift_dir = self
            .output_dir
            .join("ios/BenchRunner/BenchRunner/Generated");
        let candidate_swift = swift_dir.join(header_name);
        if candidate_swift.exists() {
            return Some(candidate_swift);
        }

        // Get the actual target directory (handles workspace case)
        let crate_dir = self.find_crate_dir().ok()?;
        let target_dir = get_cargo_target_dir(&crate_dir).ok()?;
        // Common UniFFI output location when using uniffi::generate_scaffolding
        let candidate = target_dir.join("uniffi").join(header_name);
        if candidate.exists() {
            return Some(candidate);
        }

        // Fallback: walk the target directory for the header
        let mut stack = vec![target_dir];
        while let Some(dir) = stack.pop() {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        // Limit depth by skipping non-target subtrees such as incremental caches
                        if let Some(name) = path.file_name().and_then(|n| n.to_str())
                            && name == "incremental"
                        {
                            continue;
                        }
                        stack.push(path);
                    } else if let Some(name) = path.file_name().and_then(|n| n.to_str())
                        && name == header_name
                    {
                        return Some(path);
                    }
                }
            }
        }

        None
    }
}

#[allow(clippy::collapsible_if)]
fn find_codesign_identity() -> Option<String> {
    let output = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut identities = Vec::new();
    for line in stdout.lines() {
        if let Some(start) = line.find('"') {
            if let Some(end) = line[start + 1..].find('"') {
                identities.push(line[start + 1..start + 1 + end].to_string());
            }
        }
    }
    let preferred = [
        "Apple Distribution",
        "iPhone Distribution",
        "Apple Development",
        "iPhone Developer",
    ];
    for label in preferred {
        if let Some(identity) = identities.iter().find(|i| i.contains(label)) {
            return Some(identity.clone());
        }
    }
    identities.first().cloned()
}

#[allow(clippy::collapsible_if)]
fn find_provisioning_profile() -> Option<PathBuf> {
    if let Ok(path) = env::var("MOBENCH_IOS_PROFILE") {
        let profile = PathBuf::from(path);
        if profile.exists() {
            return Some(profile);
        }
    }
    let home = env::var("HOME").ok()?;
    let profiles_dir = PathBuf::from(home).join("Library/MobileDevice/Provisioning Profiles");
    let entries = fs::read_dir(&profiles_dir).ok()?;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mobileprovision") {
            continue;
        }
        if let Ok(metadata) = entry.metadata()
            && let Ok(modified) = metadata.modified()
        {
            match &newest {
                Some((current, _)) if *current >= modified => {}
                _ => newest = Some((modified, path)),
            }
        }
    }
    newest.map(|(_, path)| path)
}

fn embed_provisioning_profile(app_path: &Path, profile: &Path) -> Result<(), BenchError> {
    let dest = app_path.join("embedded.mobileprovision");
    fs::copy(profile, &dest).map_err(|e| {
        BenchError::Build(format!(
            "Failed to embed provisioning profile at {:?}: {}. Check the profile path and file permissions.",
            dest, e
        ))
    })?;
    Ok(())
}

fn codesign_bundle(app_path: &Path, identity: &str) -> Result<(), BenchError> {
    let output = Command::new("codesign")
        .args(["--force", "--deep", "--sign", identity])
        .arg(app_path)
        .output()
        .map_err(|e| {
            BenchError::Build(format!(
                "Failed to run codesign: {}. Ensure Xcode command line tools are installed.",
                e
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BenchError::Build(format!(
            "codesign failed: {}. Verify you have a valid signing identity.",
            stderr
        )));
    }
    Ok(())
}

/// iOS code signing methods for IPA packaging
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningMethod {
    /// Ad-hoc signing (no Apple ID required, works for BrowserStack testing)
    AdHoc,
    /// Development signing (requires Apple Developer account and provisioning profile)
    Development,
}

impl IosBuilder {
    /// Packages the iOS app as an IPA file for distribution or testing
    ///
    /// This requires the app to have been built first with `build()`.
    /// The IPA can be used for:
    /// - BrowserStack device testing (ad-hoc signing)
    /// - Physical device testing (development signing)
    ///
    /// # Arguments
    ///
    /// * `scheme` - The Xcode scheme to build (e.g., "BenchRunner")
    /// * `method` - The signing method (AdHoc or Development)
    ///
    /// # Returns
    ///
    /// * `Ok(PathBuf)` - Path to the generated IPA file
    /// * `Err(BenchError)` - If the build or packaging fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// use mobench_sdk::builders::{IosBuilder, SigningMethod};
    ///
    /// let builder = IosBuilder::new(".", "bench-mobile");
    /// let ipa_path = builder.package_ipa("BenchRunner", SigningMethod::AdHoc)?;
    /// println!("IPA created at: {:?}", ipa_path);
    /// # Ok::<(), mobench_sdk::BenchError>(())
    /// ```
    pub fn package_ipa(&self, scheme: &str, method: SigningMethod) -> Result<PathBuf, BenchError> {
        // For repository structure: ios/BenchRunner/BenchRunner.xcodeproj
        // The directory and scheme happen to have the same name
        let ios_dir = self.output_dir.join("ios").join(scheme);
        let project_path = ios_dir.join(format!("{}.xcodeproj", scheme));

        // Verify Xcode project exists
        if !project_path.exists() {
            return Err(BenchError::Build(format!(
                "Xcode project not found at {}.\n\n\
                 Run `cargo mobench build --target ios` first or check --output-dir.",
                project_path.display()
            )));
        }

        let export_path = self.output_dir.join("ios");
        let ipa_path = export_path.join(format!("{}.ipa", scheme));

        // Create target/ios directory if it doesn't exist
        fs::create_dir_all(&export_path).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create export directory at {}: {}. Check output directory permissions.",
                export_path.display(),
                e
            ))
        })?;

        println!("Building {} for device...", scheme);

        // Step 1: Build the app for device (simpler than archiving)
        let build_dir = self.output_dir.join("ios/build");
        let build_configuration = "Debug";
        let mut cmd = Command::new("xcodebuild");
        cmd.args([
            "-project",
            project_path.to_str().unwrap(),
            "-scheme",
            scheme,
            "-destination",
            "generic/platform=iOS",
            "-configuration",
            build_configuration,
            "-derivedDataPath",
            build_dir.to_str().unwrap(),
            "build",
        ]);

        // Add signing parameters based on method
        match method {
            SigningMethod::AdHoc => {
                // Ad-hoc signing (works for BrowserStack, no Apple ID needed)
                // For ad-hoc, we disable signing during build and sign manually after
                cmd.args(["CODE_SIGNING_REQUIRED=NO", "CODE_SIGNING_ALLOWED=NO"]);
            }
            SigningMethod::Development => {
                // Development signing (requires Apple Developer account)
                cmd.args([
                    "CODE_SIGN_STYLE=Automatic",
                    "CODE_SIGN_IDENTITY=iPhone Developer",
                ]);
            }
        }

        if self.verbose {
            println!("  Running: {:?}", cmd);
        }

        // Run the build - may fail on validation but still produce the .app
        let build_result = cmd.output();

        // Step 2: Check if the .app bundle was created (even if validation failed)
        let app_path = build_dir
            .join(format!("Build/Products/{}-iphoneos", build_configuration))
            .join(format!("{}.app", scheme));

        if !app_path.exists() {
            match build_result {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(BenchError::Build(format!(
                        "xcodebuild build failed and app bundle was not created.\n\n\
                         Project: {}\n\
                         Scheme: {}\n\
                         Configuration: {}\n\
                         Derived data: {}\n\
                         Exit status: {}\n\n\
                         Stdout:\n{}\n\n\
                         Stderr:\n{}\n\n\
                         Tip: run xcodebuild manually to inspect the failure.",
                        project_path.display(),
                        scheme,
                        build_configuration,
                        build_dir.display(),
                        output.status,
                        stdout,
                        stderr
                    )));
                }
                Err(err) => {
                    return Err(BenchError::Build(format!(
                        "Failed to run xcodebuild: {}.\n\n\
                         App bundle not found at {}.\n\
                         Check that Xcode command line tools are installed.",
                        err,
                        app_path.display()
                    )));
                }
            }
        }

        if self.verbose {
            println!("  App bundle created successfully at {:?}", app_path);
        }

        if matches!(method, SigningMethod::AdHoc) {
            let profile = find_provisioning_profile();
            let identity = find_codesign_identity();
            match (profile.as_ref(), identity.as_ref()) {
                (Some(profile), Some(identity)) => {
                    embed_provisioning_profile(&app_path, profile)?;
                    codesign_bundle(&app_path, identity)?;
                    if self.verbose {
                        println!("  Signed app bundle with identity {}", identity);
                    }
                }
                _ => {
                    let output = Command::new("codesign")
                        .arg("--force")
                        .arg("--deep")
                        .arg("--sign")
                        .arg("-")
                        .arg(&app_path)
                        .output();
                    match output {
                        Ok(output) if output.status.success() => {
                            println!(
                                "Warning: Signed app bundle without provisioning profile; BrowserStack install may fail."
                            );
                        }
                        Ok(output) => {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            println!("Warning: Ad-hoc signing failed: {}", stderr);
                        }
                        Err(err) => {
                            println!("Warning: Could not run codesign: {}", err);
                        }
                    }
                }
            }
        }

        println!("Creating IPA from app bundle...");

        // Step 3: Create IPA (which is just a zip of Payload/{app})
        let payload_dir = export_path.join("Payload");
        if payload_dir.exists() {
            fs::remove_dir_all(&payload_dir).map_err(|e| {
                BenchError::Build(format!(
                    "Failed to remove old Payload dir at {}: {}. Close any tools using it and retry.",
                    payload_dir.display(),
                    e
                ))
            })?;
        }
        fs::create_dir_all(&payload_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create Payload dir at {}: {}. Check output directory permissions.",
                payload_dir.display(),
                e
            ))
        })?;

        // Copy app bundle into Payload/
        let dest_app = payload_dir.join(format!("{}.app", scheme));
        self.copy_dir_recursive(&app_path, &dest_app)?;

        // Create zip archive
        if ipa_path.exists() {
            fs::remove_file(&ipa_path).map_err(|e| {
                BenchError::Build(format!(
                    "Failed to remove old IPA at {}: {}. Check file permissions.",
                    ipa_path.display(),
                    e
                ))
            })?;
        }

        let mut cmd = Command::new("zip");
        cmd.args(["-qr", ipa_path.to_str().unwrap(), "Payload"])
            .current_dir(&export_path);

        if self.verbose {
            println!("  Running: {:?}", cmd);
        }

        run_command(cmd, "zip IPA")?;

        // Clean up Payload directory
        fs::remove_dir_all(&payload_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to clean up Payload dir at {}: {}. Check file permissions.",
                payload_dir.display(),
                e
            ))
        })?;

        println!("✓ IPA created: {:?}", ipa_path);
        Ok(ipa_path)
    }

    /// Packages the XCUITest runner app into a zip for BrowserStack.
    ///
    /// This requires the app project to be generated first with `build()`.
    /// The resulting zip can be supplied to BrowserStack as the test suite.
    pub fn package_xcuitest(&self, scheme: &str) -> Result<PathBuf, BenchError> {
        let ios_dir = self.output_dir.join("ios").join(scheme);
        let project_path = ios_dir.join(format!("{}.xcodeproj", scheme));

        if !project_path.exists() {
            return Err(BenchError::Build(format!(
                "Xcode project not found at {}.\n\n\
                 Run `cargo mobench build --target ios` first or check --output-dir.",
                project_path.display()
            )));
        }

        let export_path = self.output_dir.join("ios");
        fs::create_dir_all(&export_path).map_err(|e| {
            BenchError::Build(format!(
                "Failed to create export directory at {}: {}. Check output directory permissions.",
                export_path.display(),
                e
            ))
        })?;

        let build_dir = self.output_dir.join("ios/build");
        println!("Building XCUITest runner for {}...", scheme);

        let mut cmd = Command::new("xcodebuild");
        cmd.args([
            "build-for-testing",
            "-project",
            project_path.to_str().unwrap(),
            "-scheme",
            scheme,
            "-destination",
            "generic/platform=iOS",
            "-sdk",
            "iphoneos",
            "-configuration",
            "Release",
            "-derivedDataPath",
            build_dir.to_str().unwrap(),
            "VALIDATE_PRODUCT=NO",
            "CODE_SIGN_STYLE=Manual",
            "CODE_SIGN_IDENTITY=",
            "CODE_SIGNING_ALLOWED=NO",
            "CODE_SIGNING_REQUIRED=NO",
            "DEVELOPMENT_TEAM=",
            "PROVISIONING_PROFILE_SPECIFIER=",
            "ENABLE_BITCODE=NO",
            "BITCODE_GENERATION_MODE=none",
            "STRIP_BITCODE_FROM_COPIED_FILES=NO",
        ]);

        if self.verbose {
            println!("  Running: {:?}", cmd);
        }

        let runner_name = format!("{}UITests-Runner.app", scheme);
        let runner_path = build_dir
            .join("Build/Products/Release-iphoneos")
            .join(&runner_name);

        let build_result = cmd.output();
        let log_path = export_path.join("xcuitest-build.log");
        if let Ok(output) = &build_result
            && !output.status.success()
        {
            let mut log = String::new();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            log.push_str("STDOUT:\n");
            log.push_str(&stdout);
            log.push_str("\n\nSTDERR:\n");
            log.push_str(&stderr);
            let _ = fs::write(&log_path, log);
            println!("xcodebuild log written to {:?}", log_path);
            if runner_path.exists() {
                println!(
                    "Warning: xcodebuild build-for-testing failed, but runner exists: {}",
                    stderr
                );
            }
        }

        if !runner_path.exists() {
            match build_result {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(BenchError::Build(format!(
                        "xcodebuild build-for-testing failed and runner was not created.\n\n\
                         Project: {}\n\
                         Scheme: {}\n\
                         Derived data: {}\n\
                         Exit status: {}\n\
                         Log: {}\n\n\
                         Stdout:\n{}\n\n\
                         Stderr:\n{}\n\n\
                         Tip: open the log file above for more context.",
                        project_path.display(),
                        scheme,
                        build_dir.display(),
                        output.status,
                        log_path.display(),
                        stdout,
                        stderr
                    )));
                }
                Err(err) => {
                    return Err(BenchError::Build(format!(
                        "Failed to run xcodebuild: {}.\n\n\
                         XCUITest runner not found at {}.\n\
                         Check that Xcode command line tools are installed.",
                        err,
                        runner_path.display()
                    )));
                }
            }
        }

        let profile = find_provisioning_profile();
        let identity = find_codesign_identity();
        if let (Some(profile), Some(identity)) = (profile.as_ref(), identity.as_ref()) {
            embed_provisioning_profile(&runner_path, profile)?;
            codesign_bundle(&runner_path, identity)?;
            if self.verbose {
                println!("  Signed XCUITest runner with identity {}", identity);
            }
        } else {
            println!(
                "Warning: No provisioning profile/identity found; XCUITest runner may not install."
            );
        }

        let zip_path = export_path.join(format!("{}UITests.zip", scheme));
        if zip_path.exists() {
            fs::remove_file(&zip_path).map_err(|e| {
                BenchError::Build(format!(
                    "Failed to remove old zip at {}: {}. Check file permissions.",
                    zip_path.display(),
                    e
                ))
            })?;
        }

        let mut zip_cmd = Command::new("zip");
        zip_cmd
            .args(["-qr", zip_path.to_str().unwrap(), runner_name.as_str()])
            .current_dir(runner_path.parent().unwrap());

        if self.verbose {
            println!("  Running: {:?}", zip_cmd);
        }

        run_command(zip_cmd, "zip XCUITest runner")?;
        println!("✓ XCUITest runner packaged: {:?}", zip_path);

        Ok(zip_path)
    }

    /// Recursively copies a directory
    fn copy_dir_recursive(&self, src: &Path, dest: &Path) -> Result<(), BenchError> {
        fs::create_dir_all(dest).map_err(|e| {
            BenchError::Build(format!("Failed to create directory {:?}: {}", dest, e))
        })?;

        for entry in fs::read_dir(src)
            .map_err(|e| BenchError::Build(format!("Failed to read directory {:?}: {}", src, e)))?
        {
            let entry =
                entry.map_err(|e| BenchError::Build(format!("Failed to read entry: {}", e)))?;
            let path = entry.path();
            let file_name = path
                .file_name()
                .ok_or_else(|| BenchError::Build(format!("Invalid file name in {:?}", path)))?;
            let dest_path = dest.join(file_name);

            if path.is_dir() {
                self.copy_dir_recursive(&path, &dest_path)?;
            } else {
                fs::copy(&path, &dest_path).map_err(|e| {
                    BenchError::Build(format!(
                        "Failed to copy {:?} to {:?}: {}",
                        path, dest_path, e
                    ))
                })?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ios_builder_creation() {
        let builder = IosBuilder::new("/tmp/test-project", "test-bench-mobile");
        assert!(!builder.verbose);
        assert_eq!(
            builder.output_dir,
            PathBuf::from("/tmp/test-project/target/mobench")
        );
    }

    #[test]
    fn test_ios_builder_verbose() {
        let builder = IosBuilder::new("/tmp/test-project", "test-bench-mobile").verbose(true);
        assert!(builder.verbose);
    }

    #[test]
    fn test_ios_builder_custom_output_dir() {
        let builder =
            IosBuilder::new("/tmp/test-project", "test-bench-mobile").output_dir("/custom/output");
        assert_eq!(builder.output_dir, PathBuf::from("/custom/output"));
    }
}
