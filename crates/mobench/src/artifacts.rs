use anyhow::{Result, anyhow};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::{IosSigningMethodArg, MobileTarget, SdkTarget};
use crate::{
    IosXcuitestArtifacts, ResolvedProjectLayout, RunSpec, configured_android_abis,
    configured_ios_completion_timeout_secs, validate_spec_file, with_ios_benchmark_timeout_env,
    write_file,
};

pub(crate) struct ArtifactLifecycle<'a> {
    layout: &'a ResolvedProjectLayout,
    output_dir: PathBuf,
    ios_completion_timeout_secs: Option<u64>,
}

impl<'a> ArtifactLifecycle<'a> {
    pub(crate) fn new(
        layout: &'a ResolvedProjectLayout,
        output_dir: Option<PathBuf>,
        ios_completion_timeout_secs: Option<u64>,
    ) -> Self {
        Self {
            layout,
            output_dir: output_dir.unwrap_or_else(|| layout.output_dir.clone()),
            ios_completion_timeout_secs: configured_ios_completion_timeout_secs(
                layout,
                ios_completion_timeout_secs,
            ),
        }
    }

    pub(crate) fn output_dir(&self) -> &Path {
        &self.output_dir
    }

    pub(crate) fn ios_completion_timeout_secs(&self) -> Option<u64> {
        self.ios_completion_timeout_secs
    }

    pub(crate) fn build_config(
        &self,
        target: impl Into<mobench_sdk::Target>,
        release: bool,
    ) -> mobench_sdk::BuildConfig {
        let target = target.into();
        mobench_sdk::BuildConfig {
            target,
            profile: if release {
                mobench_sdk::BuildProfile::Release
            } else {
                mobench_sdk::BuildProfile::Debug
            },
            incremental: true,
            android_abis: if matches!(
                target,
                mobench_sdk::Target::Android | mobench_sdk::Target::Both
            ) {
                self.layout.android_abis.clone()
            } else {
                None
            },
        }
    }

    pub(crate) fn android_builder(
        &self,
        verbose: bool,
        dry_run: bool,
    ) -> mobench_sdk::builders::AndroidBuilder {
        mobench_sdk::builders::AndroidBuilder::new(
            &self.layout.project_root,
            self.layout.crate_name.clone(),
        )
        .verbose(verbose)
        .dry_run(dry_run)
        .output_dir(&self.output_dir)
        .crate_dir(&self.layout.crate_dir)
    }

    pub(crate) fn ios_builder(
        &self,
        verbose: bool,
        dry_run: bool,
    ) -> mobench_sdk::builders::IosBuilder {
        mobench_sdk::builders::IosBuilder::new(
            &self.layout.project_root,
            self.layout.crate_name.clone(),
        )
        .verbose(verbose)
        .dry_run(dry_run)
        .output_dir(&self.output_dir)
        .crate_dir(&self.layout.crate_dir)
    }

    pub(crate) fn run_ios_build(&self, release: bool, dry_run: bool) -> Result<(PathBuf, PathBuf)> {
        let builder = self.ios_builder(true, dry_run);
        let cfg = self.build_config(mobench_sdk::Target::Ios, release);
        let result = with_ios_benchmark_timeout_env(self.ios_completion_timeout_secs, || {
            Ok(builder.build(&cfg)?)
        })?;
        let header = self
            .output_dir
            .join("ios/include")
            .join(format!("{}.h", self.layout.library_name));
        Ok((result.app_path, header))
    }

    pub(crate) fn run_android_build(
        &self,
        release: bool,
        dry_run: bool,
    ) -> Result<mobench_sdk::BuildResult> {
        let builder = self.android_builder(true, dry_run);
        let cfg = self.build_config(mobench_sdk::Target::Android, release);
        Ok(builder.build(&cfg)?)
    }

    pub(crate) fn package_ipa(&self, scheme: &str, method: IosSigningMethodArg) -> Result<PathBuf> {
        let builder = self.ios_builder(true, false);
        let signing_method: mobench_sdk::builders::SigningMethod = method.into();
        Ok(builder.package_ipa(scheme, signing_method)?)
    }

