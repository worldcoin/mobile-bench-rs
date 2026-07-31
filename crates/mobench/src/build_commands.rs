//! Build, package, list, and verification command handlers.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use mobench_runtime::Distribution;
use serde_json::Value;

use crate::cli::{IosRunnerArg, IosSigningMethodArg, SdkTarget};
use crate::config;
use crate::project_layout::*;
use crate::reporting::json_value_to_u32;

/// Initialize a new benchmark project using `mobench-sdk`.
pub(crate) fn cmd_init_sdk(
    target: SdkTarget,
    project_name: String,
    output_dir: PathBuf,
    generate_examples: bool,
) -> Result<()> {
    println!("Initializing benchmark project with mobench-sdk...");
    println!("  Project name: {}", project_name);
    println!("  Target: {:?}", target);
    println!("  Output directory: {:?}", output_dir);

    let sdk_config = mobench_sdk::InitConfig {
        target: target.into(),
        project_name: project_name.clone(),
        output_dir: output_dir.clone(),
        generate_examples,
    };

    mobench_sdk::codegen::generate_project(&sdk_config).context("Failed to generate project")?;

    // Generate mobench.toml configuration file
    let mobench_toml_path = output_dir.join(config::CONFIG_FILE_NAME);
    if !mobench_toml_path.exists() {
        let toml_content = config::MobenchConfig::generate_starter_toml(&project_name);
        fs::write(&mobench_toml_path, toml_content)
            .with_context(|| format!("Failed to write {:?}", mobench_toml_path))?;
        println!("  Generated mobench.toml configuration file");
    }

    println!("\n[checkmark] Project initialized successfully!");
    println!("\nNext steps:");
    println!("  1. Add benchmark functions to your code with #[benchmark]");
    println!("  2. Edit mobench.toml to customize your project settings");
    println!("  3. Run 'cargo mobench build --target <platform>' to build");

    Ok(())
}

/// Build a browser-hosted WebAssembly benchmark bundle.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_build_web(
    release: bool,
    project_root: Option<PathBuf>,
    output_dir: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    dry_run: bool,
    verbose: bool,
    progress: bool,
) -> Result<()> {
    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let effective_output_dir = output_dir.unwrap_or_else(|| layout.output_dir.clone());
    let profile = if release {
        mobench_sdk::BuildProfile::Release
    } else {
        mobench_sdk::BuildProfile::Debug
    };
    let mut builder =
        mobench_sdk::builders::WebBuilder::new(&layout.project_root, layout.crate_name.clone())
            .library_name(layout.library_name.clone())
            .crate_dir(&layout.crate_dir)
            .output_dir(&effective_output_dir)
            .dry_run(dry_run)
            .verbose(verbose);
    if let Some(wasm_bindgen) = &layout.web_wasm_bindgen {
        builder = builder.wasm_bindgen(wasm_bindgen);
    }

    if progress {
        println!("[1/3] Compiling Rust benchmark to WebAssembly...");
        println!("[2/3] Generating browser bindings and harness...");
    } else {
        println!("Building web benchmark bundle...");
        println!("  Target: wasm32-unknown-unknown");
        println!("  Profile: {}", profile.as_str());
        println!("  Output: {}", effective_output_dir.join("web").display());
        if dry_run {
            println!("  Mode: dry-run (no changes will be made)");
        }
    }

    let result = builder.build(&mobench_sdk::builders::WebBuildConfig { profile })?;
    if progress {
        println!("[3/3] Done!");
    }
    if dry_run {
        println!("\n[dry-run] Web build simulation completed. No changes were made.");
    } else {
        println!("\n\u{2713} Web bundle: {}", result.bundle_dir.display());
        println!("  Entrypoint: {}", result.index_html.display());
        println!("  WebAssembly: {}", result.wasm.display());
        println!("  Manifest: {}", result.manifest.display());
    }
    Ok(())
}

