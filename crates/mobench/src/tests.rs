use super::*;
use crate::devices::{builtin_device_for_profile, resolve_devices_from_matrix};

#[cfg(unix)]
pub(crate) use root_tests::write_fake_plot_python;

mod root_tests {
    use super::*;
    use clap::CommandFactory;
    use jsonschema::JSONSchema;
    use std::path::Path;
    use tempfile::TempDir;

    fn render_profile_run_help() -> String {
        let mut root = Cli::command();
        let profile = root
            .find_subcommand_mut("profile")
            .expect("profile subcommand");
        let run = profile
            .find_subcommand_mut("run")
            .expect("profile run subcommand");
        let mut buffer = Vec::new();
        run.write_long_help(&mut buffer)
            .expect("render profile run help");
        String::from_utf8(buffer).expect("help is utf-8")
    }

    #[cfg(unix)]
    pub(crate) fn write_fake_plot_python(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join("fake-python");
        std::fs::write(
            &path,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  exit 0
fi

output=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then
    shift
    output="$1"
  fi
  shift
done

mkdir -p "$(dirname "$output")"
printf '<svg>ok</svg>' > "$output"
"#,
        )
        .expect("write fake python");

        let mut permissions = std::fs::metadata(&path)
            .expect("fake python metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("set fake python perms");
        path
    }

    fn write_custom_layout_project(temp_dir: &TempDir) -> (PathBuf, PathBuf) {
        let project_root = temp_dir.path().to_path_buf();
        let crate_dir = project_root.join("crates/zk-mobile-bench");

        fs::create_dir_all(crate_dir.join("src")).expect("create custom crate dir");
        write_file(
            &project_root.join("Cargo.toml"),
            br#"[workspace]
members = ["crates/zk-mobile-bench"]
resolver = "2"
"#,
        )
        .expect("write workspace manifest");
        write_file(
            &project_root.join("mobench.toml"),
            br#"[project]
crate = "zk-mobile-bench"
library_name = "zk_mobile_bench"

[android]
abis = ["arm64-v8a", "x86_64"]

[benchmarks]
default_function = "zk_mobile_bench::bench_query_proof_generation"

[browserstack]
ios_completion_timeout_secs = 900
"#,
        )
        .expect("write mobench config");
        write_file(
            &crate_dir.join("Cargo.toml"),
            br#"[package]
name = "zk-mobile-bench"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("write custom crate manifest");
        write_file(
            &crate_dir.join("src/lib.rs"),
            br#"#[benchmark]
pub fn bench_query_proof_generation() {}
"#,
        )
        .expect("write custom crate source");

        (
            project_root
                .canonicalize()
                .expect("canonicalize project root"),
            crate_dir.canonicalize().expect("canonicalize crate dir"),
        )
    }

    // Register a lightweight benchmark for tests so the inventory contains at least one entry.
    #[mobench_sdk::benchmark]
    fn noop_benchmark() {
        std::hint::black_box(1u8);
    }

    #[test]
    fn resolves_cli_spec() {
        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .unwrap();
        let spec = resolve_run_spec(
            MobileTarget::Android,
            "sample_fns::fibonacci".into(),
            5,
            1,
            vec!["pixel".into()],
            &layout,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            false,
            false, // release
            false,
        )
        .unwrap();
        assert_eq!(spec.function, "sample_fns::fibonacci");
        assert_eq!(spec.iterations, 5);
        assert_eq!(spec.warmup, 1);
        assert_eq!(spec.devices, vec!["pixel".to_string()]);
        assert!(spec.browserstack.is_none());
        assert!(spec.ios_xcuitest.is_none());
    }

    #[test]
    fn resolve_run_spec_prefers_cli_device_matrix_with_config() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_matrix_path = temp_dir.path().join("config-matrix.yml");
        let cli_matrix_path = temp_dir.path().join("cli-matrix.yml");
        let config_path = temp_dir.path().join("bench-config.toml");

        write_file(
            &config_matrix_path,
            br#"devices:
  - name: Config Device
    os: android
    os_version: "14"
"#,
        )
        .expect("write config matrix");
        write_file(
            &cli_matrix_path,
            br#"devices:
  - name: CLI Device
    os: android
    os_version: "14"
"#,
        )
        .expect("write cli matrix");

        let config_toml = format!(
            r#"target = "android"
function = "sample_fns::fibonacci"
iterations = 10
warmup = 2
device_matrix = "{}"

[browserstack]
app_automate_username = "user"
app_automate_access_key = "key"
project = "proj"
"#,
            config_matrix_path.display()
        );
        write_file(&config_path, config_toml.as_bytes()).expect("write config");

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .unwrap();
        let spec = resolve_run_spec(
            MobileTarget::Android,
            "ignored::value".into(),
            1,
            0,
            Vec::new(),
            &layout,
            Some(config_path.as_path()),
            Some(cli_matrix_path.as_path()),
            Vec::new(),
            None,
            None,
            None,
            false,
            false,
            false,
        )
        .expect("resolve spec");

        assert_eq!(spec.devices, vec!["CLI Device".to_string()]);
    }

    #[test]
    fn parses_project_resolution_flags() {
        assert!(
            Cli::try_parse_from([
                "mobench",
                "run",
                "--target",
                "ios",
                "--function",
                "zk_mobile_bench::bench_query_proof_generation",
                "--crate-path",
                "/tmp/custom-crate",
                "--project-root",
                "/tmp/project-root",
            ])
            .is_ok()
        );

        assert!(
            Cli::try_parse_from([
                "mobench",
                "build",
                "--target",
                "ios",
                "--project-root",
                "/tmp/project-root",
            ])
            .is_ok()
        );

        assert!(
            Cli::try_parse_from([
                "mobench",
                "package-ipa",
                "--crate-path",
                "/tmp/custom-crate",
                "--project-root",
                "/tmp/project-root",
            ])
            .is_ok()
        );

        assert!(
            Cli::try_parse_from([
                "mobench",
                "package-xcuitest",
                "--crate-path",
                "/tmp/custom-crate",
                "--project-root",
                "/tmp/project-root",
            ])
            .is_ok()
        );

        assert!(
            Cli::try_parse_from([
                "mobench",
                "list",
                "--crate-path",
                "/tmp/custom-crate",
                "--project-root",
                "/tmp/project-root",
            ])
            .is_ok()
        );

        assert!(
            Cli::try_parse_from([
                "mobench",
                "verify",
                "--crate-path",
                "/tmp/custom-crate",
                "--project-root",
                "/tmp/project-root",
                "--smoke-test",
            ])
            .is_ok()
        );
    }

    #[test]
    fn resolver_uses_mobench_toml_for_custom_crate() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, crate_dir) = write_custom_layout_project(&temp_dir);

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve project layout");

