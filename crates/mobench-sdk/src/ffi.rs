//! Unified FFI module for UniFFI integration.
//!
//! This module provides a single import point for all FFI-related types and traits
//! needed to create UniFFI bindings for mobile platforms.
//!
//! # Quick Start
//!
//! ```ignore
//! use mobench_sdk::ffi::{BenchSpecFfi, BenchSampleFfi, BenchReportFfi, BenchErrorFfi};
//! use mobench_sdk::ffi::{IntoFfi, FromFfi};
//!
//! // Define your UniFFI types using the Ffi suffix types as templates
//! #[derive(uniffi::Record)]
//! pub struct BenchSpec {
//!     pub name: String,
//!     pub iterations: u32,
//!     pub warmup: u32,
//! }
//!
//! // Implement conversions using the traits
//! impl FromFfi<BenchSpecFfi> for BenchSpec {
//!     fn from_ffi(ffi: BenchSpecFfi) -> Self {
//!         Self {
//!             name: ffi.name,
//!             iterations: ffi.iterations,
//!             warmup: ffi.warmup,
//!         }
//!     }
//! }
//! ```

use serde::{Deserialize, Serialize};

// Re-export from uniffi_types for backwards compatibility
pub use crate::uniffi_types::{
    BenchErrorVariant, BenchReportTemplate, BenchSampleTemplate, BenchSpecTemplate, FromSdkError,
    FromSdkReport, FromSdkSample, FromSdkSpec,
};

/// FFI-ready benchmark specification.
///
/// Use this as a template for your UniFFI Record type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchSpecFfi {
    /// Name of the benchmark function to run.
    pub name: String,
    /// Number of measurement iterations.
    pub iterations: u32,
    /// Number of warmup iterations before measurement.
    pub warmup: u32,
}

impl From<crate::BenchSpec> for BenchSpecFfi {
    fn from(spec: crate::BenchSpec) -> Self {
        Self {
            name: spec.name,
            iterations: spec.iterations,
            warmup: spec.warmup,
        }
    }
}

impl From<BenchSpecFfi> for crate::BenchSpec {
    fn from(spec: BenchSpecFfi) -> Self {
        Self {
            name: spec.name,
            iterations: spec.iterations,
            warmup: spec.warmup,
        }
    }
}

/// FFI-ready benchmark sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchSampleFfi {
    /// Duration of the iteration in nanoseconds.
    pub duration_ns: u64,
    /// CPU time consumed by the measured iteration in milliseconds.
    pub cpu_time_ms: Option<u64>,
    /// Peak memory growth during the measured iteration in kilobytes.
    ///
    /// This is the legacy wire field for baseline-adjusted growth, not
    /// absolute process or device peak memory.
    pub peak_memory_kb: Option<u64>,
    /// Peak resident memory of the benchmark process during the measured iteration.
    pub process_peak_memory_kb: Option<u64>,
}

impl From<crate::BenchSample> for BenchSampleFfi {
    fn from(sample: crate::BenchSample) -> Self {
        Self {
            duration_ns: sample.duration_ns,
            cpu_time_ms: sample.cpu_time_ms,
            peak_memory_kb: sample.peak_memory_kb,
            process_peak_memory_kb: sample.process_peak_memory_kb,
        }
    }
}

impl From<BenchSampleFfi> for crate::BenchSample {
    fn from(sample: BenchSampleFfi) -> Self {
        Self {
            duration_ns: sample.duration_ns,
            cpu_time_ms: sample.cpu_time_ms,
            peak_memory_kb: sample.peak_memory_kb,
            process_peak_memory_kb: sample.process_peak_memory_kb,
        }
    }
}

/// FFI-ready benchmark report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReportFfi {
    /// The specification used for this benchmark run.
    pub spec: BenchSpecFfi,
    /// All collected timing samples.
    pub samples: Vec<BenchSampleFfi>,
    /// Optional semantic phase timings captured during measured iterations.
    pub phases: Vec<SemanticPhaseFfi>,
    /// Exact harness timeline spans in execution order.
    pub timeline: Vec<HarnessTimelineSpanFfi>,
}

/// FFI-ready semantic phase timing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticPhaseFfi {
    pub name: String,
    pub duration_ns: u64,
}

impl From<crate::SemanticPhase> for SemanticPhaseFfi {
    fn from(phase: crate::SemanticPhase) -> Self {
        Self {
            name: phase.name,
            duration_ns: phase.duration_ns,
        }
    }
}

/// FFI-ready exact harness timeline span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessTimelineSpanFfi {
    pub phase: String,
    pub start_offset_ns: u64,
    pub end_offset_ns: u64,
    pub iteration: Option<u32>,
}

impl From<crate::HarnessTimelineSpan> for HarnessTimelineSpanFfi {
    fn from(span: crate::HarnessTimelineSpan) -> Self {
        Self {
            phase: span.phase,
            start_offset_ns: span.start_offset_ns,
            end_offset_ns: span.end_offset_ns,
            iteration: span.iteration,
        }
    }
}

impl From<crate::RunnerReport> for BenchReportFfi {
    fn from(report: crate::RunnerReport) -> Self {
        Self {
            spec: report.spec.into(),
            samples: report.samples.into_iter().map(Into::into).collect(),
            phases: report.phases.into_iter().map(Into::into).collect(),
            timeline: report.timeline.into_iter().map(Into::into).collect(),
        }
    }
}

/// FFI-ready error type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BenchErrorFfi {
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