/// Build mobile artifacts using `mobench-sdk`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_build(
    target: SdkTarget,
    release: bool,
    ios_completion_timeout_secs: Option<u64>,
    ios_deployment_target: Option<String>,
    ios_runner: Option<IosRunnerArg>,
    project_root: Option<PathBuf>,
    output_dir: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    dry_run: bool,
    verbose: bool,
    progress: bool,
) -> Result<()> {
    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let effective_output_dir = output_dir.unwrap_or_else(|| layout.output_dir.clone());
    let ios_completion_timeout_secs =
        configured_ios_completion_timeout_secs(&layout, ios_completion_timeout_secs);
    let (ios_deployment_target, ios_runner) = if matches!(target, SdkTarget::Ios | SdkTarget::Both)
    {
        let deployment_target =
            configured_ios_deployment_target(&layout, ios_deployment_target.as_deref())?;
        let runner_name = ios_runner.map(ios_runner_arg_name);
        let runner = configured_ios_runner(&layout, &deployment_target, runner_name)?;
        (deployment_target, runner)
    } else {
        (
            mobench_sdk::codegen::IosDeploymentTarget::default_target(),
            mobench_sdk::codegen::IosRunner::Swiftui,
        )
    };

    // Progress mode: simplified output
    if progress {
        let build_config = mobench_sdk::BuildConfig {
            target: target.into(),
            profile: if release {
                mobench_sdk::BuildProfile::Release
            } else {
                mobench_sdk::BuildProfile::Debug
            },
            incremental: true,
            android_abis: layout.android_abis.clone(),
        };

        match target {
            SdkTarget::Android => {
                println!("[1/3] Building Rust library...");
                let builder = android_builder_for_layout(&layout)
                    .verbose(false)
                    .dry_run(dry_run)
                    .output_dir(&effective_output_dir)
                    .crate_dir(&layout.crate_dir);
                println!("[2/3] Building Android APK...");
                let result = builder.build(&build_config)?;
                println!("[3/3] Done!");
                if !dry_run {
                    println!("\n\u{2713} APK: {:?}", result.app_path);
                }
            }
            SdkTarget::Ios => {
                println!("[1/3] Building Rust library...");
                let builder = ios_builder_for_layout(&layout)
                    .verbose(false)
                    .dry_run(dry_run)
                    .deployment_target(ios_deployment_target.clone())
                    .runner(Some(ios_runner))
                    .benchmark_timeout_secs(ios_completion_timeout_secs)
                    .output_dir(&effective_output_dir)
                    .crate_dir(&layout.crate_dir);
                println!("[2/3] Building iOS xcframework...");
                let result = builder.build(&build_config)?;
                println!("[3/3] Done!");
                if !dry_run {
                    println!("\n\u{2713} Framework: {:?}", result.app_path);
                }
            }
            SdkTarget::Both => {
                println!("[1/5] Building Rust library for Android...");
                let android_builder = android_builder_for_layout(&layout)
                    .verbose(false)
                    .dry_run(dry_run)
                    .output_dir(&effective_output_dir)
                    .crate_dir(&layout.crate_dir);
                println!("[2/5] Building Android APK...");
                let android_result = android_builder.build(&build_config)?;

                println!("[3/5] Building Rust library for iOS...");
                let ios_builder = ios_builder_for_layout(&layout)
                    .verbose(false)
                    .dry_run(dry_run)
                    .deployment_target(ios_deployment_target.clone())
                    .runner(Some(ios_runner))
                    .benchmark_timeout_secs(ios_completion_timeout_secs)
                    .output_dir(&effective_output_dir)
                    .crate_dir(&layout.crate_dir);
                println!("[4/5] Building iOS xcframework...");
                let ios_result = ios_builder.build(&build_config)?;

                println!("[5/5] Done!");
                if !dry_run {
                    println!("\n\u{2713} APK: {:?}", android_result.app_path);
                    println!("\u{2713} Framework: {:?}", ios_result.app_path);
                }
            }
        }
        return Ok(());
    }

    // Normal (verbose) mode
    if let Some(config_path) = &layout.config_path {
        println!("Using config file: {:?}", config_path);
    }

    println!("Building mobile artifacts...");
    println!("  Target: {:?}", target);
    println!("  Profile: {}", if release { "release" } else { "debug" });
    if dry_run {
        println!("  Mode: dry-run (no changes will be made)");
    }
    if verbose {
        println!("  Verbose: enabled");
    }

    println!("  Output: {:?}", effective_output_dir);
    println!("  Project root: {:?}", layout.project_root);
    println!("  Crate: {:?}", layout.crate_dir);

    let build_config = mobench_sdk::BuildConfig {
        target: target.into(),
        profile: if release {
            mobench_sdk::BuildProfile::Release
        } else {
            mobench_sdk::BuildProfile::Debug
        },
        incremental: true,
        android_abis: layout.android_abis.clone(),
    };

    match target {
        SdkTarget::Android => {
            println!("\nBuilding for Android...");
            println!("  Building Rust library for Android targets...");
            let builder = android_builder_for_layout(&layout)
                .verbose(verbose)
                .dry_run(dry_run)
                .output_dir(&effective_output_dir)
                .crate_dir(&layout.crate_dir);
            let result = builder.build(&build_config)?;
            if !dry_run {
                println!("\u{2713} Built Android APK");
                println!("\n[checkmark] Android build completed!");
                println!("  APK: {:?}", result.app_path);
            }
        }
        SdkTarget::Ios => {
            println!("\nBuilding for iOS...");
            println!("  Building Rust library for iOS targets...");
            let builder = ios_builder_for_layout(&layout)
                .verbose(verbose)
                .dry_run(dry_run)
                .deployment_target(ios_deployment_target.clone())
                .runner(Some(ios_runner))
                .benchmark_timeout_secs(ios_completion_timeout_secs)
                .output_dir(&effective_output_dir)
                .crate_dir(&layout.crate_dir);
            let result = builder.build(&build_config)?;
            if !dry_run {
                println!("\u{2713} Built iOS xcframework");
                println!("\n[checkmark] iOS build completed!");
                println!("  Framework: {:?}", result.app_path);
            }
        }
        SdkTarget::Both => {
            // Build Android
            println!("\nBuilding for Android...");
            println!("  Building Rust library for Android targets...");
            let android_builder = android_builder_for_layout(&layout)
                .verbose(verbose)
                .dry_run(dry_run)
                .output_dir(&effective_output_dir)
                .crate_dir(&layout.crate_dir);
            let android_result = android_builder.build(&build_config)?;
            if !dry_run {
                println!("\u{2713} Built Android APK");
                println!("\n[checkmark] Android build completed!");
                println!("  APK: {:?}", android_result.app_path);
            }

            // Build iOS
            println!("\nBuilding for iOS...");
            println!("  Building Rust library for iOS targets...");
            let ios_builder = ios_builder_for_layout(&layout)
                .verbose(verbose)
                .dry_run(dry_run)
                .deployment_target(ios_deployment_target)
                .runner(Some(ios_runner))
                .benchmark_timeout_secs(ios_completion_timeout_secs)
                .output_dir(&effective_output_dir)
                .crate_dir(&layout.crate_dir);
            let ios_result = ios_builder.build(&build_config)?;
            if !dry_run {
                println!("\u{2713} Built iOS xcframework");
                println!("\n[checkmark] iOS build completed!");
                println!("  Framework: {:?}", ios_result.app_path);
            }
        }
    }

    if dry_run {
        println!("\n[dry-run] Build simulation completed. No changes were made.");
    }

    Ok(())
}

