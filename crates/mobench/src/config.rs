//! Configuration file support for mobench.
//!
//! This module provides support for `mobench.toml` configuration files that allow
//! users to persist project settings and avoid passing CLI flags repeatedly.
//!
//! ## Configuration File Location
//!
//! The configuration file is searched for in the following order:
//! 1. Current working directory (`./mobench.toml`)
//! 2. Parent directories (up to the repository root or filesystem root)
//!
//! ## Example Configuration
//!
//! ```toml
//! [project]
//! crate = "bench-mobile"
//! library_name = "bench_mobile"
//!
//! [android]
//! package = "com.example.bench"
//! min_sdk = 24
//! target_sdk = 34
//!
//! [ios]
//! bundle_id = "com.example.bench"
//! deployment_target = "15.0"
//!
//! [benchmarks]
//! default_function = "my_crate::my_benchmark"
//! default_iterations = 100
//! default_warmup = 10
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The default configuration file name.
pub const CONFIG_FILE_NAME: &str = "mobench.toml";

/// Root configuration structure for `mobench.toml`.
///
/// This struct represents the complete configuration file format and is
/// automatically loaded when CLI commands run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MobenchConfig {
    /// Project-level configuration.
    pub project: ProjectConfig,

    /// Android-specific configuration.
    pub android: AndroidConfig,

    /// iOS-specific configuration.
    pub ios: IosConfig,

    /// Benchmark execution defaults.
    pub benchmarks: BenchmarksConfig,
}

/// Project-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    /// Name of the benchmark crate (e.g., "bench-mobile").
    ///
    /// If not specified, mobench will auto-detect the crate by looking for
    /// `bench-mobile/` or `crates/sample-fns/` directories.
    #[serde(rename = "crate")]
    pub crate_name: Option<String>,

    /// Library name for the Rust crate (e.g., "bench_mobile").
    ///
    /// This is typically the crate name with hyphens replaced by underscores.
    /// If not specified, it's derived from the crate name.
    pub library_name: Option<String>,

    /// Output directory for build artifacts.
    ///
    /// Defaults to `target/mobench/` if not specified.
    pub output_dir: Option<PathBuf>,
}

/// Android-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AndroidConfig {
    /// Android package name (e.g., "com.example.bench").
    ///
    /// Defaults to "dev.world.bench" if not specified.
    pub package: String,

    /// Minimum Android SDK version.
    ///
    /// Defaults to 24 (Android 7.0).
    pub min_sdk: u32,

    /// Target Android SDK version.
    ///
    /// Defaults to 34 (Android 14).
    pub target_sdk: u32,

    /// Android ABIs to build for.
    ///
    /// Defaults to ["arm64-v8a", "armeabi-v7a", "x86_64"].
    pub abis: Option<Vec<String>>,
}

impl Default for AndroidConfig {
    fn default() -> Self {
        Self {
            package: "dev.world.bench".to_string(),
            min_sdk: 24,
            target_sdk: 34,
            abis: None,
        }
    }
}

/// iOS-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IosConfig {
    /// iOS bundle identifier (e.g., "com.example.bench").
    ///
    /// Defaults to "dev.world.bench" if not specified.
    pub bundle_id: String,

    /// iOS deployment target version.
    ///
    /// Defaults to "15.0".
    pub deployment_target: String,

    /// Development team ID for code signing.
    ///
    /// If not specified, ad-hoc signing is used.
    pub team_id: Option<String>,
}

impl Default for IosConfig {
    fn default() -> Self {
        Self {
            bundle_id: "dev.world.bench".to_string(),
            deployment_target: "15.0".to_string(),
            team_id: None,
        }
    }
}

/// Benchmark execution defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BenchmarksConfig {
    /// Default benchmark function to run.
    ///
    /// Can be overridden via CLI `--function` flag.
    pub default_function: Option<String>,

    /// Default number of benchmark iterations.
    ///
    /// Defaults to 100. Can be overridden via CLI `--iterations` flag.
    pub default_iterations: u32,

    /// Default number of warmup iterations.
    ///
    /// Defaults to 10. Can be overridden via CLI `--warmup` flag.
    pub default_warmup: u32,
}

