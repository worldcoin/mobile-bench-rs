//! Shared benchmarking harness that will be compiled into mobile targets.
//! For now this runs on the host and provides the same API surface we will
//! expose over FFI to Kotlin/Swift.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchSpec {
    pub name: String,
    pub iterations: u32,
    pub warmup: u32,
}

impl BenchSpec {
    pub fn new(name: impl Into<String>, iterations: u32, warmup: u32) -> Result<Self, BenchError> {
        if iterations == 0 {
            return Err(BenchError::NoIterations);
        }

        Ok(Self {
            name: name.into(),
            iterations,
            warmup,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchSample {
    pub duration_ns: u128,
}

impl BenchSample {
    fn from_duration(duration: Duration) -> Self {
        Self {
            duration_ns: duration.as_nanos(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchReport {
    pub spec: BenchSpec,
    pub samples: Vec<BenchSample>,
}

#[derive(Debug, Error)]
pub enum BenchError {
    #[error("iterations must be greater than zero")]
    NoIterations,
    #[error("benchmark function failed: {0}")]
    Execution(String),
}

pub fn run_closure<F>(spec: BenchSpec, mut f: F) -> Result<BenchReport, BenchError>
where
    F: FnMut() -> Result<(), BenchError>,
{
    if spec.iterations == 0 {
        return Err(BenchError::NoIterations);
    }

    for _ in 0..spec.warmup {
        f()?;
    }

    let mut samples = Vec::with_capacity(spec.iterations as usize);
    for _ in 0..spec.iterations {
        let start = Instant::now();
        f()?;
        samples.push(BenchSample::from_duration(start.elapsed()));
    }

    Ok(BenchReport { spec, samples })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_benchmark() {
        let spec = BenchSpec::new("noop", 3, 1).unwrap();
        let report = run_closure(spec, || Ok(())).unwrap();

        assert_eq!(report.samples.len(), 3);
        let non_zero = report.samples.iter().filter(|s| s.duration_ns > 0).count();
        assert!(non_zero >= 1);
    }
}