/// List all discovered benchmark functions
///
/// This uses source code scanning to find `#[benchmark]` functions, which works
/// without requiring a full build. It also falls back to the inventory registry
/// for any benchmarks that may be registered at runtime.
pub(crate) fn cmd_list(project_root: Option<PathBuf>, crate_path: Option<PathBuf>) -> Result<()> {
    println!("Discovering benchmark functions...\n");

    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let mut all_benchmarks = discover_benchmarks_for_layout(&layout)?;

    // Method 2: Inventory registry (for runtime-registered benchmarks)
    let registry_benchmarks = mobench_sdk::discover_benchmarks();
    for bench in registry_benchmarks {
        let name = bench.name.to_string();
        if !all_benchmarks.contains(&name) {
            all_benchmarks.push(name);
        }
    }

    all_benchmarks.sort();

    if all_benchmarks.is_empty() {
        println!("No benchmarks found.\n");
        println!("Resolved crate: {}", layout.crate_dir.display());
        println!("\nTo add benchmarks:");
        println!("  1. Add #[benchmark] attribute to functions");
        println!("  2. Make sure mobench-sdk is in your dependencies");
        println!("  3. Run 'cargo mobench list' again");
    } else {
        println!("Found {} benchmark(s):", all_benchmarks.len());
        for bench in &all_benchmarks {
            println!("  {}", bench);
        }
        println!();
        println!("Usage:");
        println!(
            "  cargo mobench run --target android --function {} --iterations 100",
            all_benchmarks
                .first()
                .map(|s| s.as_str())
                .unwrap_or("my_benchmark")
        );
    }

    Ok(())
}