impl Default for BenchmarksConfig {
    fn default() -> Self {
        Self {
            default_function: None,
            default_iterations: 100,
            default_warmup: 10,
        }
    }
}

impl MobenchConfig {
    /// Creates a new configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads configuration from the specified file path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the configuration file
    ///
    /// # Returns
    ///
    /// * `Ok(MobenchConfig)` - Successfully loaded configuration
    /// * `Err` - If the file cannot be read or parsed
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;

        let config: MobenchConfig = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {:?}", path))?;

        Ok(config)
    }

    /// Attempts to find and load configuration from the current directory
    /// or any parent directory.
    ///
    /// This searches for `mobench.toml` starting from the current directory
    /// and walking up the directory tree until a config file is found or
    /// the root is reached.
    ///
    /// # Returns
    ///
    /// * `Ok(Some((config, path)))` - Found and loaded configuration with its path
    /// * `Ok(None)` - No configuration file found
    /// * `Err` - If a config file was found but couldn't be parsed
    pub fn discover() -> Result<Option<(Self, PathBuf)>> {
        let cwd = std::env::current_dir().context("Failed to get current directory")?;
        Self::discover_from(&cwd)
    }

    /// Attempts to find and load configuration starting from the specified directory.
    ///
    /// # Arguments
    ///
    /// * `start_dir` - Directory to start searching from
    ///
    /// # Returns
    ///
    /// * `Ok(Some((config, path)))` - Found and loaded configuration with its path
    /// * `Ok(None)` - No configuration file found
    /// * `Err` - If a config file was found but couldn't be parsed
    pub fn discover_from(start_dir: &Path) -> Result<Option<(Self, PathBuf)>> {
        let mut current = start_dir.to_path_buf();

        loop {
            let config_path = current.join(CONFIG_FILE_NAME);

            if config_path.is_file() {
                let config = Self::load_from_file(&config_path)?;
                return Ok(Some((config, config_path)));
            }

            // Stop at repository root or filesystem root
            if current.join(".git").exists() || !current.pop() {
                break;
            }
        }

        Ok(None)
    }

    /// Saves the configuration to the specified file path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to write the configuration file
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Successfully saved configuration
    /// * `Err` - If the file cannot be written
    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        let contents = toml::to_string_pretty(self).context("Failed to serialize configuration")?;

        std::fs::write(path, contents)
            .with_context(|| format!("Failed to write config file: {:?}", path))?;

        Ok(())
    }

    /// Returns the library name, either from config or derived from crate name.
    pub fn library_name(&self) -> Option<String> {
        self.project.library_name.clone().or_else(|| {
            self.project
                .crate_name
                .as_ref()
                .map(|c| c.replace('-', "_"))
        })
    }

    /// Generates a starter configuration with sensible defaults.
    ///
    /// # Arguments
    ///
    /// * `crate_name` - Name of the benchmark crate
    ///
    /// # Returns
    ///
    /// A new `MobenchConfig` with the provided crate name and default values.
    pub fn starter(crate_name: &str) -> Self {
        let library_name = crate_name.replace('-', "_");
        let package = format!("dev.world.{}", library_name.replace('_', ""));

        Self {
            project: ProjectConfig {
                crate_name: Some(crate_name.to_string()),
                library_name: Some(library_name.clone()),
                output_dir: None, // Use default (target/mobench/)
            },
            android: AndroidConfig {
                package: package.clone(),
                min_sdk: 24,
                target_sdk: 34,
                abis: None,
            },
            ios: IosConfig {
                bundle_id: package,
                deployment_target: "15.0".to_string(),
                team_id: None,
            },
            benchmarks: BenchmarksConfig {
                default_function: Some(format!("{}::my_benchmark", library_name)),
                default_iterations: 100,
                default_warmup: 10,
            },
        }
    }

    /// Generates a starter configuration file as a formatted TOML string.
    ///
    /// This includes helpful comments explaining each configuration option.
    ///
    /// # Arguments
    ///
    /// * `crate_name` - Name of the benchmark crate
    ///
    /// # Returns
    ///
    /// A formatted TOML string suitable for writing to `mobench.toml`.
    pub fn generate_starter_toml(crate_name: &str) -> String {
        let library_name = crate_name.replace('-', "_");
        let package = format!("dev.world.{}", library_name.replace('_', ""));

        format!(
            r#"# mobench configuration file
# This file configures mobench for building and running mobile benchmarks.
# CLI flags override these settings when provided.

[project]
# Name of the benchmark crate
crate = "{crate_name}"

# Rust library name (typically crate name with hyphens replaced by underscores)
library_name = "{library_name}"

# Output directory for build artifacts (default: target/mobench/)
# output_dir = "target/mobench"

[android]
# Android package name
package = "{package}"

# Minimum Android SDK version (default: 24 / Android 7.0)
min_sdk = 24

# Target Android SDK version (default: 34 / Android 14)
target_sdk = 34

# Android ABIs to build for (optional, defaults to all supported ABIs)
# abis = ["arm64-v8a", "armeabi-v7a", "x86_64"]

[ios]
# iOS bundle identifier
bundle_id = "{package}"

# iOS deployment target version (default: 15.0)
deployment_target = "15.0"

# Development team ID for code signing (optional, uses ad-hoc signing if not set)
# team_id = "YOUR_TEAM_ID"

[benchmarks]
# Default benchmark function to run
default_function = "{library_name}::my_benchmark"

# Default number of benchmark iterations (can be overridden with --iterations)
default_iterations = 100

# Default number of warmup iterations (can be overridden with --warmup)
default_warmup = 10
"#,
            crate_name = crate_name,
            library_name = library_name,
            package = package,
        )
    }
}