        assert_eq!(layout.project_root, project_root);
        assert_eq!(layout.crate_dir, crate_dir);
        assert_eq!(layout.crate_name, "zk-mobile-bench");
        assert_eq!(layout.library_name, "zk_mobile_bench");
        assert_eq!(
            layout.android_abis,
            Some(vec!["arm64-v8a".to_string(), "x86_64".to_string()])
        );
        assert_eq!(layout.ios_completion_timeout_secs, Some(900));
        assert_eq!(
            layout.default_function.as_deref(),
            Some("zk_mobile_bench::bench_query_proof_generation")
        );
    }

    #[test]
    fn list_uses_resolved_layout_for_custom_crate() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve project layout");

        let benchmarks = discover_benchmarks_for_layout(&layout).expect("discover benchmarks");
        assert_eq!(
            benchmarks,
            vec!["zk_mobile_bench::bench_query_proof_generation".to_string()]
        );
    }

    #[test]
    fn verify_external_crate_smoke_test_is_unsupported() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve project layout");

        let err = ensure_verify_smoke_test_supported(&layout)
            .expect_err("external crate smoke tests should be unsupported");
        assert!(
            err.to_string().contains("external crate"),
            "unexpected error: {err}"
        );
        assert!(
            err.to_string().contains("unsupported"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn build_progress_uses_configured_crate() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);

        cmd_build(
            SdkTarget::Ios,
            false,
            None,
            Some(project_root),
            None,
            None,
            true,
            false,
            true,
        )
        .expect("build --progress should resolve config-driven crate");
    }

    #[test]
    fn verify_smoke_test_skips_external_crate() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);

        cmd_verify(
            Some(project_root),
            None,
            None,
            None,
            false,
            true,
            Some("zk_mobile_bench::bench_query_proof_generation".to_string()),
            None,
        )
        .expect("verify should clearly skip unsupported external smoke tests");
    }

    #[test]
    fn run_dry_run_prepares_ios_artifacts_inside_custom_project() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve layout");
        let spec = resolve_run_spec(
            MobileTarget::Ios,
            "zk_mobile_bench::bench_query_proof_generation".into(),
            1,
            0,
            vec!["iPhone 15".into()],
            &layout,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            false,
            false,
            true,
        )
        .expect("resolve dry-run spec");

        let ios_xcuitest = spec
            .ios_xcuitest
            .expect("dry-run should prepare placeholder iOS artifacts");
        assert_eq!(spec.ios_completion_timeout_secs, Some(900));
        assert!(
            ios_xcuitest.app.starts_with(&project_root),
            "app path should stay inside project root: {}",
            ios_xcuitest.app.display()
        );
        assert!(
            ios_xcuitest.test_suite.starts_with(&project_root),
            "test suite path should stay inside project root: {}",
            ios_xcuitest.test_suite.display()
        );
        assert!(
            ios_xcuitest
                .app
                .ends_with(Path::new("target/mobench/ios/BenchRunner.ipa"))
        );
        assert!(
            ios_xcuitest
                .test_suite
                .ends_with(Path::new("target/mobench/ios/BenchRunnerUITests.zip"))
        );
    }

    #[test]
    fn snapshot_baseline_creates_distinct_copy() {
        let temp_dir = TempDir::new().expect("temp dir");
        let baseline = temp_dir.path().join("baseline.json");
        write_file(&baseline, br#"{"ok":true}"#).expect("write baseline");

        assert!(paths_point_to_same_file(&baseline, &baseline).expect("compare path"));

        let snapshot = snapshot_baseline_for_compare(&baseline).expect("snapshot baseline");
        assert_ne!(snapshot, baseline);
        let original_contents = fs::read_to_string(&baseline).expect("read baseline");
        let snapshot_contents = fs::read_to_string(&snapshot).expect("read snapshot");
        assert_eq!(snapshot_contents, original_contents);

        fs::remove_file(snapshot).expect("remove snapshot");
    }

    #[test]
    fn local_smoke_produces_samples() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "noop_benchmark".into(),
            iterations: 3,
            warmup: 1,
            devices: vec![],
            ios_completion_timeout_secs: None,
            browserstack: None,
            ios_xcuitest: None,
        };
        let report = run_local_smoke(&spec).expect("local harness");
        assert!(report["samples"].is_array());
        assert_eq!(report["spec"]["name"], "noop_benchmark");
    }

    #[test]
    fn ios_defers_packaging_browserstack_artifacts_until_run_time() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);
        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve layout");
        let spec = resolve_run_spec(
            MobileTarget::Ios,
            "zk_mobile_bench::bench_query_proof_generation".into(),
            1,
            0,
            vec!["iPhone 15".into()],
            &layout,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            false,
            false, // release
            false,
        )
        .expect("should prepare iOS BrowserStack artifact paths");
        let ios_artifacts = spec
            .ios_xcuitest
            .expect("iOS artifact paths should be populated");
        assert_eq!(
            ios_artifacts.app,
            layout.output_dir.join("ios/BenchRunner.ipa")
        );
        assert!(
            ios_artifacts
                .test_suite
                .ends_with(Path::new("target/mobench/ios/BenchRunnerUITests.zip"))
        );
        assert!(
            !ios_artifacts.app.exists(),
            "iOS app artifact should not be packaged before the current bench_spec is persisted"
        );
        assert!(
            !ios_artifacts.test_suite.exists(),
            "iOS test suite should not be packaged before the current bench_spec is persisted"
        );
    }

    #[test]
    fn ios_managed_artifact_detection_accepts_config_template_paths() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);
        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve layout");

        let config_template_artifacts = IosXcuitestArtifacts {
            app: PathBuf::from("target/ios/BenchRunner.ipa"),
            test_suite: PathBuf::from("target/ios/BenchRunnerUITests.zip"),
        };

        assert!(
            uses_managed_ios_xcuitest_artifacts(&layout, &config_template_artifacts),
            "legacy config template paths should still be treated as mobench-managed artifacts"
        );
    }

    #[test]
    fn format_duration_smart_uses_milliseconds_by_default() {
        // 500 microseconds = 0.5 ms
        assert_eq!(format_duration_smart(500_000), "0.500ms");
        // 1.5 ms
        assert_eq!(format_duration_smart(1_500_000), "1.500ms");
        // 100 ms
        assert_eq!(format_duration_smart(100_000_000), "100.000ms");
        // 999.999 ms (just below threshold)
        assert_eq!(format_duration_smart(999_999_000), "999.999ms");
    }

    #[test]
    fn format_duration_smart_switches_to_seconds_when_large() {
        // Exactly 1 second
        assert_eq!(format_duration_smart(1_000_000_000), "1.000s");
        // 1.5 seconds
        assert_eq!(format_duration_smart(1_500_000_000), "1.500s");
        // 10 seconds
        assert_eq!(format_duration_smart(10_000_000_000), "10.000s");
    }

    #[test]
    fn format_ms_handles_optional_values() {
        assert_eq!(format_ms(Some(1_500_000)), "1.500ms");
        assert_eq!(format_ms(Some(1_500_000_000)), "1.500s");
        assert_eq!(format_ms(None), "-");
    }

    #[test]
    fn doctor_browserstack_defaults_to_true() {
        let cli = Cli::parse_from(["mobench", "doctor"]);
        match cli.command {
            Command::Doctor { browserstack, .. } => assert!(browserstack),
            _ => panic!("expected doctor command"),
        }
    }

    #[test]
    fn doctor_browserstack_can_be_disabled() {
        let cli = Cli::parse_from(["mobench", "doctor", "--browserstack=false"]);
        match cli.command {
            Command::Doctor { browserstack, .. } => assert!(!browserstack),
            _ => panic!("expected doctor command"),
        }
    }

    #[test]
    fn doctor_android_prereqs_default_to_arm64_only() {
        assert_eq!(
            DEFAULT_ANDROID_DOCTOR_RUST_TARGETS,
            &["aarch64-linux-android"]
        );
    }

    #[test]
    fn rustc_msrv_parser_handles_stable_and_prerelease_versions() {
        assert_eq!(
            parse_rust_version("rustc 1.95.0 (59807616e 2026-04-14)"),
            Some((1, 95, 0))
        );
        assert_eq!(
            parse_rust_version("rustc 1.85.0-beta.1 (example)"),
            Some((1, 85, 0))
        );
        assert!(rustc_version_meets_msrv("rustc 1.85.0", WORKSPACE_MSRV));
        assert!(rustc_version_meets_msrv("rustc 1.95.0", WORKSPACE_MSRV));
        assert!(!rustc_version_meets_msrv("rustc 1.84.1", WORKSPACE_MSRV));
    }

    #[test]
    fn ci_run_parses_required_args_with_defaults() {
        let cli = Cli::parse_from([
            "mobench",
            "ci",
            "run",
            "--target",
            "android",
            "--function",
            "sample_fns::fibonacci",
        ]);

        match cli.command {
            Command::Ci {
                command: CiCommand::Run(args),
            } => {
                assert_eq!(args.target, CiTarget::Android);
                assert_eq!(args.function.as_deref(), Some("sample_fns::fibonacci"));
                assert_eq!(args.output_dir, PathBuf::from("target/mobench/ci"));
            }
            _ => panic!("expected ci run command"),
        }
    }

    #[test]
    fn ci_run_parses_both_target() {
        let cli = Cli::parse_from([
            "mobench",
            "ci",
            "run",
            "--target",
            "both",
            "--function",
            "sample_fns::fibonacci",
        ]);

        match cli.command {
            Command::Ci {
                command: CiCommand::Run(args),
            } => {
                assert_eq!(args.target, CiTarget::Both);
            }
            _ => panic!("expected ci run command"),
        }
    }

    #[test]
    fn ci_run_parses_ios_completion_timeout_secs() {
        let cli = Cli::parse_from([
            "mobench",
            "ci",
            "run",
            "--target",
            "ios",
            "--function",
            "sample_fns::fibonacci",
            "--ios-completion-timeout-secs",
            "900",
        ]);

        match cli.command {
            Command::Ci {
                command: CiCommand::Run(args),
            } => {
                assert_eq!(args.target, CiTarget::Ios);
                assert_eq!(args.ios_completion_timeout_secs, Some(900));
            }
            _ => panic!("expected ci run command"),
        }
    }

    #[test]
    fn build_parses_ios_completion_timeout_secs() {
        let cli = Cli::parse_from([
            "mobench",
            "build",
            "--target",
            "ios",
            "--ios-completion-timeout-secs",
            "750",
        ]);

        match cli.command {
            Command::Build {
                ios_completion_timeout_secs,
                ..
            } => {
                assert_eq!(ios_completion_timeout_secs, Some(750));
            }
            _ => panic!("expected build command"),
        }
    }

    #[test]
    fn resolve_run_spec_reads_ios_completion_timeout_from_config() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("bench-config.toml");

        let config_toml = r#"target = "ios"
function = "sample_fns::fibonacci"
iterations = 10
warmup = 2
device_matrix = "device-matrix.yaml"

[browserstack]
app_automate_username = "user"
app_automate_access_key = "key"
project = "proj"
ios_completion_timeout_secs = 900