/// Package iOS app as IPA for distribution or testing
pub(crate) fn cmd_package_ipa(
    scheme: &str,
    method: IosSigningMethodArg,
    project_root: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    println!("Packaging iOS app as IPA...");
    println!("  Scheme: {}", scheme);
    println!("  Method: {:?}", method);
    if let Some(ref dir) = output_dir {
        println!("  Output: {:?}", dir);
    }

    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let effective_output_dir = output_dir.unwrap_or_else(|| layout.output_dir.clone());
    let ios_deployment_target = configured_ios_deployment_target(&layout, None)?;
    let ios_runner = configured_ios_runner(&layout, &ios_deployment_target, None)?;

    let builder = ios_builder_for_layout(&layout)
        .verbose(true)
        .deployment_target(ios_deployment_target)
        .runner(Some(ios_runner))
        .crate_dir(&layout.crate_dir)
        .output_dir(&effective_output_dir);

    let signing_method: mobench_sdk::builders::SigningMethod = method.into();
    let ipa_path = builder
        .package_ipa(scheme, signing_method)
        .context("Failed to package IPA")?;

    println!("\n[checkmark] IPA packaged successfully!");
    println!("  Path: {:?}", ipa_path);
    println!("\nYou can now:");
    println!("  - Install on device: Use Xcode or ios-deploy");
    println!(
        "  - Test on BrowserStack: cargo mobench run --target ios --ios-app {:?}",
        ipa_path
    );

    Ok(())
}

/// Package XCUITest runner for BrowserStack testing
pub(crate) fn cmd_package_xcuitest(
    scheme: &str,
    project_root: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    println!("Packaging XCUITest runner...");
    println!("  Scheme: {}", scheme);
    if let Some(ref dir) = output_dir {
        println!("  Output: {:?}", dir);
    }

    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let effective_output_dir = output_dir.unwrap_or_else(|| layout.output_dir.clone());
    let ios_deployment_target = configured_ios_deployment_target(&layout, None)?;
    let ios_runner = configured_ios_runner(&layout, &ios_deployment_target, None)?;

    let builder = ios_builder_for_layout(&layout)
        .verbose(true)
        .deployment_target(ios_deployment_target)
        .runner(Some(ios_runner))
        .crate_dir(&layout.crate_dir)
        .output_dir(&effective_output_dir);

    let zip_path = builder
        .package_xcuitest(scheme)
        .context("Failed to package XCUITest runner")?;

    println!("\n[checkmark] XCUITest runner packaged successfully!");
    println!("  Path: {:?}", zip_path);
    println!("\nYou can now:");
    println!(
        "  - Test on BrowserStack: cargo mobench run --target ios --ios-test-suite {:?}",
        zip_path
    );

    Ok(())
}