/// Configuration resolver that merges config file values with CLI arguments.
///
/// CLI arguments always take precedence over config file values.
#[derive(Debug, Default)]
pub struct ConfigResolver {
    /// Loaded configuration, if any.
    pub config: Option<MobenchConfig>,

    /// Path to the loaded config file, if any.
    pub config_path: Option<PathBuf>,
}

impl ConfigResolver {
    /// Creates a new resolver by discovering and loading configuration.
    ///
    /// If no config file is found, the resolver will use default values
    /// which can be overridden by CLI arguments.
    pub fn new() -> Result<Self> {
        match MobenchConfig::discover()? {
            Some((config, path)) => Ok(Self {
                config: Some(config),
                config_path: Some(path),
            }),
            None => Ok(Self {
                config: None,
                config_path: None,
            }),
        }
    }

    /// Returns the crate name from config, or None if not configured.
    pub fn crate_name(&self) -> Option<&str> {
        self.config
            .as_ref()
            .and_then(|c| c.project.crate_name.as_deref())
    }

    /// Returns the library name from config, derived from crate name if needed.
    pub fn library_name(&self) -> Option<String> {
        self.config.as_ref().and_then(|c| c.library_name())
    }

    /// Returns the output directory from config.
    pub fn output_dir(&self) -> Option<&Path> {
        self.config
            .as_ref()
            .and_then(|c| c.project.output_dir.as_deref())
    }

    /// Returns the default function from config.
    pub fn default_function(&self) -> Option<&str> {
        self.config
            .as_ref()
            .and_then(|c| c.benchmarks.default_function.as_deref())
    }

    /// Returns the default iterations from config.
    pub fn default_iterations(&self) -> u32 {
        self.config
            .as_ref()
            .map(|c| c.benchmarks.default_iterations)
            .unwrap_or(100)
    }

    /// Returns the default warmup from config.
    pub fn default_warmup(&self) -> u32 {
        self.config
            .as_ref()
            .map(|c| c.benchmarks.default_warmup)
            .unwrap_or(10)
    }

    /// Returns the Android configuration.
    pub fn android(&self) -> AndroidConfig {
        self.config
            .as_ref()
            .map(|c| c.android.clone())
            .unwrap_or_default()
    }

    /// Returns the iOS configuration.
    pub fn ios(&self) -> IosConfig {
        self.config
            .as_ref()
            .map(|c| c.ios.clone())
            .unwrap_or_default()
    }