[ios_xcuitest]
app = "target/ios/BenchRunner.ipa"
test_suite = "target/ios/BenchRunnerUITests.zip"
"#;
        write_file(&config_path, config_toml.as_bytes()).expect("write config");
        write_file(
            &temp_dir.path().join("device-matrix.yaml"),
            br#"devices:
  - name: iPhone 16 Pro
    os: ios
    os_version: "18"
"#,
        )
        .expect("write matrix");

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .unwrap();
        let spec = resolve_run_spec(
            MobileTarget::Ios,
            "ignored::value".into(),
            1,
            0,
            Vec::new(),
            &layout,
            Some(config_path.as_path()),
            None,
            Vec::new(),
            None,
            None,
            Some(600),
            false,
            false,
            false,
        )
        .expect("resolve spec");

        assert_eq!(spec.ios_completion_timeout_secs, Some(600));
        assert_eq!(
            spec.browserstack
                .as_ref()
                .and_then(|cfg| cfg.ios_completion_timeout_secs),
            Some(900)
        );
    }

    #[test]
    fn devices_resolve_parses() {
        let cli = Cli::parse_from([
            "mobench",
            "devices",
            "resolve",
            "--platform",
            "android",
            "--profile",
            "default",
            "--device-matrix",
            "device-matrix.yaml",
        ]);
        match cli.command {
            Command::Devices {
                command:
                    Some(DevicesCommand::Resolve {
                        platform, profile, ..
                    }),
                ..
            } => {
                assert_eq!(platform, DevicePlatform::Android);
                assert_eq!(profile, Some("default".to_string()));
            }
            _ => panic!("expected devices resolve command"),
        }
    }

    #[test]
    fn fixture_cache_key_parses() {
        let cli = Cli::parse_from(["mobench", "fixture", "cache-key"]);
        match cli.command {
            Command::Fixture {
                command:
                    FixtureCommand::CacheKey {
                        config,
                        target,
                        format,
                        ..
                    },
            } => {
                assert_eq!(config, PathBuf::from("bench-config.toml"));
                assert_eq!(target, SdkTarget::Both);
                assert_eq!(format, CheckOutputFormat::Text);
            }
            _ => panic!("expected fixture cache-key command"),
        }
    }

    #[test]
    fn profile_run_parses_with_android_backend() {
        let cli = Cli::parse_from([
            "mobench",
            "profile",
            "run",
            "--target",
            "android",
            "--function",
            "sample_fns::fibonacci",
            "--backend",
            "android-native",
        ]);

        match cli.command {
            Command::Profile {
                command: ProfileCommand::Run(args),
            } => {
                assert_eq!(args.target, MobileTarget::Android);
                assert_eq!(args.function, "sample_fns::fibonacci");
                assert_eq!(args.backend, profile::ProfileBackend::AndroidNative);
            }
            _ => panic!("expected profile run command"),
        }
    }

    #[test]
    fn profile_run_parses_direct_device_selection() {
        let cli = Cli::parse_from([
            "mobench",
            "profile",
            "run",
            "--target",
            "ios",
            "--function",
            "sample_fns::fibonacci",
            "--provider",
            "browserstack",
            "--backend",
            "ios-instruments",
            "--device",
            "iPhone 14",
            "--os-version",
            "16",
        ]);

        match cli.command {
            Command::Profile {
                command: ProfileCommand::Run(args),
            } => {
                assert_eq!(args.target, MobileTarget::Ios);
                assert_eq!(args.device.as_deref(), Some("iPhone 14"));
                assert_eq!(args.os_version.as_deref(), Some("16"));
            }
            _ => panic!("expected profile run command"),
        }
    }

    #[test]
    fn profile_run_parses_profile_device_resolution_inputs() {
        let cli = Cli::parse_from([
            "mobench",
            "profile",
            "run",
            "--target",
            "ios",
            "--function",
            "sample_fns::fibonacci",
            "--provider",
            "browserstack",
            "--backend",
            "ios-instruments",
            "--profile",
            "high-spec",
            "--device-matrix",
            "device-matrix.yaml",
        ]);

        match cli.command {
            Command::Profile {
                command: ProfileCommand::Run(args),
            } => {
                assert_eq!(args.profile.as_deref(), Some("high-spec"));
                assert_eq!(
                    args.device_matrix,
                    Some(PathBuf::from("device-matrix.yaml"))
                );
            }
            _ => panic!("expected profile run command"),
        }
    }

    #[test]
    fn profile_run_parses_capture_warmup_mode() {
        let cli = Cli::parse_from([
            "mobench",
            "profile",
            "run",
            "--target",
            "android",
            "--function",
            "sample_fns::fibonacci",
            "--warmup-mode",
            "cold",
        ]);

        match cli.command {
            Command::Profile {
                command: ProfileCommand::Run(args),
            } => {
                assert_eq!(args.warmup_mode, Some(profile::CaptureWarmupMode::Cold));
            }
            _ => panic!("expected profile run command"),
        }
    }

    #[test]
    fn profile_run_help_mentions_planned_only_or_execution_scope() {
        let help = render_profile_run_help();

        assert!(
            help.contains("Plan or execute a native profiling session; local android-native and ios-instruments now attempt real native capture"),
            "expected profile run help to describe the real local Android/iOS execution scope, got:\n{help}"
        );
        assert!(
            help.contains(
                "local + android-native: attempts real simpleperf capture and symbolization"
            ),
            "expected profile run help to mention real Android native execution, got:\n{help}"
        );
        assert!(
            help.contains(
                "local + ios-instruments: attempts real simulator-host sample capture and flamegraph generation"
            ),
            "expected profile run help to mention real local iOS sample capture, got:\n{help}"
        );
        assert!(
            help.contains("--warmup-mode"),
            "expected profile run help to expose warm/cold profiling mode, got:\n{help}"
        );
    }

    #[test]
    fn profile_run_cli_surface_exposes_or_explicitly_omits_device_selection() {
        let help = render_profile_run_help();

        assert!(
            help.contains("--device")
                || help.contains("--profile")
                || help.contains("--device-matrix")
                || help.contains("device selection is unavailable"),
            "expected profile run help to either expose device selection or explicitly document that it is unavailable, got:\n{help}"
        );
    }

    #[test]
    fn profile_summarize_parses_with_default_profile_path() {
        let cli = Cli::parse_from(["mobench", "profile", "summarize"]);

        match cli.command {
            Command::Profile {
                command: ProfileCommand::Summarize(args),
            } => {
                assert_eq!(
                    args.profile,
                    PathBuf::from("target/mobench/profile/profile.json")
                );
                assert_eq!(args.output_format, profile::ProfileSummaryFormat::Markdown);
            }
            _ => panic!("expected profile summarize command"),
        }
    }

    #[test]
    fn report_github_parses() {
        let cli = Cli::parse_from(["mobench", "report", "github", "--pr", "123"]);
        match cli.command {
            Command::Report {
                command: ReportCommand::Github { pr, publish, .. },
            } => {
                assert_eq!(pr, Some("123".to_string()));
                assert!(!publish);
            }
            _ => panic!("expected report github command"),
        }
    }

    #[test]
    fn config_validate_parses_required_args_with_defaults() {
        let cli = Cli::parse_from(["mobench", "config", "validate"]);
        match cli.command {
            Command::Config {
                command: ConfigCommand::Validate { config, format },
            } => {
                assert_eq!(config, PathBuf::from("bench-config.toml"));
                assert_eq!(format, CheckOutputFormat::Text);
            }
            _ => panic!("expected config validate command"),
        }
    }

    #[test]
    fn issue_categories_align_with_contract_taxonomy() {
        let checks = vec![
            PrereqCheck {
                name: "Run config".to_string(),
                passed: false,
                detail: Some("missing".to_string()),
                fix_hint: Some("fix config".to_string()),
            },
            PrereqCheck {
                name: "BrowserStack credentials".to_string(),
                passed: false,
                detail: Some("missing".to_string()),
                fix_hint: Some("set env".to_string()),
            },
            PrereqCheck {
                name: "cargo installed".to_string(),
                passed: false,
                detail: None,
                fix_hint: Some("install rust".to_string()),
            },
        ];
        let issues = collect_issues(&checks);
        assert_eq!(issues.len(), 3);
        assert_eq!(category_slug(issues[0].category), "config_error");
        assert_eq!(category_slug(issues[1].category), "provider_error");
        assert_eq!(category_slug(issues[2].category), "preflight_error");
    }

    #[test]
    fn check_results_json_includes_issue_categories() {
        let checks = vec![PrereqCheck {
            name: "Run config".to_string(),
            passed: false,
            detail: Some("missing".to_string()),
            fix_hint: Some("fix config".to_string()),
        }];
        let issues = collect_issues(&checks);
        let rendered = render_check_results_json(&checks, &issues);
        let category = rendered
            .get("issues")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|first| first.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(category, Some("config"));
    }

    #[test]
    fn resolve_devices_from_matrix_is_deterministic() {
        let devices = vec![
            DeviceEntry {
                name: "Pixel 7".to_string(),
                os: "android".to_string(),
                os_version: "13.0".to_string(),
                tags: Some(vec!["default".to_string(), "pixel".to_string()]),
            },
            DeviceEntry {
                name: "Pixel 6".to_string(),
                os: "android".to_string(),
                os_version: "12.0".to_string(),
                tags: Some(vec!["default".to_string()]),
            },
            DeviceEntry {
                name: "iPhone 14".to_string(),
                os: "ios".to_string(),
                os_version: "16".to_string(),
                tags: Some(vec!["default".to_string(), "iphone".to_string()]),
            },
        ];

        let resolved =
            resolve_devices_from_matrix(devices, DevicePlatform::Android, &["default".to_string()])
                .expect("resolved devices");
        let ids: Vec<String> = resolved.into_iter().map(|d| d.identifier).collect();
        assert_eq!(ids, vec!["Pixel 6-12.0", "Pixel 7-13.0"]);
    }

    #[test]
    fn builtin_ios_low_spec_profile_uses_iphone_se_2020() {
        let resolved = builtin_device_for_profile(DevicePlatform::Ios, "low-spec")
            .expect("built-in low-spec iOS profile");

        assert_eq!(resolved.name, "iPhone SE 2020");
        assert_eq!(resolved.os_version, "16");
        assert_eq!(resolved.identifier, "iPhone SE 2020-16");
    }

    #[test]
    fn builtin_android_low_spec_profile_uses_moto_g9_play() {
        let resolved = builtin_device_for_profile(DevicePlatform::Android, "low-spec")
            .expect("built-in low-spec Android profile");

        assert_eq!(resolved.name, "Motorola Moto G9 Play");
        assert_eq!(resolved.os_version, "10.0");
        assert_eq!(resolved.identifier, "Motorola Moto G9 Play-10.0");
    }

    #[test]
    fn render_summary_markdown_from_merged_output() {
        let summary = json!({
            "generated_at": "2026-02-16T00:00:00Z",
            "generated_at_unix": 1708041600,
            "target": "android",
            "function": "noop_benchmark",
            "iterations": 3,
            "warmup": 1,
            "devices": ["local"],
            "device_summaries": []
        });
        let merged = json!({
            "targets": {
                "android": { "summary": summary },
                "ios": { "summary": {
                    "generated_at": "2026-02-16T00:00:00Z",
                    "generated_at_unix": 1708041600,
                    "target": "ios",
                    "function": "noop_benchmark",
                    "iterations": 3,
                    "warmup": 1,
                    "devices": ["local"],
                    "device_summaries": []
                }}
            }
        });
        let markdown = render_summary_markdown_from_output(&merged).expect("render markdown");
        assert!(markdown.contains("## android"));
        assert!(markdown.contains("## ios"));
    }

    #[test]
    fn compare_markdown_includes_delta_labels() {
        let report = CompareReport {
            baseline: PathBuf::from("baseline.json"),
            candidate: PathBuf::from("candidate.json"),
            rows: vec![CompareRow {
                device: "Pixel 7".to_string(),
                function: "noop_benchmark".to_string(),
                baseline_median_ns: Some(100),
                candidate_median_ns: Some(110),
                median_delta_pct: Some(10.0),
                median_label: "regressed".to_string(),
                baseline_p95_ns: Some(120),
                candidate_p95_ns: Some(118),
                p95_delta_pct: Some(-1.66),
                p95_label: "improved".to_string(),
            }],
        };
        let markdown = render_compare_markdown(&report);
        assert!(markdown.starts_with("### Benchmark Comparison\n"));
        assert!(markdown.contains("Median base"));
        assert!(markdown.contains("Median cand"));
        assert!(markdown.contains("P95 base"));
        assert!(markdown.contains("P95 cand"));
        assert!(!markdown.contains("Median (base ms)"));
        assert!(!markdown.contains("Median (cand ms)"));
        assert!(!markdown.contains("P95 (base ms)"));
        assert!(!markdown.contains("P95 (cand ms)"));
        assert!(markdown.contains("Median Label"));
        assert!(markdown.contains("P95 Label"));
        assert!(markdown.contains("regressed"));
        assert!(markdown.contains("improved"));
    }

    #[test]
    fn render_markdown_summary_includes_resource_usage_columns_when_present() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Google Pixel 8-14.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(482),
                        cpu_median_ms: Some(241),
                        peak_memory_kb: Some(249_416),
                        peak_memory_growth_kb: Some(249_416),
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                }],
            }],
        });

        assert!(markdown.contains(
            "| Device | Function | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak |"
        ));
        assert!(markdown.contains("1.250s"));
        assert!(markdown.contains("6.250s"));
        assert!(markdown.contains("241ms"));
        assert!(markdown.contains("482ms"));
        assert!(markdown.contains("7.7%"));
        assert!(markdown.contains("243.57 MB"));
    }

    #[test]
    fn render_markdown_summary_uses_explicit_wall_and_cpu_columns() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 4,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Google Pixel 8-14.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 4,
                    mean_ns: Some(1_000_000_000),
                    median_ns: Some(950_000_000),
                    p95_ns: Some(1_100_000_000),
                    min_ns: Some(900_000_000),
                    max_ns: Some(1_200_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(800),
                        cpu_median_ms: Some(200),
                        peak_memory_kb: Some(1_024),
                        peak_memory_growth_kb: Some(1_024),
                        process_peak_memory_kb: None,
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                }],
            }],
        });

        assert!(markdown.contains(
            "| Device | Function | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak |"
        ));
        assert!(markdown.contains(
            "| Google Pixel 8-14.0 | sample_fns::fibonacci | 4 | 1 | 1.000s | 4.000s | 200ms | 800ms | 20.0% | 1.00 MB | - |"
        ));
        assert!(!markdown.contains("### Device:"));
    }

    #[test]
    fn render_csv_summary_includes_resource_usage_columns() {
        let csv = render_csv_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Google Pixel 8-14.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(482),
                        cpu_median_ms: Some(241),
                        peak_memory_kb: Some(249_416),
                        peak_memory_growth_kb: Some(249_416),
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                }],
            }],
        });

        assert!(
            csv.starts_with(
                "device,function,samples,mean_ns,median_ns,p95_ns,min_ns,max_ns,cpu_total_ms,cpu_median_ms,peak_memory_kb,peak_memory_growth_kb,process_peak_memory_kb\n"
            )
        );
        assert!(csv.contains(",482,241,249416,249416,1477787\n"));
    }

    #[test]
    fn render_summary_uses_legacy_peak_memory_as_growth_fallback() {
        let summary = SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Google Pixel 8-14.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(482),
                        cpu_median_ms: Some(241),
                        peak_memory_kb: Some(249_416),
                        peak_memory_growth_kb: None,
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                }],
            }],
        };

        let markdown = render_markdown_summary(&summary);
        let csv = render_csv_summary(&summary);

        assert!(markdown.contains("243.57 MB"));
        assert!(csv.contains(",482,241,249416,249416,1477787\n"));
    }

    #[test]
    fn test_render_markdown_uses_cpu_total_and_peak_memory_columns() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Google Pixel 8-14.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(482),
                        cpu_median_ms: Some(241),
                        peak_memory_kb: Some(654_321),
                        peak_memory_growth_kb: Some(654_321),
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: Some(654_321),
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                }],
            }],
        });

        assert!(markdown.contains("CPU median / iter"));
        assert!(markdown.contains("CPU total"));
        assert!(markdown.contains("CPU / wall"));
        assert!(markdown.contains("Peak growth"));
        assert!(markdown.contains("Process peak"));
        assert!(!markdown.contains("Provider peak"));
        assert!(!markdown.contains("Absolute peak"));
        assert!(!markdown.contains("Peak memory"));
        assert!(markdown.contains("241ms"));
        assert!(markdown.contains("482ms"));
        assert!(markdown.contains("7.7%"));
        assert!(markdown.contains("638.99 MB"));
    }

    #[test]
    fn test_render_table_uses_cpu_total_and_peak_memory_columns() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Ios,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["iPhone 15-17.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "iPhone 15-17.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(482),
                        cpu_median_ms: Some(241),
                        peak_memory_kb: Some(654_321),
                        peak_memory_growth_kb: Some(654_321),
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                }],
            }],
        });

        assert!(markdown.contains("Device"));
        assert!(markdown.contains("Wall mean / iter"));
        assert!(markdown.contains("Wall total"));
        assert!(markdown.contains("CPU median / iter"));
        assert!(markdown.contains("CPU total"));
        assert!(markdown.contains("CPU / wall"));
        assert!(markdown.contains("Peak growth"));
        assert!(markdown.contains("Process peak"));
        assert!(!markdown.contains("Provider peak"));
        assert!(!markdown.contains("Absolute peak"));
        assert!(!markdown.contains("Peak memory"));
        assert!(markdown.contains("241ms"));
        assert!(markdown.contains("482ms"));
        assert!(markdown.contains("7.7%"));
        assert!(markdown.contains("638.99 MB"));
    }

    #[test]
    fn render_markdown_summary_notes_large_process_memory_baseline_gap() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["Motorola Moto G9 Play-11.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Motorola Moto G9 Play-11.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: None,
                        cpu_median_ms: None,
                        peak_memory_kb: Some(171_556),
                        peak_memory_growth_kb: Some(171_556),
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: Some(1_477_787),
                        private_dirty_kb: Some(1_462_460),
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                }],
            }],
        });

        assert!(markdown.contains("Peak growth"));
        assert!(markdown.contains("Process peak"));
        assert!(!markdown.contains("Provider peak"));
        assert!(!markdown.contains("Absolute peak"));
        assert!(markdown.contains(MEMORY_BASELINE_GAP_NOTE));
        assert!(!markdown.contains("Peak memory"));
    }

    #[test]
    fn build_summary_preserves_resource_usage_from_benchmark_results() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
            iterations: 3,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".into()],
            browserstack: None,
            ios_xcuitest: None,
            ios_completion_timeout_secs: None,
        };
        let run_summary = RunSummary {
            spec: spec.clone(),
            artifacts: None,
            local_report: json!({}),
            remote_run: None,
            summary: empty_summary(&spec),
            benchmark_results: Some(BTreeMap::from([(
                "Google Pixel 8-14.0".to_string(),
                vec![json!({
                    "function": "sample_fns::fibonacci",
                    "samples": [
                        { "duration_ns": 1000, "cpu_time_ms": 19, "peak_memory_kb": 48, "process_peak_memory_kb": 1048 },
                        { "duration_ns": 2000, "cpu_time_ms": 7, "peak_memory_kb": 96, "process_peak_memory_kb": 1096 },
                        { "duration_ns": 3000, "cpu_time_ms": 11, "peak_memory_kb": 64, "process_peak_memory_kb": 1064 }
                    ]
                })],
            )])),
            performance_metrics: None,
        };

        let summary = build_summary(&run_summary).expect("build summary");
        let usage = summary.device_summaries[0].benchmarks[0]
            .resource_usage
            .as_ref()
            .expect("resource usage");

        assert_eq!(usage.cpu_total_ms, Some(37));
        assert_eq!(usage.cpu_median_ms, Some(11));
        assert_eq!(usage.peak_memory_kb, Some(96));
        assert_eq!(usage.peak_memory_growth_kb, Some(96));
        assert_eq!(usage.process_peak_memory_kb, Some(1_096));
    }

    #[test]
    fn build_summary_prefers_measured_peak_memory_over_browserstack_perf_memory() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
            iterations: 2,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".into()],
            browserstack: None,
            ios_xcuitest: None,
            ios_completion_timeout_secs: None,
        };
        let run_summary = RunSummary {
            spec: spec.clone(),
            artifacts: None,
            local_report: json!({}),
            remote_run: None,
            summary: empty_summary(&spec),
            benchmark_results: Some(BTreeMap::from([(
                "Google Pixel 8-14.0".to_string(),
                vec![json!({
                    "function": "sample_fns::fibonacci",
                    "samples": [
                        { "duration_ns": 1000, "cpu_time_ms": 10, "peak_memory_kb": 64, "process_peak_memory_kb": 1064 },
                        { "duration_ns": 2000, "cpu_time_ms": 12, "peak_memory_kb": 72, "process_peak_memory_kb": 1072 }
                    ]
                })],
            )])),
            performance_metrics: Some(BTreeMap::from([(
                "Google Pixel 8-14.0".to_string(),
                browserstack::PerformanceMetrics {
                    memory: Some(browserstack::AggregateMemoryMetrics {
                        peak_mb: 999.0,
                        average_mb: 900.0,
                        min_mb: 800.0,
                    }),
                    cpu: None,
                    sample_count: 1,
                    snapshots: vec![],
                },
            )])),
        };

        let summary = build_summary(&run_summary).expect("build summary");
        let usage = summary.device_summaries[0].benchmarks[0]
            .resource_usage
            .as_ref()
            .expect("resource usage");

        assert_eq!(usage.peak_memory_kb, Some(72));
        assert_eq!(usage.peak_memory_growth_kb, Some(72));
        assert_eq!(usage.process_peak_memory_kb, Some(1_072));
    }

    #[test]
    fn format_cpu_total_duration_ms_uses_milliseconds_below_one_second() {
        assert_eq!(format_cpu_total_duration_ms(482), "482ms");
    }

    #[test]
    fn format_cpu_total_duration_ms_uses_total_seconds_at_or_above_one_second() {
        assert_eq!(format_cpu_total_duration_ms(1_000), "1.000s");
        assert_eq!(format_cpu_total_duration_ms(114_248), "114.248s");
        assert_eq!(format_cpu_total_duration_ms(515_822), "515.822s");
    }

    #[test]
    fn parse_pr_number_from_github_ref_extracts_pull_number() {
        assert_eq!(
            parse_pr_number_from_ref("refs/pull/123/merge"),
            Some("123".to_string())
        );
        assert_eq!(parse_pr_number_from_ref("refs/heads/main"), None);
    }

    #[test]
    fn contract_schema_files_compile() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let summary_schema_path = root.join("docs/schemas/summary-v1.schema.json");
        let ci_schema_path = root.join("docs/schemas/ci-contract-v1.schema.json");

        let summary_schema: Value = serde_json::from_str(
            &fs::read_to_string(&summary_schema_path).expect("read summary schema"),
        )
        .expect("parse summary schema");
        let ci_schema: Value =
            serde_json::from_str(&fs::read_to_string(&ci_schema_path).expect("read ci schema"))
                .expect("parse ci schema");

        JSONSchema::options()
            .compile(&summary_schema)
            .expect("compile summary schema");
        JSONSchema::options()
            .compile(&ci_schema)
            .expect("compile ci schema");
    }

    #[test]
    fn run_summary_validates_against_summary_schema() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let summary_schema_path = root.join("docs/schemas/summary-v1.schema.json");
        let summary_schema: Value = serde_json::from_str(
            &fs::read_to_string(&summary_schema_path).expect("read summary schema"),
        )
        .expect("parse summary schema");
        let validator = JSONSchema::options()
            .compile(&summary_schema)
            .expect("compile summary schema");

        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "noop_benchmark".into(),
            iterations: 3,
            warmup: 1,
            devices: vec![],
            ios_completion_timeout_secs: None,
            browserstack: None,
            ios_xcuitest: None,
        };
        let local_report = run_local_smoke(&spec).expect("local harness");
        let mut run_summary = RunSummary {
            spec,
            artifacts: None,
            local_report,
            remote_run: None,
            summary: empty_summary(&RunSpec {
                target: MobileTarget::Android,
                function: "noop_benchmark".into(),
                iterations: 3,
                warmup: 1,
                devices: vec![],
                ios_completion_timeout_secs: None,
                browserstack: None,
                ios_xcuitest: None,
            }),
            benchmark_results: None,
            performance_metrics: None,
        };
        run_summary.summary = build_summary(&run_summary).expect("build summary");
        let value = serde_json::to_value(&run_summary).expect("serialize run summary");

        if let Err(errors) = validator.validate(&value) {
            let messages: Vec<String> = errors.map(|e| e.to_string()).collect();
            panic!("summary schema validation failed: {}", messages.join(" | "));
        }
    }

    #[test]
    fn ci_payload_validates_against_ci_schema() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let ci_schema_path = root.join("docs/schemas/ci-contract-v1.schema.json");
        let ci_schema: Value =
            serde_json::from_str(&fs::read_to_string(&ci_schema_path).expect("read ci schema"))
                .expect("parse ci schema");
        let validator = JSONSchema::options()
            .compile(&ci_schema)
            .expect("compile ci schema");

        let payload = json!({
            "ci": {
                "metadata": {
                    "requested_by": "codex",
                    "pr_number": "123",
                    "request_command": "cargo mobench ci run --target android --function noop_benchmark",
                    "mobench_ref": "refs/heads/codex/ci-devex",
                    "mobench_version": env!("CARGO_PKG_VERSION")
                },
                "outputs": {
                    "summary_json": "target/mobench/ci/summary.json",
                    "summary_md": "target/mobench/ci/summary.md",
                    "results_csv": "target/mobench/ci/results.csv"
                }
            }
        });

        if let Err(errors) = validator.validate(&payload) {
            let messages: Vec<String> = errors.map(|e| e.to_string()).collect();
            panic!("ci schema validation failed: {}", messages.join(" | "));
        }
    }

    #[test]
    fn example_summary_fixtures_validate_against_summary_schema() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let summary_schema_path = root.join("docs/schemas/summary-v1.schema.json");
        let summary_schema: Value = serde_json::from_str(
            &fs::read_to_string(&summary_schema_path).expect("read summary schema"),
        )
        .expect("parse summary schema");
        let validator = JSONSchema::options()
            .compile(&summary_schema)
            .expect("compile summary schema");

        for fixture in [
            "examples/fixtures/basic/summary.json",
            "examples/fixtures/ffi/summary.json",
            "crates/mobench/tests/fixtures/ci-artifact-root/android/summary.json",
        ] {
            let fixture_path = root.join(fixture);
            let value: Value = serde_json::from_str(
                &fs::read_to_string(&fixture_path).expect("read summary fixture"),
            )
            .expect("parse summary fixture");

            if let Err(errors) = validator.validate(&value) {
                let messages: Vec<String> = errors.map(|e| e.to_string()).collect();
                panic!(
                    "{} failed summary schema validation: {}",
                    fixture_path.display(),
                    messages.join(" | ")
                );
            }
        }
    }

    #[test]
    fn basic_example_fixture_renders_stable_markdown_and_csv() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let fixture_path = root.join("examples/fixtures/basic/summary.json");
        let value: Value =
            serde_json::from_str(&fs::read_to_string(&fixture_path).expect("read fixture"))
                .expect("parse fixture");
        let summary = summary_report_from_value(&value).expect("parse summary report");

        let markdown = render_markdown_summary(&summary);
        assert_eq!(
            markdown,
            "\
### Benchmark Summary

- Generated: 2026-03-26T00:00:00Z
- Target: Android
- Function: multiple
- Iterations/Warmup: 5 / 1
- Devices: Google Pixel 8-14.0, Samsung Galaxy S23-14.0

| Device | Function | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Google Pixel 8-14.0 | basic_benchmark::bench_fibonacci | 5 | 1 | 100.000ms | 500.000ms | - | - | - | - | - |
| Google Pixel 8-14.0 | basic_benchmark::bench_checksum | 5 | 1 | 145.000ms | 725.000ms | - | - | - | - | - |
| Samsung Galaxy S23-14.0 | basic_benchmark::bench_fibonacci | 5 | 1 | 94.000ms | 470.000ms | - | - | - | - | - |
| Samsung Galaxy S23-14.0 | basic_benchmark::bench_checksum | 5 | 1 | 136.000ms | 680.000ms | - | - | - | - | - |

"
        );

        let csv = render_csv_summary(&summary);
        assert_eq!(
            csv,
            "\
device,function,samples,mean_ns,median_ns,p95_ns,min_ns,max_ns,cpu_total_ms,cpu_median_ms,peak_memory_kb,peak_memory_growth_kb,process_peak_memory_kb
Google Pixel 8-14.0,basic_benchmark::bench_fibonacci,5,100000000,100000000,105000000,95000000,105000000,,,,,
Google Pixel 8-14.0,basic_benchmark::bench_checksum,5,145000000,145000000,151000000,140000000,151000000,,,,,
Samsung Galaxy S23-14.0,basic_benchmark::bench_fibonacci,5,94000000,94000000,98000000,90000000,98000000,,,,,
Samsung Galaxy S23-14.0,basic_benchmark::bench_checksum,5,136000000,136000000,140000000,132000000,140000000,,,,,
"
        );
    }

    #[test]
    fn ci_function_slug_distinguishes_ambiguous_paths() {
        assert_ne!(ci_function_slug("a::b_c"), ci_function_slug("a_b::c"));
    }

    #[test]
    fn baseline_lookup_matches_device_row() {
        let baseline_report = summarize::SummarizeReport {
            platforms: vec![
                summarize::PlatformReport {
                    platform: "android".to_string(),
                    device: summarize::DeviceInfo {
                        name: "Google Pixel 6".to_string(),
                        os: "Android".to_string(),
                        os_version: "14".to_string(),
                        chipset: None,
                        ram_gb: None,
                    },
                    benchmarks: vec![summarize::BenchmarkResult {
                        name: "bench_alpha".to_string(),
                        label: "alpha".to_string(),
                        timing: summarize::TimingStats {
                            avg_ms: 100.0,
                            median_ms: 100.0,
                            best_ms: 100.0,
                            worst_ms: 100.0,
                            p95_ms: 100.0,
                            std_dev_ms: None,
                        },
                        resource_usage: None,
                    }],
                    iterations: 5,
                    warmup: 1,
                },
                summarize::PlatformReport {
                    platform: "android".to_string(),
                    device: summarize::DeviceInfo {
                        name: "Samsung Galaxy S24".to_string(),
                        os: "Android".to_string(),
                        os_version: "14".to_string(),
                        chipset: None,
                        ram_gb: None,
                    },
                    benchmarks: vec![summarize::BenchmarkResult {
                        name: "bench_alpha".to_string(),
                        label: "alpha".to_string(),
                        timing: summarize::TimingStats {
                            avg_ms: 200.0,
                            median_ms: 200.0,
                            best_ms: 200.0,
                            worst_ms: 200.0,
                            p95_ms: 200.0,
                            std_dev_ms: None,
                        },
                        resource_usage: None,
                    }],
                    iterations: 5,
                    warmup: 1,
                },
            ],
        };

        let baseline = find_baseline_benchmark(
            &baseline_report,
            "android",
            "Samsung Galaxy S24",
            "14",
            "bench_alpha",
        )
        .expect("matching baseline benchmark");

        assert_eq!(baseline.timing.avg_ms, 200.0);
    }
}

