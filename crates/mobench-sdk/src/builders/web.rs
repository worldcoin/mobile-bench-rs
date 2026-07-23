//! Build automation for browser-hosted WebAssembly benchmarks.

use super::common::{get_cargo_target_dir, run_command, validate_project_root};
use crate::{BenchError, BuildProfile};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const WASM_TARGET: &str = "wasm32-unknown-unknown";
const WEB_GLUE_NAME: &str = "mobench_web";
const INDEX_HTML: &str = include_str!("../../templates/web/index.html");
const RUNNER_JS: &str = include_str!("../../templates/web/runner.js");

/// Configuration for a generated web benchmark bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebBuildConfig {
    pub profile: BuildProfile,
}

impl Default for WebBuildConfig {
    fn default() -> Self {
        Self {
            profile: BuildProfile::Debug,
        }
    }
}

/// Paths produced by [`WebBuilder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebBuildResult {
    pub bundle_dir: PathBuf,
    pub index_html: PathBuf,
    pub runner_js: PathBuf,
    pub bindgen_js: PathBuf,
    pub wasm: PathBuf,
    pub manifest: PathBuf,
}

/// Builds a benchmark crate for the browser and generates a static harness.
pub struct WebBuilder {
    project_root: PathBuf,
    crate_name: String,
    library_name: String,
    crate_dir: Option<PathBuf>,
    output_dir: PathBuf,
    wasm_bindgen: PathBuf,
    verbose: bool,
    dry_run: bool,
}

impl WebBuilder {
    pub fn new(project_root: impl Into<PathBuf>, crate_name: impl Into<String>) -> Self {
        let project_root = project_root.into();
        let crate_name = crate_name.into();
        Self {
            output_dir: project_root.join("target/mobench"),
            library_name: crate_name.replace('-', "_"),
            project_root,
            crate_name,
            crate_dir: None,
            wasm_bindgen: PathBuf::from("wasm-bindgen"),
            verbose: false,
            dry_run: false,
        }
    }

    pub fn library_name(mut self, library_name: impl Into<String>) -> Self {
        self.library_name = library_name.into();
        self
    }

    pub fn crate_dir(mut self, crate_dir: impl Into<PathBuf>) -> Self {
        self.crate_dir = Some(crate_dir.into());
        self
    }

    pub fn output_dir(mut self, output_dir: impl Into<PathBuf>) -> Self {
        self.output_dir = output_dir.into();
        self
    }

    pub fn wasm_bindgen(mut self, wasm_bindgen: impl Into<PathBuf>) -> Self {
        self.wasm_bindgen = wasm_bindgen.into();
        self
    }