    pub(crate) fn package_xcuitest(&self, scheme: &str) -> Result<PathBuf> {
        let builder = self.ios_builder(true, false);
        Ok(builder.package_xcuitest(scheme)?)
    }

    pub(crate) fn package_ios_xcuitest_artifacts(
        &self,
        release: bool,
    ) -> Result<IosXcuitestArtifacts> {
        let builder = self.ios_builder(true, false);
        let cfg = self.build_config(mobench_sdk::Target::Ios, release);
        with_ios_benchmark_timeout_env(self.ios_completion_timeout_secs, || {
            Ok(builder.build(&cfg)?)
        })?;
        let app =
            builder.package_ipa("BenchRunner", mobench_sdk::builders::SigningMethod::AdHoc)?;
        let test_suite = builder.package_xcuitest("BenchRunner")?;
        Ok(IosXcuitestArtifacts { app, test_suite })
    }

    pub(crate) fn default_spec_paths(&self) -> [PathBuf; 4] {
        [
            self.output_dir
                .join("android/app/src/main/assets/bench_spec.json"),
            self.output_dir
                .join("ios/BenchRunner/BenchRunner/bench_spec.json"),
            self.layout
                .project_root
                .join("target/mobile-spec/android/bench_spec.json"),
            self.layout
                .project_root
                .join("target/mobile-spec/ios/bench_spec.json"),
        ]
    }

    pub(crate) fn artifact_details(&self, target: Option<SdkTarget>) -> (bool, Vec<String>) {
        let mut artifacts_ok = true;
        let mut artifact_details = Vec::new();

        if let Some(ref target) = target {
            if matches!(target, SdkTarget::Android | SdkTarget::Both) {
                let apk_debug = self
                    .output_dir
                    .join("android/app/build/outputs/apk/debug/app-debug.apk");
                let apk_release = self
                    .output_dir
                    .join("android/app/build/outputs/apk/release/app-release-unsigned.apk");
                if apk_debug.exists() {
                    artifact_details.push(format!("Android APK (debug): {:?}", apk_debug));
                } else if apk_release.exists() {
                    artifact_details.push(format!("Android APK (release): {:?}", apk_release));
                } else {
                    artifact_details.push("Android APK: NOT FOUND".to_string());
                    artifacts_ok = false;
                }

                let jni_base = self.output_dir.join("android/app/src/main/jniLibs");
                for abi in configured_android_abis(self.layout) {
                    let lib_path = jni_base
                        .join(&abi)
                        .join(format!("lib{}.so", self.layout.library_name));
                    if lib_path.exists() {
                        artifact_details.push(format!("JNI lib ({}): OK", abi));
                    }
                }
            }

            if matches!(target, SdkTarget::Ios | SdkTarget::Both) {
                let xcframework = self
                    .output_dir
                    .join("ios")
                    .join(format!("{}.xcframework", self.layout.library_name));
                if xcframework.exists() {
                    artifact_details.push(format!("iOS xcframework: {:?}", xcframework));
                } else {
                    artifact_details.push("iOS xcframework: NOT FOUND".to_string());
                    artifacts_ok = false;
                }

                let ipa_path = self.output_dir.join("ios/BenchRunner.ipa");
                if ipa_path.exists() {
                    artifact_details.push(format!("iOS IPA: {:?}", ipa_path));
                }

                let xcuitest_path = self.output_dir.join("ios/BenchRunnerUITests.zip");
                if xcuitest_path.exists() {
                    artifact_details.push(format!("XCUITest runner: {:?}", xcuitest_path));
                }
            }
        } else {
            let android_apk = self
                .output_dir
                .join("android/app/build/outputs/apk/debug/app-debug.apk");
            let ios_xcframework = self
                .output_dir
                .join("ios")
                .join(format!("{}.xcframework", self.layout.library_name));

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

        (artifacts_ok, artifact_details)
    }

    pub(crate) fn default_ios_xcuitest_artifacts(&self) -> IosXcuitestArtifacts {
        IosXcuitestArtifacts {
            app: self.output_dir.join("ios/BenchRunner.ipa"),
            test_suite: self.output_dir.join("ios/BenchRunnerUITests.zip"),
        }
    }

    pub(crate) fn legacy_ios_xcuitest_artifacts(&self) -> IosXcuitestArtifacts {
        IosXcuitestArtifacts {
            app: self.layout.project_root.join("target/ios/BenchRunner.ipa"),
            test_suite: self
                .layout
                .project_root
                .join("target/ios/BenchRunnerUITests.zip"),
        }
    }

    pub(crate) fn uses_managed_ios_xcuitest_artifacts(
        &self,
        artifacts: &IosXcuitestArtifacts,
    ) -> bool {
        let app = resolve_project_relative_path(&self.layout.project_root, &artifacts.app);
        let test_suite =
            resolve_project_relative_path(&self.layout.project_root, &artifacts.test_suite);

        [
            self.default_ios_xcuitest_artifacts(),
            self.legacy_ios_xcuitest_artifacts(),
        ]
        .into_iter()
        .any(|managed| app == managed.app && test_suite == managed.test_suite)
    }
}

pub(crate) fn persist_mobile_spec(
    lifecycle: &ArtifactLifecycle<'_>,
    spec: &RunSpec,
    release: bool,
) -> Result<()> {
    let payload = json!({
        "function": spec.function,
        "iterations": spec.iterations,
        "warmup": spec.warmup,
    });
    let contents = serde_json::to_string_pretty(&payload)?;

    for path in lifecycle.default_spec_paths().into_iter().skip(2) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_file(&path, contents.as_bytes())?;
    }