#[cfg(test)]
mod result_extraction_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_all_benchmark_results() {
        let results: HashMap<String, Vec<serde_json::Value>> = [
            (
                "Pixel 7".to_string(),
                vec![json!({
                    "function": "my_crate::bench_fn",
                    "mean_ns": 12345678,
                    "samples": [{"duration_ns": 12345678}]
                })],
            ),
            (
                "iPhone 14".to_string(),
                vec![json!({
                    "function": "my_crate::bench_fn",
                    "mean_ns": 11111111,
                    "samples": [{"duration_ns": 11111111}]
                })],
            ),
        ]
        .into_iter()
        .collect();

        let extracted = extract_benchmark_summary(&results);
        assert_eq!(extracted.len(), 2);
        assert!(extracted.iter().any(|r| r.device == "Pixel 7"));
        assert!(extracted.iter().any(|r| r.device == "iPhone 14"));
    }

    #[test]
    fn test_extract_with_multiple_samples() {
        let results: HashMap<String, Vec<serde_json::Value>> = [(
            "Device".to_string(),
            vec![json!({
                "function": "test_fn",
                "mean_ns": 100,
                "samples": [
                    {"duration_ns": 80},
                    {"duration_ns": 100},
                    {"duration_ns": 120}
                ]
            })],
        )]
        .into_iter()
        .collect();

        let extracted = extract_benchmark_summary(&results);
        assert_eq!(extracted.len(), 1);
        let result = &extracted[0];
        assert_eq!(result.sample_count, 3);
        assert_eq!(result.min_ns, Some(80));
        assert_eq!(result.max_ns, Some(120));
        assert!(result.std_dev_ns.is_some());
    }
}