    pub fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    pub fn dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    pub fn build(&self, config: &WebBuildConfig) -> Result<WebBuildResult, BenchError> {
        validate_project_root(&self.project_root, &self.crate_name)?;
        let crate_dir = self.resolve_crate_dir()?;
        let bundle_dir = self.output_dir.join("web");
        let result = WebBuildResult {
            index_html: bundle_dir.join("index.html"),
            runner_js: bundle_dir.join("runner.js"),
            bindgen_js: bundle_dir.join(format!("{WEB_GLUE_NAME}.js")),
            wasm: bundle_dir.join(format!("{WEB_GLUE_NAME}_bg.wasm")),
            manifest: bundle_dir.join("mobench-web.json"),
            bundle_dir,
        };

        let profile = config.profile.as_str();
        let target_dir = get_cargo_target_dir(&crate_dir)?;
        let input_wasm = target_dir
            .join(WASM_TARGET)
            .join(profile)
            .join(format!("{}.wasm", self.library_name));

        if self.verbose || self.dry_run {
            println!(
                "{} cargo build --manifest-path {} --target {WASM_TARGET}{}",
                if self.dry_run { "[dry-run]" } else { "[web]" },
                crate_dir.join("Cargo.toml").display(),
                if config.profile == BuildProfile::Release {
                    " --release"
                } else {
                    ""
                }
            );
            println!(
                "{} {} {} --target web --out-dir {} --out-name {WEB_GLUE_NAME}",
                if self.dry_run { "[dry-run]" } else { "[web]" },
                self.wasm_bindgen.display(),
                input_wasm.display(),
                result.bundle_dir.display()
            );
        }
        if self.dry_run {
            return Ok(result);
        }

        let mut cargo = Command::new("cargo");
        cargo
            .arg("build")
            .arg("--manifest-path")
            .arg(crate_dir.join("Cargo.toml"))
            .arg("--target")
            .arg(WASM_TARGET);
        if config.profile == BuildProfile::Release {
            cargo.arg("--release");
        }
        run_command(cargo, "web benchmark Cargo build")?;
        if !input_wasm.is_file() {
            return Err(BenchError::Build(format!(
                "WebAssembly artifact was not produced at {}.\n\n\
                 Ensure the benchmark crate has:\n\
                 [lib]\n\
                 crate-type = [\"lib\", \"cdylib\"]",
                input_wasm.display()
            )));
        }

        fs::create_dir_all(&result.bundle_dir).map_err(|error| {
            BenchError::Build(format!(
                "Failed to create web bundle directory {}: {error}",
                result.bundle_dir.display()
            ))
        })?;
        let mut bindgen = Command::new(&self.wasm_bindgen);
        bindgen
            .arg(&input_wasm)
            .arg("--target")
            .arg("web")
            .arg("--out-dir")
            .arg(&result.bundle_dir)
            .arg("--out-name")
            .arg(WEB_GLUE_NAME)
            .arg("--no-typescript");
        run_command(bindgen, "wasm-bindgen web glue generation")?;

        write_bundle_file(&result.index_html, INDEX_HTML)?;
        write_bundle_file(&result.runner_js, RUNNER_JS)?;
        let manifest = WebBundleManifest {
            schema: "mobench.web-bundle.v1",
            crate_name: &self.crate_name,
            library_name: &self.library_name,
            profile,
            target: WASM_TARGET,
            entrypoint: "index.html",
            runner: "runner.js",
            javascript: "mobench_web.js",
            wasm: "mobench_web_bg.wasm",
        };
        let manifest_json = serde_json::to_string_pretty(&manifest).map_err(|error| {
            BenchError::Build(format!("Failed to serialize web bundle manifest: {error}"))
        })?;
        write_bundle_file(&result.manifest, &manifest_json)?;

        for required in [
            &result.index_html,
            &result.runner_js,
            &result.bindgen_js,
            &result.wasm,
            &result.manifest,
        ] {
            if !required.is_file() {
                return Err(BenchError::Build(format!(
                    "Web bundle is incomplete; expected {}",
                    required.display()
                )));
            }
        }

        Ok(result)
    }

    fn resolve_crate_dir(&self) -> Result<PathBuf, BenchError> {
        if let Some(crate_dir) = &self.crate_dir {
            if crate_dir.join("Cargo.toml").is_file() {
                return Ok(crate_dir.clone());
            }
            return Err(BenchError::Build(format!(
                "Benchmark crate Cargo.toml not found at {}",
                crate_dir.join("Cargo.toml").display()
            )));
        }

        for candidate in [
            self.project_root.clone(),
            self.project_root.join("bench-mobile"),
            self.project_root.join("crates").join(&self.crate_name),
            self.project_root.join(&self.crate_name),
        ] {
            if candidate.join("Cargo.toml").is_file() {
                return Ok(candidate);
            }
        }
        Err(BenchError::Build(format!(
            "Could not locate benchmark crate '{}' under {}",
            self.crate_name,
            self.project_root.display()
        )))
    }
}

#[derive(Serialize)]
struct WebBundleManifest<'a> {
    schema: &'static str,
    crate_name: &'a str,
    library_name: &'a str,
    profile: &'a str,
    target: &'static str,
    entrypoint: &'static str,
    runner: &'static str,
    javascript: &'static str,
    wasm: &'static str,
}

fn write_bundle_file(path: &Path, contents: &str) -> Result<(), BenchError> {
    fs::write(path, contents)
        .map_err(|error| BenchError::Build(format!("Failed to write {}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn dry_run_returns_deterministic_bundle_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"demo-bench\"\nversion = \"0.1.0\"\n",
        )
        .expect("manifest");
        let output = temp.path().join("out");

        let result = WebBuilder::new(temp.path(), "demo-bench")
            .output_dir(&output)
            .dry_run(true)
            .build(&WebBuildConfig {
                profile: BuildProfile::Release,
            })
            .expect("dry-run web build");

        assert_eq!(result.bundle_dir, output.join("web"));
        assert_eq!(result.index_html, output.join("web/index.html"));
        assert_eq!(result.wasm, output.join("web/mobench_web_bg.wasm"));
        assert!(!result.bundle_dir.exists());
    }

    #[test]
    fn templates_expose_stable_window_contract() {
        assert!(INDEX_HTML.contains("runner.js"));
        assert!(RUNNER_JS.contains("window.mobench"));
        assert!(RUNNER_JS.contains("runBenchmarkJson"));
        assert!(RUNNER_JS.contains("JSON.parse"));
    }
}