impl From<crate::types::BenchError> for BenchErrorFfi {
    fn from(err: crate::types::BenchError) -> Self {
        match err {
            crate::types::BenchError::Runner(runner_err) => match runner_err {
                crate::timing::TimingError::NoIterations { .. } => BenchErrorFfi::InvalidIterations,
                crate::timing::TimingError::Execution(msg) => {
                    BenchErrorFfi::ExecutionFailed { reason: msg }
                }
            },
            crate::types::BenchError::UnknownFunction(name, _) => {
                BenchErrorFfi::UnknownFunction { name }
            }
            crate::types::BenchError::Execution(msg) => {
                BenchErrorFfi::ExecutionFailed { reason: msg }
            }
            crate::types::BenchError::Io(e) => BenchErrorFfi::IoError {
                message: e.to_string(),
            },
            crate::types::BenchError::Serialization(e) => BenchErrorFfi::ConfigError {
                message: e.to_string(),
            },
            crate::types::BenchError::Config(msg) => BenchErrorFfi::ConfigError { message: msg },
            crate::types::BenchError::Build(msg) => BenchErrorFfi::ExecutionFailed {
                reason: format!("build error: {}", msg),
            },
        }
    }
}

/// Trait for converting SDK types to FFI types.
pub trait IntoFfi<T> {
    /// Convert self into the FFI representation.
    fn into_ffi(self) -> T;
}

/// Trait for converting FFI types to SDK types.
pub trait FromFfi<T> {
    /// Convert from FFI representation to SDK type.
    fn from_ffi(ffi: T) -> Self;
}

// Blanket implementations
impl<T, U> IntoFfi<U> for T
where
    U: From<T>,
{
    fn into_ffi(self) -> U {
        U::from(self)
    }
}

impl<T, U> FromFfi<U> for T
where
    T: From<U>,
{
    fn from_ffi(ffi: U) -> Self {
        T::from(ffi)
    }
}

/// Run a benchmark and return FFI-ready result.
///
/// This is a convenience function that wraps `run_benchmark` with FFI type conversions.
#[cfg(feature = "full")]
pub fn run_benchmark_ffi(spec: BenchSpecFfi) -> Result<BenchReportFfi, BenchErrorFfi> {
    let sdk_spec: crate::BenchSpec = spec.into();
    crate::run_benchmark(sdk_spec)
        .map(Into::into)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bench_spec_ffi_conversion() {
        let sdk_spec = crate::BenchSpec {
            name: "test".to_string(),
            iterations: 100,
            warmup: 10,
        };

        let ffi: BenchSpecFfi = sdk_spec.clone().into();
        assert_eq!(ffi.name, "test");
        assert_eq!(ffi.iterations, 100);
        assert_eq!(ffi.warmup, 10);

        let back: crate::BenchSpec = ffi.into();
        assert_eq!(back.name, sdk_spec.name);
    }

    #[test]
    fn test_bench_sample_ffi_conversion() {
        let sdk_sample = crate::BenchSample {
            duration_ns: 12345,
            cpu_time_ms: Some(12),
            peak_memory_kb: Some(48),
            process_peak_memory_kb: Some(1024),
        };
        let ffi: BenchSampleFfi = sdk_sample.into();
        assert_eq!(ffi.duration_ns, 12345);
        assert_eq!(ffi.cpu_time_ms, Some(12));
        assert_eq!(ffi.peak_memory_kb, Some(48));
        assert_eq!(ffi.process_peak_memory_kb, Some(1024));
    }

    #[test]
    fn test_bench_report_ffi_conversion() {
        let report = crate::RunnerReport {
            spec: crate::BenchSpec {
                name: "test".to_string(),
                iterations: 2,
                warmup: 1,
            },
            samples: vec![
                crate::BenchSample {
                    duration_ns: 100,
                    cpu_time_ms: Some(3),
                    peak_memory_kb: Some(8),
                    process_peak_memory_kb: Some(108),
                },
                crate::BenchSample {
                    duration_ns: 200,
                    cpu_time_ms: Some(5),
                    peak_memory_kb: Some(13),
                    process_peak_memory_kb: Some(113),
                },
            ],
            phases: vec![crate::SemanticPhase {
                name: "prove".to_string(),
                duration_ns: 300,
            }],
            timeline: vec![crate::HarnessTimelineSpan {
                phase: "measured-benchmark".to_string(),
                start_offset_ns: 0,
                end_offset_ns: 100,
                iteration: Some(0),
            }],
        };

        let ffi: BenchReportFfi = report.into();
        assert_eq!(ffi.spec.name, "test");
        assert_eq!(ffi.samples.len(), 2);
        assert_eq!(ffi.samples[0].duration_ns, 100);
        assert_eq!(ffi.samples[0].cpu_time_ms, Some(3));
        assert_eq!(ffi.samples[0].peak_memory_kb, Some(8));
        assert_eq!(ffi.samples[0].process_peak_memory_kb, Some(108));
        assert_eq!(ffi.phases.len(), 1);
        assert_eq!(ffi.phases[0].name, "prove");
        assert_eq!(ffi.timeline.len(), 1);
        assert_eq!(ffi.timeline[0].phase, "measured-benchmark");
    }

    #[test]
    fn test_into_ffi_trait() {
        let spec = crate::BenchSpec {
            name: "test".to_string(),
            iterations: 50,
            warmup: 5,
        };

        let ffi: BenchSpecFfi = spec.into_ffi();
        assert_eq!(ffi.iterations, 50);
    }
}