/// Verify benchmark setup: registry, spec, artifacts, and optional smoke test
#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_verify(
    project_root: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    target: Option<SdkTarget>,
    spec_path: Option<PathBuf>,
    check_artifacts: bool,
    smoke_test: bool,
    function: Option<String>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    println!("Verifying benchmark setup...\n");

    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let resolved_benchmarks = discover_benchmarks_for_layout(&layout)?;
    let effective_output_dir = output_dir.unwrap_or_else(|| layout.output_dir.clone());

    let mut checks_passed = 0;
    let mut checks_failed = 0;
    let mut warnings = 0;

    // 1. Check benchmark registry
    print!("  [1/4] Checking benchmark registry... ");
    let registry_benchmarks = mobench_sdk::discover_benchmarks();
    if resolved_benchmarks.is_empty() && registry_benchmarks.is_empty() {
        println!("WARNING");
        println!("        No benchmarks found in registry.");
        println!("        This may be expected if benchmarks are in a separate crate.");
        println!(
            "        Tip: Add #[benchmark] attribute to functions and ensure mobench-sdk is linked."
        );
        warnings += 1;
    } else {
        let total = resolved_benchmarks.len().max(registry_benchmarks.len());
        println!("OK ({} benchmark(s) found)", total);
        for bench in &resolved_benchmarks {
            println!("        - {}", bench);
        }
        if resolved_benchmarks.is_empty() {
            for bench in &registry_benchmarks {
                println!("        - {}", bench.name);
            }
        }
        checks_passed += 1;
    }

    // 2. Validate spec file if provided
    print!("  [2/4] Checking spec file... ");
    if let Some(ref path) = spec_path {
        match validate_spec_file(path) {
            Ok(spec) => {
                println!("OK");
                println!("        Function: {}", spec.name);
                println!("        Iterations: {}", spec.iterations);
                println!("        Warmup: {}", spec.warmup);
                checks_passed += 1;
            }
            Err(e) => {
                println!("FAILED");
                println!("        Error: {}", e);
                checks_failed += 1;
            }
        }
    } else {
        // Try default locations
        let default_paths = [
            effective_output_dir.join("android/app/src/main/assets/bench_spec.json"),
            effective_output_dir.join("ios/BenchRunner/BenchRunner/bench_spec.json"),
            layout
                .project_root
                .join("target/mobile-spec/android/bench_spec.json"),
            layout
                .project_root
                .join("target/mobile-spec/ios/bench_spec.json"),
        ];

        let mut found_any = false;
        for path in &default_paths {
            if path.exists() {
                if !found_any {
                    println!("OK (found at default locations)");
                    found_any = true;
                }
                match validate_spec_file(path) {
                    Ok(spec) => {
                        println!("        {:?}", path);
                        println!(
                            "          Function: {}, Iterations: {}, Warmup: {}",
                            spec.name, spec.iterations, spec.warmup
                        );
                    }
                    Err(e) => {
                        println!("        {:?} - INVALID: {}", path, e);
                    }
                }
            }
        }
        if found_any {
            checks_passed += 1;
        } else {
            println!("SKIPPED (no spec file found, use --spec-path to specify)");
            warnings += 1;
        }
    }

    // 3. Check artifacts if requested
    print!("  [3/4] Checking build artifacts... ");
    if check_artifacts {
        let mut artifacts_ok = true;
        let mut artifact_details = Vec::new();

        if let Some(ref t) = target {
            match t {
                SdkTarget::Android | SdkTarget::Both => {
                    let apk_path = effective_output_dir
                        .join("android/app/build/outputs/apk/debug/app-debug.apk");
                    let apk_release = effective_output_dir
                        .join("android/app/build/outputs/apk/release/app-release-unsigned.apk");
                    if apk_path.exists() {
                        artifact_details.push(format!("Android APK (debug): {:?}", apk_path));
                    } else if apk_release.exists() {
                        artifact_details.push(format!("Android APK (release): {:?}", apk_release));
                    } else {
                        artifact_details.push("Android APK: NOT FOUND".to_string());
                        artifacts_ok = false;
                    }

                    // Check JNI libs
                    let jni_base = effective_output_dir.join("android/app/src/main/jniLibs");
                    let abis = configured_android_abis(&layout);
                    for abi in abis {
                        let lib_path = jni_base
                            .join(&abi)
                            .join(format!("lib{}.so", layout.library_name));
                        if lib_path.exists() {
                            artifact_details.push(format!("JNI lib ({}): OK", abi));
                        }
                    }
                }
                SdkTarget::Ios => {}
            }

            match t {
                SdkTarget::Ios | SdkTarget::Both => {
                    let xcframework = effective_output_dir
                        .join("ios")
                        .join(format!("{}.xcframework", layout.library_name));
                    if xcframework.exists() {
                        artifact_details.push(format!("iOS xcframework: {:?}", xcframework));
                    } else {
                        artifact_details.push("iOS xcframework: NOT FOUND".to_string());
                        artifacts_ok = false;
                    }

                    let ipa_path = effective_output_dir.join("ios/BenchRunner.ipa");
                    if ipa_path.exists() {
                        artifact_details.push(format!("iOS IPA: {:?}", ipa_path));
                    }

                    let xcuitest_path = effective_output_dir.join("ios/BenchRunnerUITests.zip");
                    if xcuitest_path.exists() {
                        artifact_details.push(format!("XCUITest runner: {:?}", xcuitest_path));
                    }
                }
                SdkTarget::Android => {}
            }
        } else {
            // Check both platforms by default
            let android_apk =
                effective_output_dir.join("android/app/build/outputs/apk/debug/app-debug.apk");
            let ios_xcframework = effective_output_dir
                .join("ios")
                .join(format!("{}.xcframework", layout.library_name));

            if android_apk.exists() {
                artifact_details.push(format!("Android APK: {:?}", android_apk));
            }
            if ios_xcframework.exists() {
                artifact_details.push(format!("iOS xcframework: {:?}", ios_xcframework));
            }

            if artifact_details.is_empty() {
                artifacts_ok = false;
                artifact_details
                    .push("No artifacts found. Run 'cargo mobench build' first.".to_string());
            }
        }

        if artifacts_ok {
            println!("OK");
            checks_passed += 1;
        } else {
            println!("FAILED");
            checks_failed += 1;
        }
        for detail in &artifact_details {
            println!("        {}", detail);
        }
    } else {
        println!("SKIPPED (use --check-artifacts to enable)");
    }

    // 4. Run smoke test if requested
    print!("  [4/4] Running smoke test... ");
    if smoke_test {
        if let Err(err) = ensure_verify_smoke_test_supported(&layout) {
            println!("SKIPPED");
            println!("        {}", err);
            warnings += 1;
        } else if let Some(ref func) = function {
            match run_verify_smoke_test(func) {
                Ok(report) => {
                    println!("OK");
                    let samples = report.samples.len();
                    let mean_ns = verify_report_mean_ns(&report);
                    println!("        Function: {}", func);
                    println!("        Samples: {}", samples);
                    println!(
                        "        Mean: {} ns ({:.3} ms)",
                        mean_ns,
                        mean_ns as f64 / 1_000_000.0
                    );
                    checks_passed += 1;
                }
                Err(e) => {
                    println!("FAILED");
                    println!("        Error: {}", e);
                    checks_failed += 1;
                }
            }
        } else if let Some(func) = layout
            .default_function
            .as_ref()
            .or_else(|| resolved_benchmarks.first())
        {
            match run_verify_smoke_test(func) {
                Ok(report) => {
                    println!("OK");
                    let samples = report.samples.len();
                    let mean_ns = verify_report_mean_ns(&report);
                    println!("        Function: {} (auto-selected)", func);
                    println!("        Samples: {}", samples);
                    println!(
                        "        Mean: {} ns ({:.3} ms)",
                        mean_ns,
                        mean_ns as f64 / 1_000_000.0
                    );
                    checks_passed += 1;
                }
                Err(e) => {
                    println!("FAILED");
                    println!("        Error: {}", e);
                    checks_failed += 1;
                }
            }
        } else {
            println!("SKIPPED (no benchmark function available)");
            println!(
                "        Tip: Use --function to specify a function, or add benchmarks with #[benchmark]"
            );
            warnings += 1;
        }
    } else {
        println!("SKIPPED (use --smoke-test to enable)");
    }

    // Print summary
    println!("\n----------------------------------------");
    println!("Verification Summary:");
    println!("  Passed:   {}", checks_passed);
    println!("  Failed:   {}", checks_failed);
    println!("  Warnings: {}", warnings);

    if checks_failed > 0 {
        println!("\n[X] Verification failed with {} error(s)", checks_failed);
        bail!("Verification failed");
    } else if warnings > 0 {
        println!("\n[!] Verification completed with {} warning(s)", warnings);
    } else {
        println!("\n[checkmark] All checks passed!");
    }

    Ok(())
}

