//! Local Provider Adapter backed by a generated host harness.

use std::fs;
use std::path::{Path, PathBuf};

use mobench_process::ProcessCancellation;
use mobench_provider::{AdapterRun, CollectedOutput, ExpectedSession, ProviderAdapter};
use serde_json::Value;
use tempfile::TempDir;
use thiserror::Error;

use crate::ResolvedProjectLayout;
use crate::process_adapter::ToolCommand;

const REPORT_PREFIX: &str = "MOBENCH_LOCAL_REPORT:";

/// Resolved request executed by the Local Provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalRunRequest {
    pub(crate) function: String,
    pub(crate) iterations: u32,
    pub(crate) warmup: u32,
    pub(crate) release: bool,
}

/// Local Provider Adapter for one resolved benchmark crate.
#[derive(Debug)]
pub(crate) struct LocalProviderAdapter<'layout> {
    layout: &'layout ResolvedProjectLayout,
}

impl<'layout> LocalProviderAdapter<'layout> {
    pub(crate) const fn new(layout: &'layout ResolvedProjectLayout) -> Self {
        Self { layout }
    }
}

#[derive(Debug)]
pub(crate) struct LocalRunHandle {
    _workspace: TempDir,
    manifest_path: PathBuf,
    target_dir: PathBuf,
    request: LocalRunRequest,
    session_id: String,
}

#[derive(Debug, Error)]
pub(crate) enum LocalProviderError {
    #[error("local provider was cancelled")]
    Cancelled,
    #[error("could not create local harness workspace: {0}")]
    Workspace(#[source] std::io::Error),
    #[error("could not create local harness directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write local harness file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not inspect the benchmark dependency graph: {message}")]
    MetadataCommand { message: String },
    #[error("cargo metadata failed: {message}")]
    MetadataFailed { message: String },
    #[error("could not parse cargo metadata: {message}")]
    MetadataParse { message: String },
    #[error("benchmark crate was not present in its cargo metadata graph")]
    BenchmarkPackageMissing,
    #[error("benchmark crate does not directly depend on mobench-sdk")]
    SdkDependencyMissing,
    #[error("resolved mobench-sdk package was absent from cargo metadata")]
    SdkPackageMissing,
    #[error("local provider does not yet support mobench-sdk source {sdk_source}")]
    UnsupportedSdkSource { sdk_source: String },
    #[error("could not serialize local harness manifest: {message}")]
    Manifest { message: String },
    #[error("could not execute local harness: {message}")]
    HarnessCommand { message: String },
    #[error("local harness failed with {status}: {stderr}")]
    HarnessFailed { status: String, stderr: String },
    #[error("local harness did not emit a benchmark report")]
    MissingReport,
    #[error("local harness emitted invalid report JSON: {message}")]
    InvalidReport { message: String },
    #[error("local harness report identity mismatch: {message}")]
    IdentityMismatch { message: String },
}

#[derive(Debug)]
enum SdkDependency {
    Path(PathBuf),
    Registry(String),
}

impl ProviderAdapter for LocalProviderAdapter<'_> {
    type Request = LocalRunRequest;
    type Handle = LocalRunHandle;
    type Report = Value;
    type Error = LocalProviderError;

    fn start(
        &self,
        request: &Self::Request,
        cancellation: &ProcessCancellation,
    ) -> Result<Self::Handle, Self::Error> {
        if cancellation.is_cancelled() {
            return Err(LocalProviderError::Cancelled);
        }

        let sdk = resolve_sdk_dependency(self.layout, cancellation)?;
        let workspace = tempfile::Builder::new()
            .prefix("mobench-local-")
            .tempdir()
            .map_err(LocalProviderError::Workspace)?;
        let source_dir = workspace.path().join("src");
        fs::create_dir(&source_dir).map_err(|source| LocalProviderError::CreateDirectory {
            path: source_dir.clone(),
            source,
        })?;

        let manifest_path = workspace.path().join("Cargo.toml");
        let manifest = render_harness_manifest(self.layout, sdk)?;
        write_file(&manifest_path, manifest.as_bytes())?;
        write_file(&source_dir.join("main.rs"), harness_source().as_bytes())?;

        let local_root = self.layout.output_dir.join("local-provider");
        fs::create_dir_all(&local_root).map_err(|source| LocalProviderError::CreateDirectory {
            path: local_root.clone(),
            source,
        })?;
        let target_dir = local_root.join("target");
        let suffix = workspace
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("run")
            .to_owned();

        Ok(LocalRunHandle {
            _workspace: workspace,
            manifest_path,
            target_dir,
            request: request.clone(),
            session_id: format!("local-{suffix}"),
        })
    }

