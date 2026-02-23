//! UniFFI integration helpers for generating mobile bindings.
//!
//! This module provides utilities for integrating mobench-sdk types with UniFFI
//! for generating Kotlin/Swift bindings. Since UniFFI requires scaffolding to be
//! set up in the consuming crate, this module provides conversion traits and
//! ready-to-use type definitions that can be easily adapted.
//!
//! ## Quick Start
//!
//! To use mobench-sdk with UniFFI in your crate:
//!
//! 1. Add uniffi to your dependencies:
//!
//! ```toml
//! [dependencies]
//! mobench-sdk = "0.1"
//! uniffi = { version = "0.28", features = ["cli"] }
//!
//! [build-dependencies]
//! uniffi = { version = "0.28", features = ["build"] }
//! ```
//!
//! 2. Define your FFI types with UniFFI annotations:
//!
//! ```ignore
//! use uniffi;
//!
//! // Set up UniFFI scaffolding
//! uniffi::setup_scaffolding!();
//!
//! #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
//! pub struct BenchSpec {
//!     pub name: String,
//!     pub iterations: u32,
//!     pub warmup: u32,
//! }
//!
//! #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
//! pub struct BenchSample {
//!     pub duration_ns: u64,
//! }
//!
//! #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
//! pub struct BenchReport {
//!     pub spec: BenchSpec,
//!     pub samples: Vec<BenchSample>,
//! }
//!
//! #[derive(Debug, thiserror::Error, uniffi::Error)]
//! #[uniffi(flat_error)]
//! pub enum BenchError {
//!     #[error("iterations must be greater than zero")]
//!     InvalidIterations,
//!     #[error("unknown benchmark function: {name}")]
//!     UnknownFunction { name: String },
//!     #[error("benchmark execution failed: {reason}")]
//!     ExecutionFailed { reason: String },
//! }
//! ```
//!
//! 3. Implement conversions using the traits from this module:
//!
//! ```ignore
//! use mobench_sdk::uniffi_types::{FromSdkSpec, FromSdkSample, FromSdkReport, FromSdkError};
//!
//! impl FromSdkSpec for BenchSpec {
//!     fn from_sdk(spec: mobench_sdk::BenchSpec) -> Self {
//!         Self {
//!             name: spec.name,
//!             iterations: spec.iterations,
//!             warmup: spec.warmup,
//!         }
//!     }
//!
//!     fn to_sdk(&self) -> mobench_sdk::BenchSpec {
//!         mobench_sdk::BenchSpec {
//!             name: self.name.clone(),
//!             iterations: self.iterations,
//!             warmup: self.warmup,
//!         }
//!     }
//! }
//!
//! // ... implement other traits similarly
//! ```
//!
//! 4. Export your benchmark function:
//!
//! ```ignore
//! #[uniffi::export]
//! pub fn run_benchmark(spec: BenchSpec) -> Result<BenchReport, BenchError> {
//!     let sdk_spec = spec.to_sdk();
//!     let sdk_report = mobench_sdk::run_benchmark(sdk_spec)?;
//!     Ok(BenchReport::from_sdk_report(sdk_report))
//! }
//! ```
//!
//! ## Complete Example
//!
//! See the `examples/ffi-benchmark` directory for a complete working example.

use serde::{Deserialize, Serialize};

/// Trait for converting from SDK's BenchSpec type.
///
/// Implement this trait on your UniFFI-annotated BenchSpec type.
pub trait FromSdkSpec: Sized {
    /// Convert from the SDK's BenchSpec type.
    fn from_sdk(spec: crate::BenchSpec) -> Self;

    /// Convert to the SDK's BenchSpec type.
    fn to_sdk(&self) -> crate::BenchSpec;
}

/// Trait for converting from SDK's BenchSample type.
///
/// Implement this trait on your UniFFI-annotated BenchSample type.
pub trait FromSdkSample: Sized {
    /// Convert from the SDK's BenchSample type.
    fn from_sdk(sample: crate::BenchSample) -> Self;

    /// Convert to the SDK's BenchSample type.
    fn to_sdk(&self) -> crate::BenchSample;
}

/// Trait for converting from SDK's RunnerReport type.
///
/// Implement this trait on your UniFFI-annotated BenchReport type.
pub trait FromSdkReport<Spec: FromSdkSpec, Sample: FromSdkSample>: Sized {
    /// Convert from the SDK's RunnerReport type.
    fn from_sdk_report(report: crate::RunnerReport) -> Self;
}

/// Trait for converting from SDK's BenchError type.
///
/// Implement this trait on your UniFFI-annotated error type.
pub trait FromSdkError: Sized {
    /// Convert from the SDK's BenchError type.
    fn from_sdk(err: crate::types::BenchError) -> Self;
}

/// Pre-defined BenchSpec structure matching SDK's BenchSpec.
///
/// This struct can be used as a template for your own UniFFI-annotated type.
/// Copy this definition and add the `#[derive(uniffi::Record)]` attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchSpecTemplate {
    /// Name of the benchmark function to run.
    pub name: String,
    /// Number of measurement iterations.
    pub iterations: u32,
    /// Number of warmup iterations before measurement.
    pub warmup: u32,
}

impl From<crate::BenchSpec> for BenchSpecTemplate {
    fn from(spec: crate::BenchSpec) -> Self {
        Self {
            name: spec.name,
            iterations: spec.iterations,
            warmup: spec.warmup,
        }
    }
}

impl From<BenchSpecTemplate> for crate::BenchSpec {
    fn from(spec: BenchSpecTemplate) -> Self {
        Self {
            name: spec.name,
            iterations: spec.iterations,
            warmup: spec.warmup,
        }
    }
}