    let apps_exist =
        lifecycle.output_dir.join("android").exists() || lifecycle.output_dir.join("ios").exists();

    if let Err(e) = embed_spec_into_apps(&lifecycle.output_dir, spec) {
        if apps_exist {
            println!(
                "Warning: Failed to embed bench spec into app bundles: {}",
                e
            );
        }
    } else if apps_exist {
        println!("Embedded bench_spec.json in mobile app bundles");
    }

    let profile = if release { "release" } else { "debug" };
    let target_str = match spec.target {
        MobileTarget::Android => "android",
        MobileTarget::Ios => "ios",
    };

    if let Err(e) = embed_meta_into_apps(&lifecycle.output_dir, spec, target_str, profile) {
        if apps_exist {
            println!(
                "Warning: Failed to embed bench meta into app bundles: {}",
                e
            );
        }
    } else if apps_exist {
        println!("Embedded bench_meta.json with build metadata");
    }
    Ok(())
}

pub(crate) fn validate_default_specs(
    lifecycle: &ArtifactLifecycle<'_>,
) -> Vec<(PathBuf, Result<mobench_sdk::BenchSpec>)> {
    lifecycle
        .default_spec_paths()
        .into_iter()
        .filter(|path| path.exists())
        .map(|path| {
            let result = validate_spec_file(&path);
            (path, result)
        })
        .collect()
}

fn embed_spec_into_apps(output_dir: &Path, spec: &RunSpec) -> Result<()> {
    let embedded_spec = mobench_sdk::builders::EmbeddedBenchSpec {
        function: spec.function.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
    };
    mobench_sdk::builders::embed_bench_spec(output_dir, &embedded_spec)
        .map_err(|e| anyhow!("Failed to embed bench spec: {}", e))
}

fn embed_meta_into_apps(
    output_dir: &Path,
    spec: &RunSpec,
    target: &str,
    profile: &str,
) -> Result<()> {
    let embedded_spec = mobench_sdk::builders::EmbeddedBenchSpec {
        function: spec.function.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
    };
    mobench_sdk::builders::embed_bench_meta(output_dir, &embedded_spec, target, profile)
        .map_err(|e| anyhow!("Failed to embed bench meta: {}", e))
}

fn resolve_project_relative_path(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}