pub(crate) fn verify_report_mean_ns(report: &mobench_sdk::timing::BenchReport) -> u64 {
    let durations = report
        .samples
        .iter()
        .map(|sample| sample.duration_ns)
        .collect::<Vec<_>>();
    Distribution::from_slice(&durations)
        .cli_v1_summary()
        .map_or(0, |summary| summary.mean_ns)
}

/// Validate a bench_spec.json file
///
/// Handles both "name" and "function" field names for compatibility
/// with different spec file formats.
pub(crate) fn validate_spec_file(path: &Path) -> Result<mobench_sdk::BenchSpec> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading spec file {:?}", path))?;

    // Try parsing directly first (standard BenchSpec format with "name" field)
    if let Ok(spec) = serde_json::from_str::<mobench_sdk::BenchSpec>(&contents) {
        // Validate spec fields
        if spec.name.trim().is_empty() {
            bail!("spec.name is empty");
        }
        if spec.iterations == 0 {
            bail!("spec.iterations must be > 0");
        }
        return Ok(spec);
    }

    // Fall back to generic Value parsing for "function" field format
    // (used by persist_mobile_spec and some older formats)
    let value: Value =
        serde_json::from_str(&contents).with_context(|| format!("parsing spec file {:?}", path))?;

    // Extract name from either "name" or "function" field
    let name = value
        .get("name")
        .or_else(|| value.get("function"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("spec must have 'name' or 'function' field"))?
        .to_string();

    let iterations = value
        .get("iterations")
        .map(|value| {
            json_value_to_u32(value)
                .ok_or_else(|| anyhow!("spec.iterations must be an unsigned 32-bit integer"))
        })
        .transpose()?
        .unwrap_or(100);

    let warmup = value
        .get("warmup")
        .map(|value| {
            json_value_to_u32(value)
                .ok_or_else(|| anyhow!("spec.warmup must be an unsigned 32-bit integer"))
        })
        .transpose()?
        .unwrap_or(10);

    // Validate
    if name.trim().is_empty() {
        bail!("spec.name/function is empty");
    }
    if iterations == 0 {
        bail!("spec.iterations must be > 0");
    }

    mobench_sdk::BenchSpec::new(name, iterations, warmup).map_err(anyhow::Error::from)
}

/// Run a minimal smoke test for verification
pub(crate) fn run_verify_smoke_test(function: &str) -> Result<mobench_sdk::RunnerReport> {
    let spec = mobench_sdk::BenchSpec {
        name: function.to_string(),
        iterations: 3, // Minimal iterations for smoke test
        warmup: 1,
    };

    mobench_sdk::run_benchmark(spec).map_err(|e| anyhow!("smoke test failed: {}", e))
}