    /// Resolves a CLI value, using config as fallback.
    ///
    /// # Arguments
    ///
    /// * `cli_value` - Value from CLI argument (None if not provided)
    /// * `config_getter` - Function to get value from config
    /// * `default` - Default value if neither CLI nor config provides a value
    ///
    /// # Returns
    ///
    /// The resolved value, preferring CLI over config over default.
    pub fn resolve<T, F>(&self, cli_value: Option<T>, config_getter: F, default: T) -> T
    where
        F: FnOnce(&MobenchConfig) -> Option<T>,
    {
        cli_value
            .or_else(|| self.config.as_ref().and_then(config_getter))
            .unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = MobenchConfig::default();
        assert_eq!(config.android.min_sdk, 24);
        assert_eq!(config.android.target_sdk, 34);
        assert_eq!(config.ios.deployment_target, "15.0");
        assert_eq!(config.benchmarks.default_iterations, 100);
        assert_eq!(config.benchmarks.default_warmup, 10);
    }

    #[test]
    fn test_starter_config() {
        let config = MobenchConfig::starter("my-bench");
        assert_eq!(config.project.crate_name, Some("my-bench".to_string()));
        assert_eq!(config.project.library_name, Some("my_bench".to_string()));
        assert_eq!(config.android.package, "dev.world.mybench");
        assert_eq!(config.ios.bundle_id, "dev.world.mybench");
    }

    #[test]
    fn test_load_from_file() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("mobench.toml");

        let toml_content = r#"
[project]
crate = "test-bench"
library_name = "test_bench"

[android]
package = "com.test.bench"
min_sdk = 21
target_sdk = 33

[ios]
bundle_id = "com.test.bench"
deployment_target = "14.0"

[benchmarks]
default_function = "test_bench::test_fn"
default_iterations = 50
default_warmup = 5
"#;

        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = MobenchConfig::load_from_file(&config_path).unwrap();

        assert_eq!(config.project.crate_name, Some("test-bench".to_string()));
        assert_eq!(config.project.library_name, Some("test_bench".to_string()));
        assert_eq!(config.android.package, "com.test.bench");
        assert_eq!(config.android.min_sdk, 21);
        assert_eq!(config.android.target_sdk, 33);
        assert_eq!(config.ios.bundle_id, "com.test.bench");
        assert_eq!(config.ios.deployment_target, "14.0");
        assert_eq!(
            config.benchmarks.default_function,
            Some("test_bench::test_fn".to_string())
        );
        assert_eq!(config.benchmarks.default_iterations, 50);
        assert_eq!(config.benchmarks.default_warmup, 5);
    }

    #[test]
    fn test_discover_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("mobench.toml");

        let toml_content = r#"
[project]
crate = "discovered-bench"
"#;

        std::fs::write(&config_path, toml_content).unwrap();

        let result = MobenchConfig::discover_from(temp_dir.path()).unwrap();
        assert!(result.is_some());

        let (config, path) = result.unwrap();
        assert_eq!(
            config.project.crate_name,
            Some("discovered-bench".to_string())
        );
        assert_eq!(path, config_path);
    }

    #[test]
    fn test_discover_no_config() {
        let temp_dir = TempDir::new().unwrap();
        // Create a .git directory to stop the search
        std::fs::create_dir(temp_dir.path().join(".git")).unwrap();

        let result = MobenchConfig::discover_from(temp_dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_config_resolver() {
        let config = MobenchConfig::starter("test-crate");
        let resolver = ConfigResolver {
            config: Some(config),
            config_path: None,
        };

        // CLI value takes precedence
        let result = resolver.resolve(Some(200), |c| Some(c.benchmarks.default_iterations), 50);
        assert_eq!(result, 200);

        // Config value used when CLI is None
        let result: u32 = resolver.resolve(None, |c| Some(c.benchmarks.default_iterations), 50);
        assert_eq!(result, 100);
    }

    #[test]
    fn test_generate_starter_toml() {
        let toml = MobenchConfig::generate_starter_toml("my-bench");
        assert!(toml.contains("crate = \"my-bench\""));
        assert!(toml.contains("library_name = \"my_bench\""));
        assert!(toml.contains("min_sdk = 24"));
        assert!(toml.contains("target_sdk = 34"));
        assert!(toml.contains("deployment_target = \"15.0\""));
        assert!(toml.contains("default_iterations = 100"));
        assert!(toml.contains("default_warmup = 10"));
    }
}