#[cfg(test)]
mod ci_merge_tests {
    use super::*;
    use serde_json::json;

    fn sample_run_summary(
        target: MobileTarget,
        function: &str,
        device: &str,
        mean_ns: u64,
    ) -> Value {
        json!({
            "summary": {
                "generated_at": "2026-02-16T00:00:00Z",
                "generated_at_unix": 1708041600,
                "target": target.as_str(),
                "function": function,
                "iterations": 3,
                "warmup": 1,
                "devices": [device],
                "device_summaries": [{
                    "device": device,
                    "benchmarks": [{
                        "function": function,
                        "samples": 3,
                        "mean_ns": mean_ns,
                        "median_ns": mean_ns,
                        "p95_ns": mean_ns,
                        "min_ns": mean_ns,
                        "max_ns": mean_ns
                    }]
                }]
            }
        })
    }

    #[test]
    fn merge_ci_target_runs_preserves_all_functions() {
        let runs = BTreeMap::from([
            (
                "bench_a".to_string(),
                sample_run_summary(MobileTarget::Ios, "bench_a", "iPhone 14-16.0", 100),
            ),
            (
                "bench_b".to_string(),
                sample_run_summary(MobileTarget::Ios, "bench_b", "iPhone 14-16.0", 200),
            ),
        ]);

        let merged = merge_ci_target_runs(MobileTarget::Ios, &runs).unwrap();
        let functions = merged
            .get("functions")
            .and_then(|v| v.as_object())
            .expect("functions map");
        assert_eq!(functions.len(), 2);

        let benchmarks = merged["summary"]["device_summaries"][0]["benchmarks"]
            .as_array()
            .expect("benchmarks");
        assert_eq!(benchmarks.len(), 2);
        assert_eq!(benchmarks[0]["function"], "bench_a");
        assert_eq!(benchmarks[1]["function"], "bench_b");
    }