/// Pre-defined BenchSample structure matching SDK's BenchSample.
///
/// This struct can be used as a template for your own UniFFI-annotated type.
/// Copy this definition and add the `#[derive(uniffi::Record)]` attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchSampleTemplate {
    /// Duration of the iteration in nanoseconds.
    pub duration_ns: u64,
}

impl From<crate::BenchSample> for BenchSampleTemplate {
    fn from(sample: crate::BenchSample) -> Self {
        Self {
            duration_ns: sample.duration_ns,
        }
    }
}

impl From<BenchSampleTemplate> for crate::BenchSample {
    fn from(sample: BenchSampleTemplate) -> Self {
        Self {
            duration_ns: sample.duration_ns,
        }
    }
}

/// Pre-defined BenchReport structure matching SDK's RunnerReport.
///
/// This struct can be used as a template for your own UniFFI-annotated type.
/// Copy this definition and add the `#[derive(uniffi::Record)]` attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReportTemplate {
    /// The specification used for this benchmark run.
    pub spec: BenchSpecTemplate,
    /// All collected timing samples.
    pub samples: Vec<BenchSampleTemplate>,
}

impl From<crate::RunnerReport> for BenchReportTemplate {
    fn from(report: crate::RunnerReport) -> Self {
        Self {
            spec: report.spec.into(),
            samples: report.samples.into_iter().map(Into::into).collect(),
        }
    }
}

/// Error variant enum for UniFFI integration.
///
/// This enum provides the standard error variants. Copy this and add
/// `#[derive(uniffi::Error)]` and `#[uniffi(flat_error)]` attributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BenchErrorVariant {
    /// The iteration count was zero.
    InvalidIterations,
    /// The requested benchmark function was not found.
    UnknownFunction { name: String },
    /// An error occurred during benchmark execution.
    ExecutionFailed { reason: String },
    /// Configuration error.
    ConfigError { message: String },
    /// I/O error.
    IoError { message: String },
}

impl From<crate::types::BenchError> for BenchErrorVariant {
    fn from(err: crate::types::BenchError) -> Self {
        match err {
            crate::types::BenchError::Runner(runner_err) => match runner_err {
                crate::timing::TimingError::NoIterations { .. } => {
                    BenchErrorVariant::InvalidIterations
                }
                crate::timing::TimingError::Execution(msg) => {
                    BenchErrorVariant::ExecutionFailed { reason: msg }
                }
            },
            crate::types::BenchError::UnknownFunction(name, _available) => {
                BenchErrorVariant::UnknownFunction { name }
            }
            crate::types::BenchError::Execution(msg) => {
                BenchErrorVariant::ExecutionFailed { reason: msg }
            }
            crate::types::BenchError::Io(e) => BenchErrorVariant::IoError {
                message: e.to_string(),
            },
            crate::types::BenchError::Serialization(e) => BenchErrorVariant::ConfigError {
                message: e.to_string(),
            },
            crate::types::BenchError::Config(msg) => {
                BenchErrorVariant::ConfigError { message: msg }
            }
            crate::types::BenchError::Build(msg) => BenchErrorVariant::ExecutionFailed {
                reason: format!("build error: {}", msg),
            },
        }
    }
}

impl From<crate::timing::TimingError> for BenchErrorVariant {
    fn from(err: crate::timing::TimingError) -> Self {
        match err {
            crate::timing::TimingError::NoIterations { .. } => BenchErrorVariant::InvalidIterations,
            crate::timing::TimingError::Execution(msg) => {
                BenchErrorVariant::ExecutionFailed { reason: msg }
            }
        }
    }
}

/// Helper function to run a benchmark and convert result to template types.
///
/// This is useful for implementing your own `run_benchmark` FFI function:
///
/// ```ignore
/// #[uniffi::export]
/// pub fn run_benchmark(spec: BenchSpec) -> Result<BenchReport, BenchError> {
///     let sdk_spec: mobench_sdk::BenchSpec = spec.into();
///     let template_result = mobench_sdk::uniffi_types::run_benchmark_template(sdk_spec);
///     match template_result {
///         Ok(report) => Ok(BenchReport::from(report)),
///         Err(err) => Err(BenchError::from(err)),
///     }
/// }
/// ```
#[cfg(feature = "full")]
pub fn run_benchmark_template(
    spec: crate::BenchSpec,
) -> Result<BenchReportTemplate, BenchErrorVariant> {
    crate::run_benchmark(spec)
        .map(Into::into)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bench_spec_template_conversion() {
        let sdk_spec = crate::BenchSpec {
            name: "test".to_string(),
            iterations: 100,
            warmup: 10,
        };

        let template: BenchSpecTemplate = sdk_spec.clone().into();
        assert_eq!(template.name, "test");
        assert_eq!(template.iterations, 100);
        assert_eq!(template.warmup, 10);

        let back: crate::BenchSpec = template.into();
        assert_eq!(back.name, sdk_spec.name);
        assert_eq!(back.iterations, sdk_spec.iterations);
        assert_eq!(back.warmup, sdk_spec.warmup);
    }

    #[test]
    fn test_bench_sample_template_conversion() {
        let sdk_sample = crate::BenchSample { duration_ns: 12345 };
        let template: BenchSampleTemplate = sdk_sample.into();
        assert_eq!(template.duration_ns, 12345);
    }

    #[test]
    fn test_bench_error_variant_conversion() {
        let err = crate::types::BenchError::UnknownFunction(
            "test_func".to_string(),
            vec!["available_func".to_string()],
        );
        let variant: BenchErrorVariant = err.into();
        match variant {
            BenchErrorVariant::UnknownFunction { name } => assert_eq!(name, "test_func"),
            _ => panic!("Expected UnknownFunction variant"),
        }
    }
}