    fn collect(
        &self,
        handle: &Self::Handle,
        cancellation: &ProcessCancellation,
    ) -> Result<AdapterRun<Self::Report>, Self::Error> {
        if cancellation.is_cancelled() {
            return Err(LocalProviderError::Cancelled);
        }

        let mut command = ToolCommand::path_search("cargo");
        command
            .arg("run")
            .arg("--quiet")
            .arg("--manifest-path")
            .arg(&handle.manifest_path)
            .arg("--target-dir")
            .arg(&handle.target_dir);
        if handle.request.release {
            command.arg("--release");
        }
        command
            .arg("--")
            .arg(&handle.request.function)
            .arg(handle.request.iterations.to_string())
            .arg(handle.request.warmup.to_string())
            .current_dir(
                handle
                    .manifest_path
                    .parent()
                    .expect("local harness manifest always has a parent"),
            );

        let output = command.output_cancellable(cancellation).map_err(|error| {
            LocalProviderError::HarnessCommand {
                message: error.to_string(),
            }
        })?;
        if !output.status.success() {
            return Err(LocalProviderError::HarnessFailed {
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let report_text = stdout
            .lines()
            .find_map(|line| line.strip_prefix(REPORT_PREFIX))
            .ok_or(LocalProviderError::MissingReport)?;
        let report: Value = serde_json::from_str(report_text).map_err(|error| {
            LocalProviderError::InvalidReport {
                message: error.to_string(),
            }
        })?;
        validate_report_identity(&report, &handle.request)?;

        Ok(AdapterRun {
            expected: vec![ExpectedSession {
                session_id: handle.session_id.clone(),
                device_id: "host".to_owned(),
                status: "passed".to_owned(),
            }],
            collected: vec![CollectedOutput {
                session_id: handle.session_id.clone(),
                reports: vec![report],
                failure: None,
            }],
        })
    }

    fn cancel(
        &self,
        _handle: &Self::Handle,
        _cancellation: &ProcessCancellation,
    ) -> Result<(), Self::Error> {
        // Local execution is a supervised child scope. The shared cancellation
        // token terminates and reaps it inside mobench-process.
        Ok(())
    }
}

fn resolve_sdk_dependency(
    layout: &ResolvedProjectLayout,
    cancellation: &ProcessCancellation,
) -> Result<SdkDependency, LocalProviderError> {
    let benchmark_manifest = layout.crate_dir.join("Cargo.toml");
    let mut command = ToolCommand::path_search("cargo");
    command
        .arg("metadata")
        .arg("--format-version=1")
        .arg("--manifest-path")
        .arg(&benchmark_manifest)
        .current_dir(&layout.project_root);
    let output = command.output_cancellable(cancellation).map_err(|error| {
        LocalProviderError::MetadataCommand {
            message: error.to_string(),
        }
    })?;
    if !output.status.success() {
        return Err(LocalProviderError::MetadataFailed {
            message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        LocalProviderError::MetadataParse {
            message: error.to_string(),
        }
    })?;

    let benchmark_manifest = fs::canonicalize(&benchmark_manifest).map_err(|source| {
        LocalProviderError::MetadataCommand {
            message: format!(
                "could not canonicalize {}: {source}",
                benchmark_manifest.display()
            ),
        }
    })?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| LocalProviderError::MetadataParse {
            message: "packages was not an array".to_owned(),
        })?;
    let benchmark = packages
        .iter()
        .find(|package| {
            package
                .get("manifest_path")
                .and_then(Value::as_str)
                .and_then(|path| fs::canonicalize(path).ok())
                .is_some_and(|path| path == benchmark_manifest)
        })
        .ok_or(LocalProviderError::BenchmarkPackageMissing)?;
    let benchmark_id = benchmark.get("id").and_then(Value::as_str).ok_or_else(|| {
        LocalProviderError::MetadataParse {
            message: "benchmark package id was missing".to_owned(),
        }
    })?;
    let nodes = metadata
        .pointer("/resolve/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| LocalProviderError::MetadataParse {
            message: "resolve.nodes was not an array".to_owned(),
        })?;
    let benchmark_node = nodes
        .iter()
        .find(|node| node.get("id").and_then(Value::as_str) == Some(benchmark_id))
        .ok_or(LocalProviderError::BenchmarkPackageMissing)?;
    let sdk_id = benchmark_node
        .get("deps")
        .and_then(Value::as_array)
        .and_then(|dependencies| {
            dependencies.iter().find(|dependency| {
                dependency.get("name").and_then(Value::as_str) == Some("mobench_sdk")
            })
        })
        .and_then(|dependency| dependency.get("pkg"))
        .and_then(Value::as_str)
        .ok_or(LocalProviderError::SdkDependencyMissing)?;
    let sdk = packages
        .iter()
        .find(|package| package.get("id").and_then(Value::as_str) == Some(sdk_id))
        .ok_or(LocalProviderError::SdkPackageMissing)?;

    match sdk.get("source") {
        None | Some(Value::Null) => {
            let manifest = sdk
                .get("manifest_path")
                .and_then(Value::as_str)
                .ok_or_else(|| LocalProviderError::MetadataParse {
                    message: "path mobench-sdk manifest was missing".to_owned(),
                })?;
            Ok(SdkDependency::Path(
                Path::new(manifest)
                    .parent()
                    .expect("cargo package manifest has a parent")
                    .to_path_buf(),
            ))
        }
        Some(Value::String(source)) if source.starts_with("registry+") => {
            let version = sdk.get("version").and_then(Value::as_str).ok_or_else(|| {
                LocalProviderError::MetadataParse {
                    message: "registry mobench-sdk version was missing".to_owned(),
                }
            })?;
            Ok(SdkDependency::Registry(version.to_owned()))
        }
        Some(Value::String(source)) => Err(LocalProviderError::UnsupportedSdkSource {
            sdk_source: source.clone(),
        }),
        Some(_) => Err(LocalProviderError::MetadataParse {
            message: "mobench-sdk source was not a string".to_owned(),
        }),
    }
}

fn render_harness_manifest(
    layout: &ResolvedProjectLayout,
    sdk: SdkDependency,
) -> Result<String, LocalProviderError> {
    let mut root = toml::map::Map::new();
    root.insert(
        "package".to_owned(),
        toml::Value::Table(toml::map::Map::from_iter([
            (
                "name".to_owned(),
                toml::Value::String("mobench-local-harness".to_owned()),
            ),
            (
                "version".to_owned(),
                toml::Value::String("0.0.0".to_owned()),
            ),
            ("edition".to_owned(), toml::Value::String("2024".to_owned())),
            ("publish".to_owned(), toml::Value::Boolean(false)),
        ])),
    );
    root.insert(
        "workspace".to_owned(),
        toml::Value::Table(toml::map::Map::new()),
    );

    let mut dependencies = toml::map::Map::new();
    dependencies.insert(
        "benchmark_crate".to_owned(),
        toml::Value::Table(toml::map::Map::from_iter([
            (
                "package".to_owned(),
                toml::Value::String(layout.crate_name.clone()),
            ),
            (
                "path".to_owned(),
                toml::Value::String(layout.crate_dir.to_string_lossy().into_owned()),
            ),
        ])),
    );
    let sdk_dependency = match sdk {
        SdkDependency::Path(path) => toml::Value::Table(toml::map::Map::from_iter([(
            "path".to_owned(),
            toml::Value::String(path.to_string_lossy().into_owned()),
        )])),
        SdkDependency::Registry(version) => toml::Value::Table(toml::map::Map::from_iter([(
            "version".to_owned(),
            toml::Value::String(format!("={version}")),
        )])),
    };
    dependencies.insert("mobench-sdk".to_owned(), sdk_dependency);
    dependencies.insert("serde_json".to_owned(), toml::Value::String("1".to_owned()));
    root.insert("dependencies".to_owned(), toml::Value::Table(dependencies));

    toml::to_string(&toml::Value::Table(root)).map_err(|error| LocalProviderError::Manifest {
        message: error.to_string(),
    })
}

fn harness_source() -> &'static str {
    r#"use benchmark_crate as _;

fn main() {
    let mut arguments = std::env::args();
    let _program = arguments.next();
    let function = arguments.next().expect("missing benchmark function");
    let iterations = arguments
        .next()
        .expect("missing iterations")
        .parse::<u32>()
        .expect("invalid iterations");
    let warmup = arguments
        .next()
        .expect("missing warmup")
        .parse::<u32>()
        .expect("invalid warmup");
    assert!(arguments.next().is_none(), "unexpected harness argument");

    let report = mobench_sdk::run_benchmark(mobench_sdk::BenchSpec {
        name: function,
        iterations,
        warmup,
    })
    .expect("benchmark execution failed");
    let json = serde_json::to_string(&report).expect("benchmark report serialization failed");
    println!("MOBENCH_LOCAL_REPORT:{json}");
}
"#
}

fn validate_report_identity(
    report: &Value,
    request: &LocalRunRequest,
) -> Result<(), LocalProviderError> {
    let spec = report
        .get("spec")
        .ok_or_else(|| LocalProviderError::IdentityMismatch {
            message: "report.spec was missing".to_owned(),
        })?;
    let function = spec.get("name").and_then(Value::as_str);
    if function != Some(request.function.as_str()) {
        return Err(LocalProviderError::IdentityMismatch {
            message: format!(
                "expected function {}, observed {}",
                request.function,
                function.unwrap_or("<missing>")
            ),
        });
    }
    let iterations = spec.get("iterations").and_then(Value::as_u64);
    if iterations != Some(u64::from(request.iterations)) {
        return Err(LocalProviderError::IdentityMismatch {
            message: format!(
                "expected {} iterations, observed {:?}",
                request.iterations, iterations
            ),
        });
    }
    let warmup = spec.get("warmup").and_then(Value::as_u64);
    if warmup != Some(u64::from(request.warmup)) {
        return Err(LocalProviderError::IdentityMismatch {
            message: format!("expected {} warmups, observed {:?}", request.warmup, warmup),
        });
    }
    let samples = report.get("samples").and_then(Value::as_array);
    if samples.map(Vec::len) != Some(request.iterations as usize) {
        return Err(LocalProviderError::IdentityMismatch {
            message: format!(
                "expected {} samples, observed {:?}",
                request.iterations,
                samples.map(Vec::len)
            ),
        });
    }
    Ok(())
}

fn write_file(path: &Path, contents: &[u8]) -> Result<(), LocalProviderError> {
    fs::write(path, contents).map_err(|source| LocalProviderError::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use mobench_provider::ProviderEngine;

    use super::*;

    #[test]
    fn local_adapter_executes_the_basic_example_and_returns_real_samples() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let layout = ResolvedProjectLayout {
            project_root: root.clone(),
            crate_dir: root.join("examples/basic-benchmark"),
            crate_name: "basic-benchmark".to_owned(),
            library_name: "basic_benchmark".to_owned(),
            ffi_backend: mobench_sdk::FfiBackend::Uniffi,
            android_abis: None,
            ios_completion_timeout_secs: None,
            ios_deployment_target: "13.0".to_owned(),
            ios_runner: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
            config_path: None,
            output_dir: root.join("target/mobench/local-provider-test"),
            default_function: None,
        };
        let request = LocalRunRequest {
            function: "basic_benchmark::bench_fibonacci".to_owned(),
            iterations: 3,
            warmup: 1,
            release: false,
        };
        let engine = ProviderEngine::new(LocalProviderAdapter::new(&layout));
        let run = engine
            .execute(&request, &ProcessCancellation::default())
            .expect("execute local provider");

        assert!(run.assessment().is_complete());
        let report = &run.sessions()[0].reports[0];
        assert_eq!(report["spec"]["name"], request.function);
        assert_eq!(report["samples"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn local_report_validation_rejects_wrong_function_and_counts() {
        let request = LocalRunRequest {
            function: "crate::expected".to_owned(),
            iterations: 2,
            warmup: 1,
            release: false,
        };
        let report = serde_json::json!({
            "spec": { "name": "crate::wrong", "iterations": 2, "warmup": 1 },
            "samples": [{}, {}]
        });
        assert!(matches!(
            validate_report_identity(&report, &request),
            Err(LocalProviderError::IdentityMismatch { .. })
        ));
    }
}