    #[test]
    fn root_summary_from_merged_targets_returns_summary_for_single_target() {
        let merged_target = merge_ci_target_runs(
            MobileTarget::Ios,
            &BTreeMap::from([(
                "bench_a".to_string(),
                sample_run_summary(MobileTarget::Ios, "bench_a", "iPhone 14-16.0", 100),
            )]),
        )
        .unwrap();
        let targets = BTreeMap::from([("ios".to_string(), merged_target)]);

        let root_summary = root_summary_from_merged_targets(&targets).expect("single target");
        assert_eq!(root_summary["target"], "ios");
        assert_eq!(
            root_summary["device_summaries"][0]["benchmarks"][0]["function"],
            "bench_a"
        );
    }

    #[test]
    fn merge_ci_target_runs_preserves_resource_usage() {
        let runs = BTreeMap::from([
            (
                "bench_a".to_string(),
                json!({
                    "summary": {
                        "generated_at": "2026-02-16T00:00:00Z",
                        "generated_at_unix": 1708041600,
                        "target": "android",
                        "function": "bench_a",
                        "iterations": 3,
                        "warmup": 1,
                        "devices": ["Pixel 8-14.0"],
                        "device_summaries": [{
                            "device": "Pixel 8-14.0",
                            "benchmarks": [{
                                "function": "bench_a",
                                "samples": 3,
                                "mean_ns": 100,
                                "median_ns": 100,
                                "p95_ns": 100,
                                "min_ns": 100,
                                "max_ns": 100,
                                "resource_usage": {
                                    "cpu_total_ms": 482,
                                    "peak_memory_kb": 654321,
                                    "total_pss_kb": 654321
                                }
                            }]
                        }]
                    }
                }),
            ),
            (
                "bench_b".to_string(),
                sample_run_summary(MobileTarget::Android, "bench_b", "Pixel 8-14.0", 200),
            ),
        ]);

        let merged = merge_ci_target_runs(MobileTarget::Android, &runs).expect("merge targets");
        let benchmarks = merged["summary"]["device_summaries"][0]["benchmarks"]
            .as_array()
            .expect("benchmarks");
        let bench_a = benchmarks
            .iter()
            .find(|benchmark| benchmark["function"] == "bench_a")
            .expect("bench_a");

        assert_eq!(bench_a["resource_usage"]["cpu_total_ms"], 482);
        assert_eq!(bench_a["resource_usage"]["peak_memory_kb"], 654321);
    }

    #[test]
    fn render_summary_markdown_from_output_renders_all_functions_from_merged_targets() {
        let ios = merge_ci_target_runs(
            MobileTarget::Ios,
            &BTreeMap::from([
                (
                    "bench_a".to_string(),
                    sample_run_summary(MobileTarget::Ios, "bench_a", "iPhone 14-16.0", 100),
                ),
                (
                    "bench_b".to_string(),
                    sample_run_summary(MobileTarget::Ios, "bench_b", "iPhone 14-16.0", 200),
                ),
            ]),
        )
        .unwrap();
        let android = merge_ci_target_runs(
            MobileTarget::Android,
            &BTreeMap::from([(
                "bench_c".to_string(),
                sample_run_summary(MobileTarget::Android, "bench_c", "Pixel 7-14.0", 300),
            )]),
        )
        .unwrap();

        let markdown = render_summary_markdown_from_output(&json!({
            "targets": {
                "ios": ios,
                "android": android
            }
        }))
        .unwrap();

        assert!(markdown.contains("## ios"));
        assert!(markdown.contains("## android"));
        assert!(markdown.contains("bench_a"));
        assert!(markdown.contains("bench_b"));
        assert!(markdown.contains("bench_c"));
    }

    #[test]
    fn report_summarize_accepts_raw_benchmark_report() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let report_path = temp_dir.path().join("bench-report.json");
        write_file(
            &report_path,
            serde_json::to_string_pretty(&json!({
                "spec": {
                    "name": "bench_mobile::bench_prove",
                    "iterations": 3,
                    "warmup": 1
                },
                "samples": [
                    { "duration_ns": 1_000_000_u64 },
                    { "duration_ns": 2_000_000_u64 },
                    { "duration_ns": 3_000_000_u64 }
                ],
                "resource_usage": {
                    "cpu_total_ms": 42_u64,
                    "peak_memory_kb": 1024_u64
                }
            }))
            .expect("serialize raw benchmark report")
            .as_bytes(),
        )
        .expect("write raw report");

