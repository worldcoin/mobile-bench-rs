//! iOS build automation
//!
//! This module provides functionality to build Rust libraries for iOS and
//! create an xcframework that can be used in Xcode projects.

use crate::types::{BenchError, BuildConfig, BuildProfile, BuildResult, Target};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// iOS builder that handles the complete build pipeline
pub struct IosBuilder {
    /// Root directory of the project
    project_root: PathBuf,
    /// Name of the bench-mobile crate
    crate_name: String,
    /// Whether to use verbose output
    verbose: bool,
}

impl IosBuilder {
    /// Creates a new iOS builder
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

    /// Builds the iOS app with the given configuration
    ///
    /// This performs the following steps:
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
        let include_dir = self.project_root.join("target/ios/include");
        fs::create_dir_all(&include_dir)
            .map_err(|e| BenchError::Build(format!("Failed to create include dir: {}", e)))?;
        let header_dest = include_dir.join(format!("{}.h", framework_name));
        fs::copy(&header_src, &header_dest).map_err(|e| {
            BenchError::Build(format!(
                "Failed to copy UniFFI header to {:?}: {}",
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

        for target in targets {
            if self.verbose {
                println!("  Building for {}", target);
            }

            let mut cmd = Command::new("cargo");
            cmd.arg("build").arg("--target").arg(target).arg("--lib");

            // Add release flag if needed
            if matches!(config.profile, BuildProfile::Release) {
                cmd.arg("--release");
            }

            // Set working directory
            cmd.current_dir(&crate_dir);

            // Execute build
            let output = cmd
                .output()
                .map_err(|e| BenchError::Build(format!("Failed to run cargo: {}", e)))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(BenchError::Build(format!(
                    "cargo build failed for {}: {}",
                    target, stderr
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
            .map_err(|e| BenchError::Build(format!("Failed to check rustup targets: {}", e)))?;

        let installed = String::from_utf8_lossy(&output.stdout);

        for target in targets {
            if !installed.contains(target) {
                return Err(BenchError::Build(format!(
                    "Rust target {} is not installed. Install it with: rustup target add {}",
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
            .project_root
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
            .join("ios")
            .join("BenchRunner")
            .join("BenchRunner")
            .join("Generated");
        fs::create_dir_all(&out_dir).map_err(|e| {
            BenchError::Build(format!("Failed to create Swift bindings dir: {}", e))
        })?;

        let mut cmd = Command::new("uniffi-bindgen");
        cmd.arg("generate")
            .arg("--library")
            .arg(&lib_path)
            .arg("--language")
            .arg("swift")
            .arg("--out-dir")
            .arg(&out_dir);
        run_command(cmd, "uniffi-bindgen swift")?;

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

        let target_dir = self.project_root.join("target");
        let xcframework_dir = target_dir.join("ios");
        let framework_name = &self.crate_name.replace("-", "_");
        let xcframework_path = xcframework_dir.join(format!("{}.xcframework", framework_name));

        // Remove existing xcframework if it exists
        if xcframework_path.exists() {
            fs::remove_dir_all(&xcframework_path).map_err(|e| {
                BenchError::Build(format!("Failed to remove old xcframework: {}", e))
            })?;
        }

        // Create xcframework directory
        fs::create_dir_all(&xcframework_dir).map_err(|e| {
            BenchError::Build(format!("Failed to create xcframework directory: {}", e))
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
            BenchError::Build(format!("Failed to create framework directories: {}", e))
        })?;

        // Copy static library
        let src_lib = lib_path.join(format!("lib{}.a", framework_name));
        let dest_lib = framework_dir.join(framework_name);

        if !src_lib.exists() {
            return Err(BenchError::Build(format!(
                "Static library not found: {:?}",
                src_lib
            )));
        }

        fs::copy(&src_lib, &dest_lib)
            .map_err(|e| BenchError::Build(format!("Failed to copy static library: {}", e)))?;

        // Copy UniFFI-generated header into the framework
        let header_name = format!("{}FFI.h", framework_name);
        let header_path = self.find_uniffi_header(&header_name).ok_or_else(|| {
            BenchError::Build(format!(
                "UniFFI header {} not found; run binding generation before building",
                header_name
            ))
        })?;
        fs::copy(&header_path, headers_dir.join(&header_name)).map_err(|e| {
            BenchError::Build(format!(
                "Failed to copy UniFFI header from {:?}: {}",
                header_path, e
            ))
        })?;

        // Create module.modulemap
        let modulemap_content = format!(
            "framework module {} {{\n  umbrella header \"{}FFI.h\"\n  export *\n  module * {{ export * }}\n}}",
            framework_name, framework_name
        );
        fs::write(headers_dir.join("module.modulemap"), modulemap_content)
            .map_err(|e| BenchError::Build(format!("Failed to write module.modulemap: {}", e)))?;

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
            framework_name,
            framework_name,
            if platform == "ios" {
                "iPhoneOS"
            } else {
                "iPhoneSimulator"
            }
        );

        fs::write(framework_dir.join("Info.plist"), plist_content).map_err(|e| {
            BenchError::Build(format!("Failed to write framework Info.plist: {}", e))
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

        fs::write(xcframework_path.join("Info.plist"), plist_content).map_err(|e| {
            BenchError::Build(format!("Failed to write xcframework Info.plist: {}", e))
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
        let ios_dir = self.project_root.join("ios");
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

        let output = Command::new("xcodegen")
            .arg("generate")
            .current_dir(ios_dir.join("BenchRunner"))
            .output();

        match output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(BenchError::Build(format!("xcodegen failed: {}", stderr)))
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
            .project_root
            .join("ios/BenchRunner/BenchRunner/Generated");
        let candidate_swift = swift_dir.join(header_name);
        if candidate_swift.exists() {
            return Some(candidate_swift);
        }

        let target_dir = self.project_root.join("target");
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

// Shared helpers (duplicated with android builder)
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

/// iOS code signing methods for IPA packaging
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningMethod {
    /// Ad-hoc signing (no Apple ID required, works for BrowserStack testing)
    AdHoc,
    /// Development signing (requires Apple Developer account and provisioning profile)
    Development,
}

impl SigningMethod {
    /// Returns the CODE_SIGN_IDENTITY value for xcodebuild
    fn identity(&self) -> &'static str {
        match self {
            SigningMethod::AdHoc => "-",
            SigningMethod::Development => "iPhone Developer",
        }
    }

    /// Returns the export method for ExportOptions.plist
    fn export_method(&self) -> &'static str {
        match self {
            SigningMethod::AdHoc => "ad-hoc",
            SigningMethod::Development => "development",
        }
    }
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
    pub fn package_ipa(
        &self,
        scheme: &str,
        method: SigningMethod,
    ) -> Result<PathBuf, BenchError> {
        // For repository structure: ios/BenchRunner/BenchRunner.xcodeproj
        // The directory and scheme happen to have the same name
        let ios_dir = self.project_root.join("ios").join(scheme);
        let project_path = ios_dir.join(format!("{}.xcodeproj", scheme));

        // Verify Xcode project exists
        if !project_path.exists() {
            return Err(BenchError::Build(format!(
                "Xcode project not found at {:?}. Run `cargo mobench build --target ios` first.",
                project_path
            )));
        }

        let export_path = self.project_root.join("target/ios");
        let ipa_path = export_path.join(format!("{}.ipa", scheme));

        // Create target/ios directory if it doesn't exist
        fs::create_dir_all(&export_path)
            .map_err(|e| BenchError::Build(format!("Failed to create export directory: {}", e)))?;

        println!("Building {} for device...", scheme);

        // Step 1: Build the app for device (simpler than archiving)
        let build_dir = self.project_root.join("target/ios/build");
        let mut cmd = Command::new("xcodebuild");
        cmd.args(&[
            "-project", project_path.to_str().unwrap(),
            "-scheme", scheme,
            "-destination", "generic/platform=iOS",
            "-configuration", "Release",
            "-derivedDataPath", build_dir.to_str().unwrap(),
            "build",
        ]);

        // Add signing parameters based on method
        match method {
            SigningMethod::AdHoc => {
                // Ad-hoc signing (works for BrowserStack, no Apple ID needed)
                // For ad-hoc, we disable signing during build and sign manually after
                cmd.args(&[
                    "CODE_SIGNING_REQUIRED=NO",
                    "CODE_SIGNING_ALLOWED=NO",
                ]);
            }
            SigningMethod::Development => {
                // Development signing (requires Apple Developer account)
                cmd.args(&[
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
            .join("Build/Products/Release-iphoneos")
            .join(format!("{}.app", scheme));

        if !app_path.exists() {
            // Only fail if the .app wasn't created
            if let Ok(output) = build_result {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(BenchError::Build(format!(
                    "xcodebuild build failed and app bundle not found: {}",
                    stderr
                )));
            } else {
                return Err(BenchError::Build(format!(
                    "App bundle not found at {:?}. Build may have failed.",
                    app_path
                )));
            }
        }

        if self.verbose {
            println!("  App bundle created successfully at {:?}", app_path);
        }

        println!("Creating IPA from app bundle...");

        // Step 3: Create IPA (which is just a zip of Payload/{app})
        let payload_dir = export_path.join("Payload");
        if payload_dir.exists() {
            fs::remove_dir_all(&payload_dir)
                .map_err(|e| BenchError::Build(format!("Failed to remove old Payload dir: {}", e)))?;
        }
        fs::create_dir_all(&payload_dir)
            .map_err(|e| BenchError::Build(format!("Failed to create Payload dir: {}", e)))?;

        // Copy app bundle into Payload/
        let dest_app = payload_dir.join(format!("{}.app", scheme));
        self.copy_dir_recursive(&app_path, &dest_app)?;

        // Create zip archive
        if ipa_path.exists() {
            fs::remove_file(&ipa_path)
                .map_err(|e| BenchError::Build(format!("Failed to remove old IPA: {}", e)))?;
        }

        let mut cmd = Command::new("zip");
        cmd.args(&["-qr", ipa_path.to_str().unwrap(), "Payload"])
            .current_dir(&export_path);

        if self.verbose {
            println!("  Running: {:?}", cmd);
        }

        run_command(cmd, "zip IPA")?;

        // Clean up Payload directory
        fs::remove_dir_all(&payload_dir)
            .map_err(|e| BenchError::Build(format!("Failed to clean up Payload dir: {}", e)))?;

        println!("✓ IPA created: {:?}", ipa_path);
        Ok(ipa_path)
    }

    /// Recursively copies a directory
    fn copy_dir_recursive(&self, src: &Path, dest: &Path) -> Result<(), BenchError> {
        fs::create_dir_all(dest)
            .map_err(|e| BenchError::Build(format!("Failed to create directory {:?}: {}", dest, e)))?;

        for entry in fs::read_dir(src)
            .map_err(|e| BenchError::Build(format!("Failed to read directory {:?}: {}", src, e)))?
        {
            let entry = entry.map_err(|e| BenchError::Build(format!("Failed to read entry: {}", e)))?;
            let path = entry.path();
            let file_name = path.file_name().ok_or_else(|| {
                BenchError::Build(format!("Invalid file name in {:?}", path))
            })?;
            let dest_path = dest.join(file_name);

            if path.is_dir() {
                self.copy_dir_recursive(&path, &dest_path)?;
            } else {
                fs::copy(&path, &dest_path).map_err(|e| {
                    BenchError::Build(format!("Failed to copy {:?} to {:?}: {}", path, dest_path, e))
                })?;
            }
        }

        Ok(())
    }

    /// Creates an ExportOptions.plist file for xcodebuild -exportArchive
    fn create_export_options_plist(
        &self,
        path: &Path,
        method: SigningMethod,
    ) -> Result<(), BenchError> {
        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>method</key>
    <string>{}</string>
    <key>compileBitcode</key>
    <false/>
    <key>stripSwiftSymbols</key>
    <true/>
    <key>uploadSymbols</key>
    <false/>
    <key>signingStyle</key>
    <string>automatic</string>
</dict>
</plist>
"#,
            method.export_method()
        );

        let mut file = fs::File::create(path).map_err(|e| {
            BenchError::Build(format!("Failed to create ExportOptions.plist: {}", e))
        })?;

        file.write_all(plist_content.as_bytes()).map_err(|e| {
            BenchError::Build(format!("Failed to write ExportOptions.plist: {}", e))
        })?;

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
    }

    #[test]
    fn test_ios_builder_verbose() {
        let builder = IosBuilder::new("/tmp/test-project", "test-bench-mobile").verbose(true);
        assert!(builder.verbose);
    }
}