        let markdown =
            cmd_report_summarize(&report_path, None, plots::PlotMode::Off).expect("summarize");

        assert!(markdown.contains("bench_mobile::bench_prove"));
        assert!(markdown.contains("local"));
        assert!(markdown.contains("2.000ms"));
        assert!(markdown.contains("42ms"));
        assert!(markdown.contains("1.00 MB"));
    }

    #[test]
    fn report_summarize_accepts_raw_benchmark_report_array() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let report_path = temp_dir.path().join("bench-report.json");
        write_file(
            &report_path,
            serde_json::to_string_pretty(&json!([
                {
                    "spec": {
                        "name": "bench_mobile::bench_prove",
                        "iterations": 3,
                        "warmup": 1
                    },
                    "samples": [
                        { "duration_ns": 1_000_000_u64 },
                        { "duration_ns": 2_000_000_u64 },
                        { "duration_ns": 3_000_000_u64 }
                    ]
                },
                {
                    "spec": {
                        "name": "bench_mobile::bench_verify",
                        "iterations": 3,
                        "warmup": 1
                    },
                    "samples_ns": [
                        4_000_000_u64,
                        5_000_000_u64,
                        6_000_000_u64
                    ]
                }
            ]))
            .expect("serialize raw benchmark report")
            .as_bytes(),
        )
        .expect("write raw report");

        let markdown =
            cmd_report_summarize(&report_path, None, plots::PlotMode::Off).expect("summarize");

        assert!(markdown.contains("bench_mobile::bench_prove"));
        assert!(markdown.contains("bench_mobile::bench_verify"));
        assert!(markdown.contains("2.000ms"));
        assert!(markdown.contains("5.000ms"));
    }

    #[test]
    fn render_markdown_summary_uses_h3_heading_and_ios_label() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-03-27T00:45:55.028899Z".to_string(),
            generated_at_unix: 1_774_569_955,
            target: MobileTarget::Ios,
            function: "ffi_benchmark::bench_fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["iPhone 13-15".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "iPhone 13".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "ffi_benchmark::bench_fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(17_000),
                    median_ns: Some(17_000),
                    p95_ns: Some(18_000),
                    min_ns: Some(16_000),
                    max_ns: Some(19_000),
                    resource_usage: None,
                }],
            }],
        });

        assert!(markdown.starts_with("### Benchmark Summary\n"));
        assert!(markdown.contains("- Target: iOS"));
        assert!(markdown.contains("| Device | Function | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak |"));
        assert!(markdown.contains("| iPhone 13 | ffi_benchmark::bench_fibonacci | 5 | 1 | 0.017ms | 0.085ms | - | - | - | - | - |"));
        assert!(!markdown.contains("### Device:"));
    }

    #[cfg(unix)]
    #[test]
    fn render_summary_markdown_from_output_with_plots_embeds_image_links() {
        let output = json!({
            "summary": {
                "generated_at": "2026-03-25T00:00:00Z",
                "generated_at_unix": 1_742_862_400_u64,
                "target": "android",
                "function": "bench_alpha",
                "iterations": 3,
                "warmup": 1,
                "devices": ["Google Pixel 8-14.0", "iPhone 15-17.4"],
                "device_summaries": [
                    {
                        "device": "Google Pixel 8-14.0",
                        "benchmarks": [{
                            "function": "bench_alpha",
                            "samples": 3,
                            "mean_ns": 97_u64,
                            "median_ns": 98_u64,
                            "p95_ns": 100_u64,
                            "min_ns": 95_u64,
                            "max_ns": 100_u64
                        }]
                    },
                    {
                        "device": "iPhone 15-17.4",
                        "benchmarks": [{
                            "function": "bench_alpha",
                            "samples": 3,
                            "mean_ns": 82_u64,
                            "median_ns": 82_u64,
                            "p95_ns": 84_u64,
                            "min_ns": 80_u64,
                            "max_ns": 84_u64
                        }]
                    }
                ]
            },
            "benchmark_results": {
                "Google Pixel 8-14.0": [{
                    "function": "bench_alpha",
                    "samples": [95_u64, 98_u64, 100_u64]
                }],
                "iPhone 15-17.4": [{
                    "function": "bench_alpha",
                    "samples": [80_u64, 82_u64, 84_u64]
                }]
            }
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let fake_python = crate::tests::write_fake_plot_python(dir.path());

        let markdown = render_summary_markdown_from_output_with_plots_using_python(
            &output,
            dir.path(),
            plots::PlotMode::Require,
            Some(&fake_python),
        )
        .expect("render markdown with plots");

        assert!(markdown.contains("### Device Comparison Plots"));
        assert!(markdown.contains("![alpha](plots/alpha.svg)"));
        assert!(dir.path().join("plots/alpha.svg").exists());
    }

    #[cfg(unix)]
    #[test]
    fn render_summary_markdown_from_output_with_plots_deduplicates_across_targets() {
        let merged = json!({
            "targets": {
                "android": {
                    "summary": {
                        "generated_at": "2026-03-25T00:00:00Z",
                        "generated_at_unix": 1_742_862_400_u64,
                        "target": "android",
                        "function": "bench_alpha",
                        "iterations": 3,
                        "warmup": 1,
                        "devices": ["Google Pixel 8-14.0"],
                        "device_summaries": [{
                            "device": "Google Pixel 8-14.0",
                            "benchmarks": [{
                                "function": "bench_alpha",
                                "samples": 3,
                                "mean_ns": 97_u64,
                                "median_ns": 98_u64,
                                "p95_ns": 100_u64,
                                "min_ns": 95_u64,
                                "max_ns": 100_u64
                            }]
                        }]
                    },
                    "functions": {
                        "bench_alpha": {
                            "summary": {
                                "generated_at": "2026-03-25T00:00:00Z",
                                "generated_at_unix": 1_742_862_400_u64,
                                "target": "android",
                                "function": "bench_alpha",
                                "iterations": 3,
                                "warmup": 1,
                                "devices": ["Google Pixel 8-14.0"],
                                "device_summaries": [{
                                    "device": "Google Pixel 8-14.0",
                                    "benchmarks": [{
                                        "function": "bench_alpha",
                                        "samples": 3,
                                        "mean_ns": 97_u64,
                                        "median_ns": 98_u64,
                                        "p95_ns": 100_u64,
                                        "min_ns": 95_u64,
                                        "max_ns": 100_u64
                                    }]
                                }]
                            },
                            "benchmark_results": {
                                "Google Pixel 8-14.0": [{
                                    "function": "bench_alpha",
                                    "samples": [95_u64, 98_u64, 100_u64]
                                }]
                            }
                        }
                    }
                },
                "ios": {
                    "summary": {
                        "generated_at": "2026-03-25T00:00:00Z",
                        "generated_at_unix": 1_742_862_400_u64,
                        "target": "ios",
                        "function": "bench_alpha",
                        "iterations": 3,
                        "warmup": 1,
                        "devices": ["iPhone 15-17.4"],
                        "device_summaries": [{
                            "device": "iPhone 15-17.4",
                            "benchmarks": [{
                                "function": "bench_alpha",
                                "samples": 3,
                                "mean_ns": 82_u64,
                                "median_ns": 82_u64,
                                "p95_ns": 84_u64,
                                "min_ns": 80_u64,
                                "max_ns": 84_u64
                            }]
                        }]
                    },
                    "functions": {
                        "bench_alpha": {
                            "summary": {
                                "generated_at": "2026-03-25T00:00:00Z",
                                "generated_at_unix": 1_742_862_400_u64,
                                "target": "ios",
                                "function": "bench_alpha",
                                "iterations": 3,
                                "warmup": 1,
                                "devices": ["iPhone 15-17.4"],
                                "device_summaries": [{
                                    "device": "iPhone 15-17.4",
                                    "benchmarks": [{
                                        "function": "bench_alpha",
                                        "samples": 3,
                                        "mean_ns": 82_u64,
                                        "median_ns": 82_u64,
                                        "p95_ns": 84_u64,
                                        "min_ns": 80_u64,
                                        "max_ns": 84_u64
                                    }]
                                }]
                            },
                            "benchmark_results": {
                                "iPhone 15-17.4": [{
                                    "function": "bench_alpha",
                                    "samples": [80_u64, 82_u64, 84_u64]
                                }]
                            }
                        }
                    }
                }
            }
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let fake_python = crate::tests::write_fake_plot_python(dir.path());

        let markdown = render_summary_markdown_from_output_with_plots_using_python(
            &merged,
            dir.path(),
            plots::PlotMode::Require,
            Some(&fake_python),
        )
        .expect("render merged markdown with plots");

        assert!(markdown.contains("## android"));
        assert!(markdown.contains("## ios"));
        assert!(markdown.contains("![alpha](plots/alpha.svg)"));
        assert!(markdown.contains("![alpha](plots/alpha-ios.svg)"));
        assert!(dir.path().join("plots/alpha.svg").exists());
        assert!(dir.path().join("plots/alpha-ios.svg").exists());
    }

    #[test]
    fn build_summary_preserves_resource_usage_from_benchmark_results() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "bench_nullifier_proving_only".into(),
            iterations: 3,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".into()],
            browserstack: None,
            ios_xcuitest: None,
            ios_completion_timeout_secs: None,
        };
        let local_report = json!({});
        let run_summary = RunSummary {
            spec: spec.clone(),
            artifacts: None,
            local_report,
            remote_run: None,
            summary: empty_summary(&spec),
            benchmark_results: Some(BTreeMap::from([(
                "Google Pixel 8-14.0".to_string(),
                vec![json!({
                    "function": "bench_nullifier_proving_only",
                    "mean_ns": 125000000_u64,
                    "samples": [
                        { "duration_ns": 120000000_u64 },
                        { "duration_ns": 130000000_u64 }
                    ],
                    "resources": {
                        "elapsed_cpu_ms": 482,
                        "total_pss_kb": 654321,
                        "private_dirty_kb": 321000,
                        "native_heap_kb": 120000,
                        "java_heap_kb": 45000
                    }
                })],
            )])),
            performance_metrics: None,
        };

        let summary = build_summary(&run_summary).expect("build summary");
        let value = serde_json::to_value(summary).expect("serialize summary");
        let resource_usage = &value["device_summaries"][0]["benchmarks"][0]["resource_usage"];

        assert_eq!(resource_usage["cpu_total_ms"], 482);
        assert_eq!(resource_usage["peak_memory_kb"], Value::Null);
        assert_eq!(resource_usage["peak_memory_growth_kb"], Value::Null);
        assert_eq!(resource_usage["process_peak_memory_kb"], Value::Null);
        assert_eq!(resource_usage["total_pss_kb"], 654321);
        assert_eq!(resource_usage["private_dirty_kb"], 321000);
        assert_eq!(resource_usage["native_heap_kb"], 120000);
        assert_eq!(resource_usage["java_heap_kb"], 45000);
    }

    #[test]
    fn build_summary_ignores_browserstack_peak_memory_for_ci_summary() {
        let spec = RunSpec {
            target: MobileTarget::Ios,
            function: "bench_nullifier_proving_only".into(),
            iterations: 3,
            warmup: 1,
            devices: vec!["iPhone 15-17.0".into()],
            browserstack: None,
            ios_xcuitest: None,
            ios_completion_timeout_secs: None,
        };
        let run_summary = RunSummary {
            spec: spec.clone(),
            artifacts: None,
            local_report: json!({}),
            remote_run: None,
            summary: empty_summary(&spec),
            benchmark_results: Some(BTreeMap::from([(
                "iPhone 15-17.0".to_string(),
                vec![json!({
                    "function": "bench_nullifier_proving_only",
                    "mean_ns": 125000000_u64,
                    "samples": [
                        { "duration_ns": 120000000_u64 },
                        { "duration_ns": 130000000_u64 }
                    ],
                    "resources": {
                        "platform": "ios"
                    }
                })],
            )])),
            performance_metrics: Some(BTreeMap::from([(
                "iPhone 15-17.0".to_string(),
                browserstack::PerformanceMetrics {
                    sample_count: 1,
                    memory: Some(browserstack::AggregateMemoryMetrics {
                        peak_mb: 243.57,
                        average_mb: 169.45,
                        min_mb: 169.45,
                    }),
                    cpu: Some(browserstack::AggregateCpuMetrics {
                        peak_percent: 12.52,
                        average_percent: 5.06,
                        min_percent: 5.06,
                    }),
                    snapshots: Vec::new(),
                },
            )])),
        };

        let summary = build_summary(&run_summary).expect("build summary");
        let value = serde_json::to_value(summary).expect("serialize summary");
        let benchmark = &value["device_summaries"][0]["benchmarks"][0];

        assert_eq!(benchmark["resource_usage"], Value::Null);
    }
}

#[cfg(test)]
mod init_sdk_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_sdk_creates_mobench_toml() {
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path().join("my-bench");

        // Run init-sdk
        cmd_init_sdk(
            SdkTarget::Android,
            "my-bench".to_string(),
            output_dir.clone(),
            false,
        )
        .unwrap();

        // Check mobench.toml was created
        let config_path = output_dir.join("mobench.toml");
        assert!(
            config_path.exists(),
            "mobench.toml should be created by init-sdk"
        );

        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            contents.contains("my-bench"),
            "Config should contain project name"
        );
        assert!(
            contents.contains("[project]"),
            "Config should have [project] section"
        );
        assert!(
            contents.contains("[benchmarks]"),
            "Config should have [benchmarks] section"
        );
    }

    #[test]
    fn test_init_sdk_mobench_toml_has_correct_library_name() {
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path().join("my-project");

        cmd_init_sdk(
            SdkTarget::Android,
            "my-project".to_string(),
            output_dir.clone(),
            false,
        )
        .unwrap();

        let config_path = output_dir.join("mobench.toml");
        let contents = std::fs::read_to_string(&config_path).unwrap();

        // Library name should have hyphens replaced with underscores
        assert!(
            contents.contains("library_name = \"my_project\""),
            "Config should have library_name with underscores"
        );
    }
}

#[cfg(test)]
mod resource_usage_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_resource_usage_from_entry_fields() {
        let entry = json!({
            "resources": {
                "elapsed_cpu_ms": 120,
                "total_pss_kb": 4096,
                "private_dirty_kb": 2048,
                "native_heap_kb": 1024,
                "java_heap_kb": 512
            }
        });

        let usage = extract_benchmark_resource_usage(&entry, None).unwrap();
        assert_eq!(usage.cpu_total_ms, Some(120));
        assert_eq!(usage.total_pss_kb, Some(4096));
        assert_eq!(usage.private_dirty_kb, Some(2048));
        assert_eq!(usage.native_heap_kb, Some(1024));
        assert_eq!(usage.java_heap_kb, Some(512));
        assert_eq!(usage.peak_memory_kb, None);
        assert_eq!(usage.peak_memory_growth_kb, None);
        assert_eq!(usage.process_peak_memory_kb, None);
    }

    #[test]
    fn test_extract_resource_usage_ignores_provider_peak() {
        let entry = json!({
            "resources": {
                "total_pss_kb": 4096
            }
        });
        let perf = browserstack::PerformanceMetrics {
            sample_count: 5,
            memory: Some(browserstack::AggregateMemoryMetrics {
                peak_mb: 10.0,
                average_mb: 8.0,
                min_mb: 6.0,
            }),
            cpu: None,
            snapshots: vec![],
        };

        let usage = extract_benchmark_resource_usage(&entry, Some(&perf)).unwrap();
        assert_eq!(usage.peak_memory_kb, None);
        assert_eq!(usage.peak_memory_growth_kb, None);
        assert_eq!(usage.process_peak_memory_kb, None);
        assert_eq!(usage.total_pss_kb, Some(4096));
    }

    #[test]
    fn test_extract_resource_usage_preserves_moto_growth_and_process_peak() {
        let entry = json!({
            "resources": {
                "peak_memory_kb": 171556,
                "process_peak_memory_kb": 1477787,
                "total_pss_kb": 1477787,
                "private_dirty_kb": 1462460,
                "native_heap_kb": 532000,
                "java_heap_kb": 212000
            }
        });
        let perf = browserstack::PerformanceMetrics {
            sample_count: 5,
            memory: Some(browserstack::AggregateMemoryMetrics {
                peak_mb: 1640.65,
                average_mb: 1500.0,
                min_mb: 1400.0,
            }),
            cpu: None,
            snapshots: vec![],
        };

        let usage = extract_benchmark_resource_usage(&entry, Some(&perf)).unwrap();

        assert_eq!(usage.peak_memory_growth_kb, Some(171_556));
        assert_eq!(usage.peak_memory_kb, Some(171_556));
        assert_eq!(usage.process_peak_memory_kb, Some(1_477_787));
        assert_eq!(usage.total_pss_kb, Some(1_477_787));
        assert_eq!(usage.private_dirty_kb, Some(1_462_460));
        assert_eq!(usage.native_heap_kb, Some(532_000));
        assert_eq!(usage.java_heap_kb, Some(212_000));
    }

    #[test]
    fn test_extract_resource_usage_empty_returns_none() {
        let entry = json!({});
        let usage = extract_benchmark_resource_usage(&entry, None);
        assert!(usage.is_none());
    }

    #[test]
    fn test_resource_usage_json_round_trip() {
        let usage = BenchmarkResourceUsage {
            cpu_total_ms: Some(250),
            cpu_median_ms: Some(125),
            peak_memory_kb: Some(8192),
            peak_memory_growth_kb: Some(8192),
            process_peak_memory_kb: Some(12288),
            total_pss_kb: Some(4096),
            private_dirty_kb: Some(2048),
            native_heap_kb: Some(1024),
            java_heap_kb: None,
        };

        let json_str = serde_json::to_string(&usage).unwrap();
        let deserialized: BenchmarkResourceUsage = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.cpu_total_ms, Some(250));
        assert_eq!(deserialized.cpu_median_ms, Some(125));
        assert_eq!(deserialized.peak_memory_kb, Some(8192));
        assert_eq!(deserialized.peak_memory_growth_kb, Some(8192));
        assert_eq!(deserialized.process_peak_memory_kb, Some(12288));
        assert_eq!(deserialized.total_pss_kb, Some(4096));
        assert_eq!(deserialized.private_dirty_kb, Some(2048));
        assert_eq!(deserialized.native_heap_kb, Some(1024));
        assert_eq!(deserialized.java_heap_kb, None);

        // java_heap_kb should be absent in JSON due to skip_serializing_if
        assert!(!json_str.contains("java_heap_kb"));
        assert!(json_str.contains("peak_memory_kb"));
        assert!(json_str.contains("peak_memory_growth_kb"));
        assert!(json_str.contains("process_peak_memory_kb"));
        assert!(!json_str.contains("absolute_peak_memory_kb"));
    }
}
