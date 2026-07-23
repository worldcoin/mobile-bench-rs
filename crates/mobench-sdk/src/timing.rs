//! Lightweight benchmarking harness for mobile platforms.
//!
//! This module provides the core timing infrastructure for the mobench ecosystem.
//! It was previously a separate crate (`mobench-runner`) but has been consolidated
//! into `mobench-sdk` for a simpler dependency graph.
//!
//! The module is designed to be minimal and portable, with no platform-specific
//! dependencies, making it suitable for compilation to Android and iOS targets.
//!
//! ## Overview
//!
//! The timing module executes benchmark functions with:
//! - Configurable warmup iterations
//! - Precise nanosecond-resolution timing
//! - Simple, serializable results
//!
//! ## Usage
//!
//! Most users should use this via the higher-level [`crate::run_benchmark`] function
//! or [`crate::BenchmarkBuilder`]. Direct usage is for custom integrations:
//!
//! ```
//! use mobench_sdk::timing::{BenchSpec, run_closure, TimingError};
//!
//! // Define a benchmark specification
//! let spec = BenchSpec::new("my_benchmark", 100, 10)?;
//!
//! // Run the benchmark
//! let report = run_closure(spec, || {
//!     // Your benchmark code
//!     let sum: u64 = (0..1000).sum();
//!     std::hint::black_box(sum);
//!     Ok(())
//! })?;
//!
//! // Analyze results
//! let mean_ns = report.samples.iter()
//!     .map(|s| s.duration_ns)
//!     .sum::<u64>() / report.samples.len() as u64;
//!
//! println!("Mean: {} ns", mean_ns);
//! # Ok::<(), TimingError>(())
//! ```
//!
//! ## Types
//!
//! | Type | Description |
//! |------|-------------|
//! | [`BenchSpec`] | Benchmark configuration (name, iterations, warmup) |
//! | [`BenchSample`] | Single timing measurement in nanoseconds |
//! | [`BenchReport`] | Complete results with all samples |
//! | [`TimingError`] | Error conditions during benchmarking |
//!
//! ## Feature Flags
//!
//! This module is always available. When using `mobench-sdk` with default features,
//! you also get build automation and template generation. For minimal binary size
//! (e.g., on mobile targets), use the `runner-only` feature:
//!
//! ```toml
//! [dependencies]
//! mobench-sdk = { version = "0.1.37", default-features = false, features = ["runner-only"] }
//! ```

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, mpsc};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::{self, JoinHandle};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
use thiserror::Error;

#[cfg(target_arch = "wasm32")]
mod browser_time {
    use std::time::Duration;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = performance, js_name = now)]
        fn performance_now_ms() -> f64;
    }

    /// Browser-backed monotonic instant.
    ///
    /// `std::time::Instant::now()` panics on `wasm32-unknown-unknown`, so the
    /// timing harness uses the browser's high-resolution monotonic clock.
    #[derive(Clone, Copy)]
    pub(super) struct Instant(f64);

    impl Instant {
        pub(super) fn now() -> Self {
            Self(performance_now_ms())
        }

        pub(super) fn duration_since(self, earlier: Self) -> Duration {
            Duration::from_secs_f64(((self.0 - earlier.0).max(0.0)) / 1_000.0)
        }

        pub(super) fn elapsed(self) -> Duration {
            Self::now().duration_since(self)
        }
    }
}

#[cfg(target_arch = "wasm32")]
use browser_time::Instant;

/// Benchmark specification defining what and how to benchmark.
///
/// Contains the benchmark name, number of measurement iterations, and
/// warmup iterations to perform before measuring.
///
/// # Example
///
/// ```
/// use mobench_sdk::timing::BenchSpec;
///
/// // Create a spec for 100 iterations with 10 warmup runs
/// let spec = BenchSpec::new("sorting_benchmark", 100, 10)?;
///
/// assert_eq!(spec.name, "sorting_benchmark");
/// assert_eq!(spec.iterations, 100);
/// assert_eq!(spec.warmup, 10);
/// # Ok::<(), mobench_sdk::timing::TimingError>(())
/// ```
///
/// # Serialization
///
/// `BenchSpec` implements `Serialize` and `Deserialize` for JSON persistence:
///
/// ```
/// use mobench_sdk::timing::BenchSpec;
///
/// let spec = BenchSpec {
///     name: "my_bench".to_string(),
///     iterations: 50,
///     warmup: 5,
/// };
///
/// let json = serde_json::to_string(&spec)?;
/// let restored: BenchSpec = serde_json::from_str(&json)?;
///
/// assert_eq!(spec.name, restored.name);
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchSpec {
    /// Name of the benchmark, typically the fully-qualified function name.
    ///
    /// Examples: `"my_crate::fibonacci"`, `"sorting_benchmark"`
    pub name: String,

    /// Number of iterations to measure.
    ///
    /// Each iteration produces one [`BenchSample`]. Must be greater than zero.
    pub iterations: u32,

    /// Number of warmup iterations before measurement.
    ///
    /// Warmup iterations are not recorded. They allow CPU caches to warm
    /// and any JIT compilation to complete. Can be zero.
    pub warmup: u32,
}

impl BenchSpec {
    /// Creates a new benchmark specification.
    ///
    /// # Arguments
    ///
    /// * `name` - Name identifier for the benchmark
    /// * `iterations` - Number of measured iterations (must be > 0)
    /// * `warmup` - Number of warmup iterations (can be 0)
    ///
    /// # Errors
    ///
    /// Returns [`TimingError::NoIterations`] if `iterations` is zero.
    ///
    /// # Example
    ///
    /// ```
    /// use mobench_sdk::timing::BenchSpec;
    ///
    /// let spec = BenchSpec::new("test", 100, 10)?;
    /// assert_eq!(spec.iterations, 100);
    ///
    /// // Zero iterations is an error
    /// let err = BenchSpec::new("test", 0, 10);
    /// assert!(err.is_err());
    /// # Ok::<(), mobench_sdk::timing::TimingError>(())
    /// ```
    pub fn new(name: impl Into<String>, iterations: u32, warmup: u32) -> Result<Self, TimingError> {
        if iterations == 0 {
            return Err(TimingError::NoIterations { count: iterations });
        }

        Ok(Self {
            name: name.into(),
            iterations,
            warmup,
        })
    }
}

/// A single timing sample from a benchmark iteration.
///
/// Holds the elapsed wall time in nanoseconds plus optional per-iteration
/// resource metrics (CPU time, peak memory growth, and process peak memory).
/// The optional fields are only populated on platforms where the harness can
/// capture them and are skipped from the JSON output when absent.
///
/// # Example
///
/// ```
/// use mobench_sdk::timing::BenchSample;
///
/// let sample = BenchSample {
///     duration_ns: 1_500_000,
///     ..Default::default()
/// };
///
/// // Convert to milliseconds
/// let ms = sample.duration_ns as f64 / 1_000_000.0;
/// assert_eq!(ms, 1.5);
/// ```
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BenchSample {
    /// Duration of the iteration in nanoseconds.
    ///
    /// Measured using [`std::time::Instant`] for monotonic, high-resolution timing.
    pub duration_ns: u64,

    /// CPU time consumed by the measured iteration in milliseconds.
    ///
    /// This is captured around the measured benchmark closure only and excludes
    /// warmup, setup, teardown, and report generation overhead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_time_ms: Option<u64>,

    /// Peak memory growth during the measured iteration in kilobytes.
    ///
    /// This legacy wire field is baseline-adjusted immediately before the
    /// measured closure enters. It reports growth during the measured
    /// iteration, not absolute process or device peak memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_memory_kb: Option<u64>,

    /// Peak resident memory of the benchmark process during the measured iteration.
    ///
    /// This is sampled from the current process while the measured closure is
    /// running. Unlike `peak_memory_kb`, it is not baseline-adjusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_peak_memory_kb: Option<u64>,
}

impl BenchSample {
    fn from_measurement(duration: Duration, resources: IterationResourceUsage) -> Self {
        Self {
            duration_ns: duration.as_nanos() as u64,
            cpu_time_ms: resources.cpu_time_ms,
            peak_memory_kb: resources.peak_memory_kb,
            process_peak_memory_kb: resources.process_peak_memory_kb,
        }
    }
}

/// Complete benchmark report with all timing samples.
///
/// Contains the original specification and all collected samples.
/// Can be serialized to JSON for storage or transmission.
///
/// # Example
///
/// ```
/// use mobench_sdk::timing::{BenchSpec, run_closure};
///
/// let spec = BenchSpec::new("example", 50, 5)?;
/// let report = run_closure(spec, || {
///     std::hint::black_box(42);
///     Ok(())
/// })?;
///
/// // Calculate statistics
/// let samples: Vec<u64> = report.samples.iter()
///     .map(|s| s.duration_ns)
///     .collect();
///
/// let min = samples.iter().min().unwrap();
/// let max = samples.iter().max().unwrap();
/// let mean = samples.iter().sum::<u64>() / samples.len() as u64;
///
/// println!("Min: {} ns, Max: {} ns, Mean: {} ns", min, max, mean);
/// # Ok::<(), mobench_sdk::timing::TimingError>(())
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchReport {
    /// The specification used for this benchmark run.
    pub spec: BenchSpec,

    /// All collected timing samples.
    ///
    /// The length equals `spec.iterations`. Samples are in execution order.
    pub samples: Vec<BenchSample>,

    /// Optional semantic phase timings captured during measured iterations.
    ///
    /// Defaults to an empty vector when deserializing reports produced by
    /// older mobench versions that did not emit phase data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<SemanticPhase>,

    /// Exact harness timeline spans in execution order.
    ///
    /// Defaults to an empty vector when deserializing reports produced by
    /// older mobench versions that did not emit timeline data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline: Vec<HarnessTimelineSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessTimelineSpan {
    pub phase: String,
    pub start_offset_ns: u64,
    pub end_offset_ns: u64,
    pub iteration: Option<u32>,
}

impl BenchReport {
    /// Returns the mean (average) duration in nanoseconds.
    #[must_use]
    pub fn mean_ns(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.samples.iter().map(|s| s.duration_ns).sum();
        sum as f64 / self.samples.len() as f64
    }

    /// Returns the median duration in nanoseconds.
    #[must_use]
    pub fn median_ns(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut sorted: Vec<u64> = self.samples.iter().map(|s| s.duration_ns).collect();
        sorted.sort_unstable();
        let len = sorted.len();
        if len % 2 == 0 {
            (sorted[len / 2 - 1] + sorted[len / 2]) as f64 / 2.0
        } else {
            sorted[len / 2] as f64
        }
    }

    /// Returns the standard deviation in nanoseconds (sample std dev, n-1).
    #[must_use]
    pub fn std_dev_ns(&self) -> f64 {
        if self.samples.len() < 2 {
            return 0.0;
        }
        let mean = self.mean_ns();
        let variance: f64 = self
            .samples
            .iter()
            .map(|s| {
                let diff = s.duration_ns as f64 - mean;
                diff * diff
            })
            .sum::<f64>()
            / (self.samples.len() - 1) as f64;
        variance.sqrt()
    }

    /// Returns the given percentile (0-100) in nanoseconds.
    #[must_use]
    pub fn percentile_ns(&self, p: f64) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut sorted: Vec<u64> = self.samples.iter().map(|s| s.duration_ns).collect();
        sorted.sort_unstable();
        let p = p.clamp(0.0, 100.0) / 100.0;
        let index = (p * (sorted.len() - 1) as f64).round() as usize;
        sorted[index.min(sorted.len() - 1)] as f64
    }

    /// Returns the minimum duration in nanoseconds.
    #[must_use]
    pub fn min_ns(&self) -> u64 {
        self.samples
            .iter()
            .map(|s| s.duration_ns)
            .min()
            .unwrap_or(0)
    }

    /// Returns the maximum duration in nanoseconds.
    #[must_use]
    pub fn max_ns(&self) -> u64 {
        self.samples
            .iter()
            .map(|s| s.duration_ns)
            .max()
            .unwrap_or(0)
    }

    /// Returns the total measured CPU time in milliseconds across all iterations.
    #[must_use]
    pub fn cpu_total_ms(&self) -> Option<u64> {
        let values = self
            .samples
            .iter()
            .filter_map(|sample| sample.cpu_time_ms)
            .collect::<Vec<_>>();
        if values.is_empty() {
            return None;
        }

        let total = values
            .iter()
            .fold(0_u128, |sum, value| sum.saturating_add(u128::from(*value)));
        Some(total.min(u128::from(u64::MAX)) as u64)
    }

    /// Returns the median measured CPU time in milliseconds across all iterations.
    #[must_use]
    pub fn cpu_median_ms(&self) -> Option<u64> {
        let mut values = self
            .samples
            .iter()
            .filter_map(|sample| sample.cpu_time_ms)
            .collect::<Vec<_>>();
        if values.is_empty() {
            return None;
        }

        values.sort_unstable();
        let len = values.len();
        Some(if len % 2 == 0 {
            let lower = u128::from(values[(len / 2) - 1]);
            let upper = u128::from(values[len / 2]);
            ((lower + upper) / 2) as u64
        } else {
            values[len / 2]
        })
    }

    /// Returns the maximum baseline-adjusted peak memory growth in kilobytes.
    ///
    /// This is the legacy accessor for the serialized `peak_memory_kb` sample
    /// field. It does not report absolute process or device peak memory.
    #[must_use]
    pub fn peak_memory_kb(&self) -> Option<u64> {
        self.samples
            .iter()
            .filter_map(|sample| sample.peak_memory_kb)
            .max()
    }

    /// Returns the maximum baseline-adjusted peak memory growth in kilobytes.
    ///
    /// This is an explicit alias for [`BenchReport::peak_memory_kb`] to make the
    /// growth semantics clear while preserving the legacy wire field.
    #[must_use]
    #[doc(alias = "peak_memory_kb")]
    pub fn peak_memory_growth_kb(&self) -> Option<u64> {
        self.peak_memory_kb()
    }

    /// Returns the maximum process resident memory peak in kilobytes.
    ///
    /// This reports the current benchmark process peak sampled during measured
    /// iterations. It excludes BrowserStack/session-level provider memory.
    #[must_use]
    pub fn process_peak_memory_kb(&self) -> Option<u64> {
        self.samples
            .iter()
            .filter_map(|sample| sample.process_peak_memory_kb)
            .max()
    }

    /// Returns a statistical summary of the benchmark results.
    #[must_use]
    pub fn summary(&self) -> BenchSummary {
        BenchSummary {
            name: self.spec.name.clone(),
            iterations: self.samples.len() as u32,
            warmup: self.spec.warmup,
            mean_ns: self.mean_ns(),
            median_ns: self.median_ns(),
            std_dev_ns: self.std_dev_ns(),
            min_ns: self.min_ns(),
            max_ns: self.max_ns(),
            p95_ns: self.percentile_ns(95.0),
            p99_ns: self.percentile_ns(99.0),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct IterationResourceUsage {
    cpu_time_ms: Option<u64>,
    peak_memory_kb: Option<u64>,
    process_peak_memory_kb: Option<u64>,
}

fn instant_offset_ns(origin: Instant, instant: Instant) -> u64 {
    instant
        .duration_since(origin)
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
}

fn push_timeline_span(
    timeline: &mut Vec<HarnessTimelineSpan>,
    origin: Instant,
    phase: &str,
    started_at: Instant,
    ended_at: Instant,
    iteration: Option<u32>,
) {
    timeline.push(HarnessTimelineSpan {
        phase: phase.to_string(),
        start_offset_ns: instant_offset_ns(origin, started_at),
        end_offset_ns: instant_offset_ns(origin, ended_at),
        iteration,
    });
}

/// Statistical summary of benchmark results.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchSummary {
    /// Name of the benchmark.
    pub name: String,
    /// Number of measured iterations.
    pub iterations: u32,
    /// Number of warmup iterations.
    pub warmup: u32,
    /// Mean duration in nanoseconds.
    pub mean_ns: f64,
    /// Median duration in nanoseconds.
    pub median_ns: f64,
    /// Standard deviation in nanoseconds.
    pub std_dev_ns: f64,
    /// Minimum duration in nanoseconds.
    pub min_ns: u64,
    /// Maximum duration in nanoseconds.
    pub max_ns: u64,
    /// 95th percentile in nanoseconds.
    pub p95_ns: f64,
    /// 99th percentile in nanoseconds.
    pub p99_ns: f64,
}

/// Flat semantic phase timing captured during a benchmark run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticPhase {
    pub name: String,
    pub duration_ns: u64,
}

#[derive(Default)]
struct SemanticPhaseCollector {
    enabled: bool,
    depth: usize,
    phases: Vec<SemanticPhase>,
}

impl SemanticPhaseCollector {
    fn reset(&mut self) {
        self.enabled = false;
        self.depth = 0;
        self.phases.clear();
    }

    fn begin_measurement(&mut self) {
        self.reset();
        self.enabled = true;
    }

    fn finish(&mut self) -> Vec<SemanticPhase> {
        self.enabled = false;
        self.depth = 0;
        std::mem::take(&mut self.phases)
    }

    fn enter_phase(&mut self) -> Option<bool> {
        if !self.enabled {
            return None;
        }
        let top_level = self.depth == 0;
        self.depth += 1;
        Some(top_level)
    }

    fn exit_phase(&mut self, name: &str, top_level: bool, elapsed: Duration) {
        self.depth = self.depth.saturating_sub(1);
        if !self.enabled || !top_level {
            return;
        }

        let duration_ns = elapsed.as_nanos().min(u128::from(u64::MAX)) as u64;
        if let Some(phase) = self.phases.iter_mut().find(|phase| phase.name == name) {
            phase.duration_ns = phase.duration_ns.saturating_add(duration_ns);
        } else {
            self.phases.push(SemanticPhase {
                name: name.to_string(),
                duration_ns,
            });
        }
    }
}

thread_local! {
    static SEMANTIC_PHASE_COLLECTOR: RefCell<SemanticPhaseCollector> =
        RefCell::new(SemanticPhaseCollector::default());
}

struct SemanticPhaseGuard {
    name: String,
    started_at: Option<Instant>,
    top_level: bool,
}

impl Drop for SemanticPhaseGuard {
    fn drop(&mut self) {
        let Some(started_at) = self.started_at else {
            return;
        };

        let elapsed = started_at.elapsed();
        SEMANTIC_PHASE_COLLECTOR.with(|collector| {
            collector
                .borrow_mut()
                .exit_phase(&self.name, self.top_level, elapsed);
        });
    }
}

fn reset_semantic_phase_collection() {
    SEMANTIC_PHASE_COLLECTOR.with(|collector| collector.borrow_mut().reset());
}

fn begin_semantic_phase_collection() {
    SEMANTIC_PHASE_COLLECTOR.with(|collector| collector.borrow_mut().begin_measurement());
}

fn finish_semantic_phase_collection() -> Vec<SemanticPhase> {
    SEMANTIC_PHASE_COLLECTOR.with(|collector| collector.borrow_mut().finish())
}

trait ResourceMonitor {
    type Token;

    fn start(&mut self) -> Self::Token;

    fn finish(&mut self, token: Self::Token) -> IterationResourceUsage;
}

#[derive(Default)]
struct DefaultResourceMonitor {
    /// Lazily-initialized long-lived sampler shared across measured iterations.
    ///
    /// We pay thread-spawn cost once per benchmark function rather than per
    /// iteration. On constrained mobile devices (Android/Bionic) thread
    /// creation is significantly more expensive than on desktop Linux, and
    /// 1000+ iteration benchmarks would otherwise spawn 1000+ throwaway
    /// threads.
    #[cfg(not(target_arch = "wasm32"))]
    memory_sampler: Option<PersistentMemorySampler>,
    /// Set after the first attempt to start the sampler so we do not retry
    /// on platforms where the sampler is not supported.
    #[cfg(not(target_arch = "wasm32"))]
    sampler_init_attempted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessCpuTimeSnapshot {
    user_ns: u64,
    system_ns: u64,
}

impl ProcessCpuTimeSnapshot {
    #[cfg(unix)]
    fn from_rusage_timevals(user: libc::timeval, system: libc::timeval) -> Option<Self> {
        Some(Self {
            user_ns: timeval_to_ns(user)?,
            system_ns: timeval_to_ns(system)?,
        })
    }

    #[cfg(unix)]
    fn total_ns(self) -> u64 {
        self.user_ns.saturating_add(self.system_ns)
    }
}

struct DefaultResourceToken {
    cpu_time_start: Option<ProcessCpuTimeSnapshot>,
    /// True if the persistent sampler accepted a `Begin` for this iteration
    /// and we therefore expect a corresponding result on `finish`.
    #[cfg(not(target_arch = "wasm32"))]
    has_memory_window: bool,
}

impl ResourceMonitor for DefaultResourceMonitor {
    type Token = DefaultResourceToken;

    fn start(&mut self) -> Self::Token {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !self.sampler_init_attempted {
                self.memory_sampler = PersistentMemorySampler::start();
                self.sampler_init_attempted = true;
            }
            let has_memory_window = self
                .memory_sampler
                .as_ref()
                .is_some_and(PersistentMemorySampler::begin_window);
            Self::Token {
                cpu_time_start: current_process_cpu_time(),
                has_memory_window,
            }
        }

        #[cfg(target_arch = "wasm32")]
        Self::Token {
            cpu_time_start: None,
        }
    }

    fn finish(&mut self, token: Self::Token) -> IterationResourceUsage {
        let cpu_time_ms = token
            .cpu_time_start
            .zip(current_process_cpu_time())
            .and_then(|(start, end)| process_cpu_delta_ms(start, end));

        #[cfg(not(target_arch = "wasm32"))]
        let memory_peak = if token.has_memory_window {
            self.memory_sampler
                .as_ref()
                .and_then(PersistentMemorySampler::end_window)
        } else {
            None
        };

        #[cfg(not(target_arch = "wasm32"))]
        {
            IterationResourceUsage {
                cpu_time_ms,
                peak_memory_kb: memory_peak
                    .and_then(|peak| (peak.growth_kb > 0).then_some(peak.growth_kb)),
                process_peak_memory_kb: memory_peak
                    .and_then(|peak| (peak.process_peak_kb > 0).then_some(peak.process_peak_kb)),
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            IterationResourceUsage {
                cpu_time_ms,
                ..IterationResourceUsage::default()
            }
        }
    }
}

#[cfg(unix)]
fn round_ns_to_ms(ns: u64) -> u64 {
    ((u128::from(ns) + 500_000) / 1_000_000) as u64
}

#[cfg(unix)]
fn process_cpu_delta_ms(start: ProcessCpuTimeSnapshot, end: ProcessCpuTimeSnapshot) -> Option<u64> {
    Some(round_ns_to_ms(
        end.total_ns().checked_sub(start.total_ns())?,
    ))
}

#[cfg(not(unix))]
fn process_cpu_delta_ms(
    _start: ProcessCpuTimeSnapshot,
    _end: ProcessCpuTimeSnapshot,
) -> Option<u64> {
    None
}

#[cfg(unix)]
fn timeval_to_ns(value: libc::timeval) -> Option<u64> {
    let secs = u64::try_from(value.tv_sec).ok()?;
    let micros = u64::try_from(value.tv_usec).ok()?;
    Some(
        secs.saturating_mul(1_000_000_000)
            .saturating_add(micros.saturating_mul(1_000)),
    )
}

#[cfg(unix)]
fn current_process_cpu_time() -> Option<ProcessCpuTimeSnapshot> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: `RUSAGE_SELF` is always a valid `who` value and the kernel
    // writes a fully-initialized `rusage` into the provided pointer on
    // success. We bail out via `rc != 0` before touching the buffer below.
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }

    // SAFETY: `getrusage` returned 0, so the buffer is fully initialized.
    let usage = unsafe { usage.assume_init() };
    ProcessCpuTimeSnapshot::from_rusage_timevals(usage.ru_utime, usage.ru_stime)
}

#[cfg(not(unix))]
fn current_process_cpu_time() -> Option<ProcessCpuTimeSnapshot> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
const MEMORY_SAMPLER_INTERVAL: Duration = Duration::from_millis(1);
#[cfg(not(target_arch = "wasm32"))]
type MemoryReader = Arc<dyn Fn() -> Option<u64> + Send + Sync + 'static>;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessMemoryPeak {
    growth_kb: u64,
    process_peak_kb: u64,
}

/// Long-lived memory sampler. Spawned once per benchmark function and reused
/// across every measured iteration via `begin_window` / `end_window`.
///
/// Replaces the previous per-iteration design that spawned and joined a fresh
/// thread for every sample. On Android (Bionic) and iOS that thread-creation
/// overhead is non-trivial and would inflate harness wall time on
/// high-iteration runs.
#[cfg(not(target_arch = "wasm32"))]
struct PersistentMemorySampler {
    cmd_tx: mpsc::SyncSender<SamplerCmd>,
    result_rx: mpsc::Receiver<Option<ProcessMemoryPeak>>,
    handle: Option<JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
enum SamplerCmd {
    Begin(mpsc::SyncSender<bool>),
    End,
    Shutdown,
}

#[cfg(not(target_arch = "wasm32"))]
impl PersistentMemorySampler {
    fn start() -> Option<Self> {
        Self::start_with_reader(Arc::new(current_process_memory_kb))
    }

    fn start_with_reader(reader: MemoryReader) -> Option<Self> {
        let (cmd_tx, cmd_rx) = mpsc::sync_channel::<SamplerCmd>(1);
        let (result_tx, result_rx) = mpsc::sync_channel::<Option<ProcessMemoryPeak>>(1);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<()>(1);

        let handle = thread::Builder::new()
            .name("mobench-memory-sampler".to_string())
            .spawn(move || {
                // Touch the sampler thread's own stack and runtime state once
                // before any window opens so its initialization cost cannot
                // contaminate the first iteration's baseline measurement.
                let _ = reader();
                if ready_tx.send(()).is_err() {
                    return;
                }
                drop(ready_tx);

                Self::run(reader, &cmd_rx, &result_tx);
            })
            .ok()?;

        if ready_rx.recv().is_err() {
            // Thread failed before sending readiness. Send Shutdown to make
            // sure it does not get stuck on a later cmd recv, then join.
            let _ = cmd_tx.send(SamplerCmd::Shutdown);
            let _ = handle.join();
            return None;
        }

        Some(Self {
            cmd_tx,
            result_rx,
            handle: Some(handle),
        })
    }

    fn run(
        reader: MemoryReader,
        cmd_rx: &mpsc::Receiver<SamplerCmd>,
        result_tx: &mpsc::SyncSender<Option<ProcessMemoryPeak>>,
    ) {
        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                SamplerCmd::Begin(ack_tx) => {
                    let baseline = match reader() {
                        Some(v) => v,
                        None => {
                            let _ = ack_tx.send(false);
                            continue;
                        }
                    };
                    if ack_tx.send(true).is_err() {
                        continue;
                    }
                    let mut peak = baseline;
                    let shutting_down = loop {
                        match cmd_rx.recv_timeout(MEMORY_SAMPLER_INTERVAL) {
                            Ok(SamplerCmd::End) => break false,
                            Ok(SamplerCmd::Shutdown) => break true,
                            // A stray Begin while a window is already open
                            // means the producer side desynced — preserve
                            // existing behavior by ignoring it.
                            Ok(SamplerCmd::Begin(ack_tx)) => {
                                let _ = ack_tx.send(false);
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                if let Some(current) = reader()
                                    && current > peak
                                {
                                    peak = current;
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => break true,
                        }
                    };
                    // One last sample after the window closes so a final
                    // allocation that happens between the last poll and the
                    // End command is still accounted for.
                    if let Some(current) = reader()
                        && current > peak
                    {
                        peak = current;
                    }
                    let _ = result_tx.send(Some(ProcessMemoryPeak {
                        growth_kb: peak.saturating_sub(baseline),
                        process_peak_kb: peak,
                    }));
                    if shutting_down {
                        return;
                    }
                }
                SamplerCmd::Shutdown => return,
                // End without an active Begin — ignore.
                SamplerCmd::End => {}
            }
        }
    }

    fn begin_window(&self) -> bool {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.cmd_tx
            .send(SamplerCmd::Begin(ack_tx))
            .ok()
            .and_then(|()| ack_rx.recv().ok())
            .unwrap_or(false)
    }

    fn end_window(&self) -> Option<ProcessMemoryPeak> {
        self.cmd_tx.send(SamplerCmd::End).ok()?;
        self.result_rx.recv().ok().flatten()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for PersistentMemorySampler {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(SamplerCmd::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(any(target_os = "android", target_os = "linux"))]
fn current_process_memory_kb() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages = statm
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())?;
    // SAFETY: `_SC_PAGESIZE` is a valid sysconf selector on every supported
    // POSIX target; sysconf has no side effects and reports failures via -1.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return None;
    }
    let page_size = u64::try_from(page_size).ok()?;
    Some(resident_pages.saturating_mul(page_size) / 1024)
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
fn current_process_memory_kb() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info_data_t>::uninit();
    let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
    // `mach_task_self` is marked deprecated by libc in favor of
    // `mach_task_self_`, but the replacement symbol is not yet exposed by the
    // libc crate's iOS/macOS bindings (libc < 0.3). The deprecation is purely
    // cosmetic — the function continues to be the documented way to obtain
    // the current task port and is what Apple's headers expand the macro to.
    #[allow(deprecated)]
    // SAFETY: `mach_task_self` always returns a valid task port for the
    // current process. `MACH_TASK_BASIC_INFO` matches the
    // `mach_task_basic_info_data_t` layout we pass; `count` carries the
    // capacity in 32-bit words and is updated by the kernel on success.
    // We check for `KERN_SUCCESS` before assuming the buffer is initialized.
    let rc = unsafe {
        libc::task_info(
            libc::mach_task_self(),
            libc::MACH_TASK_BASIC_INFO,
            info.as_mut_ptr().cast::<libc::integer_t>(),
            &mut count,
        )
    };
    if rc != libc::KERN_SUCCESS {
        return None;
    }

    // SAFETY: `task_info` returned `KERN_SUCCESS`, so the basic info struct
    // is fully populated.
    let info = unsafe { info.assume_init() };
    Some(info.resident_size / 1024)
}

#[cfg(not(any(
    target_os = "android",
    target_os = "linux",
    target_os = "ios",
    target_os = "macos",
    target_arch = "wasm32"
)))]
fn current_process_memory_kb() -> Option<u64> {
    None
}

fn measure_iteration<M, F>(
    monitor: &mut M,
    f: F,
) -> Result<(BenchSample, Instant, Instant), TimingError>
where
    M: ResourceMonitor,
    F: FnOnce() -> Result<(), TimingError>,
{
    let token = monitor.start();
    let started_at = Instant::now();
    let result = f();
    let ended_at = Instant::now();
    let resources = monitor.finish(token);
    result.map(|_| {
        (
            BenchSample::from_measurement(ended_at.duration_since(started_at), resources),
            started_at,
            ended_at,
        )
    })
}

/// Records a flat semantic phase when called inside an active benchmark measurement loop.
///
/// Phases are aggregated across measured iterations and ignored during warmup/setup.
/// Nested phases are intentionally collapsed in v1 to keep the output flat.
pub fn profile_phase<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let guard = SEMANTIC_PHASE_COLLECTOR.with(|collector| {
        let mut collector = collector.borrow_mut();
        match collector.enter_phase() {
            Some(top_level) => SemanticPhaseGuard {
                name: name.to_string(),
                started_at: Some(Instant::now()),
                top_level,
            },
            None => SemanticPhaseGuard {
                name: String::new(),
                started_at: None,
                top_level: false,
            },
        }
    });

    let result = f();
    drop(guard);
    result
}

/// Errors that can occur during benchmark execution.
///
/// # Example
///
/// ```
/// use mobench_sdk::timing::{BenchSpec, TimingError};
///
/// // Zero iterations produces an error
/// let result = BenchSpec::new("test", 0, 10);
/// assert!(matches!(result, Err(TimingError::NoIterations { .. })));
/// ```
#[derive(Debug, Error)]
pub enum TimingError {
    /// The iteration count was zero or invalid.
    ///
    /// At least one iteration is required to produce a measurement.
    /// The error includes the actual value provided for diagnostic purposes.
    #[error("iterations must be greater than zero (got {count}). Minimum recommended: 10")]
    NoIterations {
        /// The invalid iteration count that was provided.
        count: u32,
    },

    /// The benchmark function failed during execution.
    ///
    /// Contains a description of the failure.
    #[error("benchmark function failed: {0}")]
    Execution(String),
}

/// Runs a benchmark by executing a closure repeatedly.
///
/// This is the core benchmarking function. It:
///
/// 1. Executes the closure `spec.warmup` times without recording
/// 2. Executes the closure `spec.iterations` times, recording each duration
/// 3. Returns a [`BenchReport`] with all samples
///
/// # Arguments
///
/// * `spec` - Benchmark configuration specifying iterations and warmup
/// * `f` - Closure to benchmark; must return `Result<(), TimingError>`
///
/// # Returns
///
/// A [`BenchReport`] containing all timing samples, or a [`TimingError`] if
/// the benchmark fails.
///
/// # Example
///
/// ```
/// use mobench_sdk::timing::{BenchSpec, run_closure, TimingError};
///
/// let spec = BenchSpec::new("sum_benchmark", 100, 10)?;
///
/// let report = run_closure(spec, || {
///     let sum: u64 = (0..1000).sum();
///     std::hint::black_box(sum);
///     Ok(())
/// })?;
///
/// assert_eq!(report.samples.len(), 100);
///
/// // Calculate mean duration
/// let total_ns: u64 = report.samples.iter().map(|s| s.duration_ns).sum();
/// let mean_ns = total_ns / report.samples.len() as u64;
/// println!("Mean: {} ns", mean_ns);
/// # Ok::<(), TimingError>(())
/// ```
///
/// # Error Handling
///
/// If the closure returns an error, the benchmark stops immediately:
///
/// ```
/// use mobench_sdk::timing::{BenchSpec, run_closure, TimingError};
///
/// let spec = BenchSpec::new("failing_bench", 100, 0)?;
///
/// let result = run_closure(spec, || {
///     Err(TimingError::Execution("simulated failure".into()))
/// });
///
/// assert!(result.is_err());
/// # Ok::<(), TimingError>(())
/// ```
///
/// # Timing Precision
///
/// Uses [`std::time::Instant`] for timing, which provides monotonic,
/// nanosecond-resolution measurements on most platforms.
pub fn run_closure<F>(spec: BenchSpec, f: F) -> Result<BenchReport, TimingError>
where
    F: FnMut() -> Result<(), TimingError>,
{
    let mut monitor = DefaultResourceMonitor::default();
    run_closure_with_monitor(spec, &mut monitor, f)
}

fn run_closure_with_monitor<F, M>(
    spec: BenchSpec,
    monitor: &mut M,
    mut f: F,
) -> Result<BenchReport, TimingError>
where
    F: FnMut() -> Result<(), TimingError>,
    M: ResourceMonitor,
{
    if spec.iterations == 0 {
        return Err(TimingError::NoIterations {
            count: spec.iterations,
        });
    }

    reset_semantic_phase_collection();
    let harness_origin = Instant::now();
    let mut timeline = Vec::new();

    // Warmup phase - not measured
    for iteration in 0..spec.warmup {
        let phase_start = Instant::now();
        f()?;
        push_timeline_span(
            &mut timeline,
            harness_origin,
            "warmup-benchmark",
            phase_start,
            Instant::now(),
            Some(iteration),
        );
    }

    // Measurement phase
    begin_semantic_phase_collection();
    let mut samples = Vec::with_capacity(spec.iterations as usize);
    for iteration in 0..spec.iterations {
        let (sample, start, end) = match measure_iteration(monitor, &mut f) {
            Ok(measurement) => measurement,
            Err(err) => {
                let _ = finish_semantic_phase_collection();
                return Err(err);
            }
        };
        samples.push(sample);
        push_timeline_span(
            &mut timeline,
            harness_origin,
            "measured-benchmark",
            start,
            end,
            Some(iteration),
        );
    }
    let phases = finish_semantic_phase_collection();

    Ok(BenchReport {
        spec,
        samples,
        phases,
        timeline,
    })
}

/// Runs a benchmark with setup that executes once before all iterations.
///
/// The setup function is called once before timing begins, then the benchmark
/// runs multiple times using a reference to the setup result. This is useful
/// for expensive initialization that shouldn't be included in timing.
///
/// # Arguments
///
/// * `spec` - Benchmark configuration specifying iterations and warmup
/// * `setup` - Function that creates the input data (called once, not timed)
/// * `f` - Benchmark closure that receives a reference to setup result
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::timing::{BenchSpec, run_closure_with_setup};
///
/// fn setup_data() -> Vec<u8> {
///     vec![0u8; 1_000_000]  // Expensive allocation not measured
/// }
///
/// let spec = BenchSpec::new("hash_benchmark", 100, 10)?;
/// let report = run_closure_with_setup(spec, setup_data, |data| {
///     std::hint::black_box(compute_hash(data));
///     Ok(())
/// })?;
/// ```
pub fn run_closure_with_setup<S, T, F>(
    spec: BenchSpec,
    setup: S,
    mut f: F,
) -> Result<BenchReport, TimingError>
where
    S: FnOnce() -> T,
    F: FnMut(&T) -> Result<(), TimingError>,
{
    let mut monitor = DefaultResourceMonitor::default();
    run_closure_with_setup_with_monitor(spec, &mut monitor, setup, move |input| f(input))
}

fn run_closure_with_setup_with_monitor<S, T, F, M>(
    spec: BenchSpec,
    monitor: &mut M,
    setup: S,
    mut f: F,
) -> Result<BenchReport, TimingError>
where
    S: FnOnce() -> T,
    F: FnMut(&T) -> Result<(), TimingError>,
    M: ResourceMonitor,
{
    if spec.iterations == 0 {
        return Err(TimingError::NoIterations {
            count: spec.iterations,
        });
    }

    reset_semantic_phase_collection();
    let harness_origin = Instant::now();
    let mut timeline = Vec::new();

    // Setup phase - not timed
    let setup_start = Instant::now();
    let input = setup();
    push_timeline_span(
        &mut timeline,
        harness_origin,
        "setup",
        setup_start,
        Instant::now(),
        None,
    );

    // Warmup phase - not recorded
    for iteration in 0..spec.warmup {
        let phase_start = Instant::now();
        f(&input)?;
        push_timeline_span(
            &mut timeline,
            harness_origin,
            "warmup-benchmark",
            phase_start,
            Instant::now(),
            Some(iteration),
        );
    }

    // Measurement phase
    begin_semantic_phase_collection();
    let mut samples = Vec::with_capacity(spec.iterations as usize);
    for iteration in 0..spec.iterations {
        let (sample, start, end) = match measure_iteration(monitor, || f(&input)) {
            Ok(measurement) => measurement,
            Err(err) => {
                let _ = finish_semantic_phase_collection();
                return Err(err);
            }
        };
        samples.push(sample);
        push_timeline_span(
            &mut timeline,
            harness_origin,
            "measured-benchmark",
            start,
            end,
            Some(iteration),
        );
    }
    let phases = finish_semantic_phase_collection();

    Ok(BenchReport {
        spec,
        samples,
        phases,
        timeline,
    })
}

/// Runs a benchmark with per-iteration setup.
///
/// Setup runs before each iteration and is not timed. The benchmark takes
/// ownership of the setup result, making this suitable for benchmarks that
/// mutate their input (e.g., sorting).
///
/// # Arguments
///
/// * `spec` - Benchmark configuration specifying iterations and warmup
/// * `setup` - Function that creates fresh input for each iteration (not timed)
/// * `f` - Benchmark closure that takes ownership of setup result
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::timing::{BenchSpec, run_closure_with_setup_per_iter};
///
/// fn generate_random_vec() -> Vec<i32> {
///     (0..1000).map(|_| rand::random()).collect()
/// }
///
/// let spec = BenchSpec::new("sort_benchmark", 100, 10)?;
/// let report = run_closure_with_setup_per_iter(spec, generate_random_vec, |mut data| {
///     data.sort();
///     std::hint::black_box(data);
///     Ok(())
/// })?;
/// ```
pub fn run_closure_with_setup_per_iter<S, T, F>(
    spec: BenchSpec,
    setup: S,
    f: F,
) -> Result<BenchReport, TimingError>
where
    S: FnMut() -> T,
    F: FnMut(T) -> Result<(), TimingError>,
{
    let mut monitor = DefaultResourceMonitor::default();
    run_closure_with_setup_per_iter_with_monitor(spec, &mut monitor, setup, f)
}

fn run_closure_with_setup_per_iter_with_monitor<S, T, F, M>(
    spec: BenchSpec,
    monitor: &mut M,
    mut setup: S,
    mut f: F,
) -> Result<BenchReport, TimingError>
where
    S: FnMut() -> T,
    F: FnMut(T) -> Result<(), TimingError>,
    M: ResourceMonitor,
{
    if spec.iterations == 0 {
        return Err(TimingError::NoIterations {
            count: spec.iterations,
        });
    }

    reset_semantic_phase_collection();
    let harness_origin = Instant::now();
    let mut timeline = Vec::new();

    // Warmup phase
    for iteration in 0..spec.warmup {
        let setup_start = Instant::now();
        let input = setup();
        push_timeline_span(
            &mut timeline,
            harness_origin,
            "fixture-setup",
            setup_start,
            Instant::now(),
            Some(iteration),
        );
        let phase_start = Instant::now();
        f(input)?;
        push_timeline_span(
            &mut timeline,
            harness_origin,
            "warmup-benchmark",
            phase_start,
            Instant::now(),
            Some(iteration),
        );
    }

    // Measurement phase
    begin_semantic_phase_collection();
    let mut samples = Vec::with_capacity(spec.iterations as usize);
    for iteration in 0..spec.iterations {
        let setup_start = Instant::now();
        let input = setup(); // Not timed
        push_timeline_span(
            &mut timeline,
            harness_origin,
            "fixture-setup",
            setup_start,
            Instant::now(),
            Some(iteration),
        );

        let (sample, start, end) = match measure_iteration(monitor, || f(input)) {
            Ok(measurement) => measurement,
            Err(err) => {
                let _ = finish_semantic_phase_collection();
                return Err(err);
            }
        };
        samples.push(sample);
        push_timeline_span(
            &mut timeline,
            harness_origin,
            "measured-benchmark",
            start,
            end,
            Some(iteration),
        );
    }
    let phases = finish_semantic_phase_collection();

    Ok(BenchReport {
        spec,
        samples,
        phases,
        timeline,
    })
}

/// Runs a benchmark with setup and teardown.
///
/// Setup runs once before all iterations, teardown runs once after all
/// iterations complete. Neither is included in timing.
///
/// # Arguments
///
/// * `spec` - Benchmark configuration specifying iterations and warmup
/// * `setup` - Function that creates the input data (called once, not timed)
/// * `f` - Benchmark closure that receives a reference to setup result
/// * `teardown` - Function that cleans up the input (called once, not timed)
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::timing::{BenchSpec, run_closure_with_setup_teardown};
///
/// fn setup_db() -> Database { Database::connect("test.db") }
/// fn cleanup_db(db: Database) { db.close(); std::fs::remove_file("test.db").ok(); }
///
/// let spec = BenchSpec::new("db_benchmark", 100, 10)?;
/// let report = run_closure_with_setup_teardown(
///     spec,
///     setup_db,
///     |db| { db.query("SELECT *"); Ok(()) },
///     cleanup_db,
/// )?;
/// ```
pub fn run_closure_with_setup_teardown<S, T, F, D>(
    spec: BenchSpec,
    setup: S,
    mut f: F,
    teardown: D,
) -> Result<BenchReport, TimingError>
where
    S: FnOnce() -> T,
    F: FnMut(&T) -> Result<(), TimingError>,
    D: FnOnce(T),
{
    let mut monitor = DefaultResourceMonitor::default();
    run_closure_with_setup_teardown_with_monitor(
        spec,
        &mut monitor,
        setup,
        move |input| f(input),
        teardown,
    )
}

fn run_closure_with_setup_teardown_with_monitor<S, T, F, D, M>(
    spec: BenchSpec,
    monitor: &mut M,
    setup: S,
    mut f: F,
    teardown: D,
) -> Result<BenchReport, TimingError>
where
    S: FnOnce() -> T,
    F: FnMut(&T) -> Result<(), TimingError>,
    D: FnOnce(T),
    M: ResourceMonitor,
{
    if spec.iterations == 0 {
        return Err(TimingError::NoIterations {
            count: spec.iterations,
        });
    }

    reset_semantic_phase_collection();
    let harness_origin = Instant::now();
    let mut timeline = Vec::new();

    // Setup phase - not timed
    let setup_start = Instant::now();
    let input = setup();
    push_timeline_span(
        &mut timeline,
        harness_origin,
        "setup",
        setup_start,
        Instant::now(),
        None,
    );

    let result = (|| {
        // Warmup phase
        for iteration in 0..spec.warmup {
            let phase_start = Instant::now();
            f(&input)?;
            push_timeline_span(
                &mut timeline,
                harness_origin,
                "warmup-benchmark",
                phase_start,
                Instant::now(),
                Some(iteration),
            );
        }

        // Measurement phase
        begin_semantic_phase_collection();
        let mut samples = Vec::with_capacity(spec.iterations as usize);
        for iteration in 0..spec.iterations {
            let (sample, start, end) = match measure_iteration(monitor, || f(&input)) {
                Ok(measurement) => measurement,
                Err(err) => {
                    let _ = finish_semantic_phase_collection();
                    return Err(err);
                }
            };
            samples.push(sample);
            push_timeline_span(
                &mut timeline,
                harness_origin,
                "measured-benchmark",
                start,
                end,
                Some(iteration),
            );
        }
        let phases = finish_semantic_phase_collection();

        Ok((samples, phases))
    })();

    // Teardown phase - not timed. It runs even when warmup/measurement fails.
    let teardown_start = Instant::now();
    teardown(input);
    push_timeline_span(
        &mut timeline,
        harness_origin,
        "teardown",
        teardown_start,
        Instant::now(),
        None,
    );

    let (samples, phases) = result?;
    Ok(BenchReport {
        spec,
        samples,
        phases,
        timeline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeResourceMonitor {
        samples: Vec<IterationResourceUsage>,
        started: usize,
        finished: usize,
    }

    impl FakeResourceMonitor {
        fn new(samples: Vec<IterationResourceUsage>) -> Self {
            Self {
                samples,
                started: 0,
                finished: 0,
            }
        }
    }

    impl ResourceMonitor for FakeResourceMonitor {
        type Token = usize;

        fn start(&mut self) -> Self::Token {
            let token = self.started;
            self.started += 1;
            assert!(
                token < self.samples.len(),
                "resource capture should only run for measured iterations"
            );
            token
        }

        fn finish(&mut self, token: Self::Token) -> IterationResourceUsage {
            self.finished += 1;
            self.samples
                .get(token)
                .cloned()
                .expect("resource usage for measured iteration")
        }
    }

    #[cfg(unix)]
    #[test]
    fn process_cpu_time_snapshot_sums_user_and_kernel_time() {
        let snapshot = ProcessCpuTimeSnapshot::from_rusage_timevals(
            libc::timeval {
                tv_sec: 1,
                tv_usec: 250_000,
            },
            libc::timeval {
                tv_sec: 0,
                tv_usec: 750_000,
            },
        )
        .expect("valid snapshot");

        assert_eq!(snapshot.total_ns(), 2_000_000_000);
    }

    #[cfg(unix)]
    #[test]
    fn process_cpu_time_delta_ms_uses_user_and_kernel_time() {
        let start = ProcessCpuTimeSnapshot::from_rusage_timevals(
            libc::timeval {
                tv_sec: 1,
                tv_usec: 250_000,
            },
            libc::timeval {
                tv_sec: 0,
                tv_usec: 750_000,
            },
        )
        .expect("valid start snapshot");
        let end = ProcessCpuTimeSnapshot::from_rusage_timevals(
            libc::timeval {
                tv_sec: 1,
                tv_usec: 900_000,
            },
            libc::timeval {
                tv_sec: 1,
                tv_usec: 400_600,
            },
        )
        .expect("valid end snapshot");

        assert_eq!(process_cpu_delta_ms(start, end), Some(1_301));
    }

    #[test]
    fn runs_benchmark_collects_requested_samples() {
        let spec = BenchSpec::new("noop", 3, 1).unwrap();
        let report = run_closure(spec, || Ok(())).unwrap();

        assert_eq!(report.samples.len(), 3);
        assert_eq!(report.spec.name, "noop");
        assert_eq!(report.spec.iterations, 3);
    }

    #[test]
    fn rejects_zero_iterations() {
        let result = BenchSpec::new("test", 0, 10);
        assert!(matches!(
            result,
            Err(TimingError::NoIterations { count: 0 })
        ));
    }

    #[test]
    fn allows_zero_warmup() {
        let spec = BenchSpec::new("test", 5, 0).unwrap();
        assert_eq!(spec.warmup, 0);

        let report = run_closure(spec, || Ok(())).unwrap();
        assert_eq!(report.samples.len(), 5);
    }

    #[test]
    fn serializes_to_json() {
        let report = BenchReport {
            spec: BenchSpec::new("test", 10, 2).unwrap(),
            samples: vec![BenchSample {
                duration_ns: 1_000_000,
                cpu_time_ms: Some(42),
                peak_memory_kb: Some(512),
                process_peak_memory_kb: Some(1536),
            }],
            phases: vec![SemanticPhase {
                name: "prove".to_string(),
                duration_ns: 1_000_000,
            }],
            timeline: vec![HarnessTimelineSpan {
                phase: "measured-benchmark".to_string(),
                start_offset_ns: 0,
                end_offset_ns: 1_000_000,
                iteration: Some(0),
            }],
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"peak_memory_kb\""));
        assert!(json.contains("\"process_peak_memory_kb\""));
        assert!(!json.contains("peak_memory_growth_kb"));
        let restored: BenchReport = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.spec.name, "test");
        assert_eq!(restored.samples.len(), 1);
        assert_eq!(restored.samples[0].cpu_time_ms, Some(42));
        assert_eq!(restored.samples[0].peak_memory_kb, Some(512));
        assert_eq!(restored.samples[0].process_peak_memory_kb, Some(1536));
        assert_eq!(restored.phases.len(), 1);
        assert_eq!(restored.phases[0].name, "prove");
        assert!(restored.phases[0].duration_ns > 0);
    }

    #[test]
    fn profile_phase_records_only_measured_iterations() {
        let spec = BenchSpec::new("semantic", 2, 1).unwrap();
        let mut call_index = 0u32;
        let report = run_closure(spec, || {
            let phase_name = if call_index == 0 {
                "warmup-only"
            } else {
                "prove"
            };
            call_index += 1;
            profile_phase(phase_name, || std::thread::sleep(Duration::from_millis(1)));
            Ok(())
        })
        .unwrap();

        assert!(
            !report
                .phases
                .iter()
                .any(|phase| phase.name == "warmup-only"),
            "warmup phases should not be recorded"
        );
        let prove = report
            .phases
            .iter()
            .find(|phase| phase.name == "prove")
            .expect("prove phase");
        assert!(prove.duration_ns > 0);
    }

    #[test]
    fn profile_phase_keeps_the_v1_model_flat() {
        let spec = BenchSpec::new("semantic-flat", 1, 0).unwrap();
        let report = run_closure(spec, || {
            profile_phase("prove", || {
                std::thread::sleep(Duration::from_millis(1));
                profile_phase("inner", || std::thread::sleep(Duration::from_millis(1)));
            });
            Ok(())
        })
        .unwrap();

        assert!(report.phases.iter().any(|phase| phase.name == "prove"));
        assert!(
            !report.phases.iter().any(|phase| phase.name == "inner"),
            "nested phases should not create a second flat phase entry"
        );
    }

    #[test]
    fn measured_cpu_excludes_warmup_iterations() {
        let spec = BenchSpec::new("cpu", 2, 1).unwrap();
        let mut monitor = FakeResourceMonitor::new(vec![
            IterationResourceUsage {
                cpu_time_ms: Some(11),
                peak_memory_kb: Some(32),
                ..Default::default()
            },
            IterationResourceUsage {
                cpu_time_ms: Some(17),
                peak_memory_kb: Some(64),
                ..Default::default()
            },
        ]);
        let mut calls = 0_u32;

        let report = run_closure_with_monitor(spec, &mut monitor, || {
            calls += 1;
            Ok(())
        })
        .unwrap();

        assert_eq!(calls, 3);
        assert_eq!(monitor.started, 2);
        assert_eq!(monitor.finished, 2);
        assert_eq!(
            report
                .samples
                .iter()
                .map(|sample| sample.cpu_time_ms)
                .collect::<Vec<_>>(),
            vec![Some(11), Some(17)]
        );
        assert_eq!(report.cpu_total_ms(), Some(28));
    }

    #[test]
    fn measured_cpu_excludes_outer_harness_and_report_overhead() {
        let spec = BenchSpec::new("cpu-harness", 2, 1).unwrap();
        let mut monitor = FakeResourceMonitor::new(vec![
            IterationResourceUsage {
                cpu_time_ms: Some(5),
                peak_memory_kb: Some(12),
                ..Default::default()
            },
            IterationResourceUsage {
                cpu_time_ms: Some(7),
                peak_memory_kb: Some(18),
                ..Default::default()
            },
        ]);

        let mut setup_calls = 0_u32;
        let mut teardown_calls = 0_u32;
        let report = run_closure_with_setup_teardown_with_monitor(
            spec,
            &mut monitor,
            || {
                setup_calls += 1;
                vec![1_u8, 2, 3]
            },
            |_fixture| Ok(()),
            |_fixture| {
                teardown_calls += 1;
            },
        )
        .unwrap();

        let _serialized = serde_json::to_string(&report).unwrap();

        assert_eq!(setup_calls, 1);
        assert_eq!(teardown_calls, 1);
        assert_eq!(monitor.started, 2);
        assert_eq!(report.cpu_total_ms(), Some(12));
        assert_eq!(report.cpu_median_ms(), Some(6));
    }

    #[test]
    fn setup_teardown_runs_teardown_when_warmup_fails() {
        let spec = BenchSpec::new("teardown-on-error", 1, 1).unwrap();
        let mut teardown_calls = 0_u32;

        let result = run_closure_with_setup_teardown(
            spec,
            || vec![1_u8, 2, 3],
            |_fixture| Err(TimingError::Execution("warmup failed".to_string())),
            |_fixture| {
                teardown_calls += 1;
            },
        );

        assert!(result.is_err());
        assert_eq!(teardown_calls, 1);
    }

    #[test]
    fn single_iteration_cpu_median_matches_the_measured_iteration() {
        let spec = BenchSpec::new("single", 1, 0).unwrap();
        let mut monitor = FakeResourceMonitor::new(vec![IterationResourceUsage {
            cpu_time_ms: Some(42),
            peak_memory_kb: Some(24),
            ..Default::default()
        }]);

        let report = run_closure_with_monitor(spec, &mut monitor, || Ok(())).unwrap();

        assert_eq!(report.samples[0].cpu_time_ms, Some(42));
        assert_eq!(report.cpu_total_ms(), Some(42));
        assert_eq!(report.cpu_median_ms(), Some(42));
    }

    #[test]
    fn multiple_iterations_export_the_median_cpu_sample() {
        let spec = BenchSpec::new("median", 3, 0).unwrap();
        let mut monitor = FakeResourceMonitor::new(vec![
            IterationResourceUsage {
                cpu_time_ms: Some(19),
                peak_memory_kb: Some(10),
                ..Default::default()
            },
            IterationResourceUsage {
                cpu_time_ms: Some(7),
                peak_memory_kb: Some(30),
                ..Default::default()
            },
            IterationResourceUsage {
                cpu_time_ms: Some(11),
                peak_memory_kb: Some(20),
                ..Default::default()
            },
        ]);

        let report = run_closure_with_monitor(spec, &mut monitor, || Ok(())).unwrap();

        assert_eq!(report.cpu_median_ms(), Some(11));
        assert_eq!(report.cpu_total_ms(), Some(37));
    }

    #[test]
    fn peak_memory_excludes_harness_baseline_overhead() {
        let spec = BenchSpec::new("memory", 2, 1).unwrap();
        let mut monitor = FakeResourceMonitor::new(vec![
            IterationResourceUsage {
                cpu_time_ms: Some(3),
                peak_memory_kb: Some(48),
                process_peak_memory_kb: Some(1_048),
            },
            IterationResourceUsage {
                cpu_time_ms: Some(4),
                peak_memory_kb: Some(96),
                process_peak_memory_kb: Some(1_096),
            },
        ]);

        let report = run_closure_with_setup_teardown_with_monitor(
            spec,
            &mut monitor,
            || vec![0_u8; 1024],
            |_fixture| Ok(()),
            |_fixture| {},
        )
        .unwrap();

        assert_eq!(
            report
                .samples
                .iter()
                .map(|sample| sample.peak_memory_kb)
                .collect::<Vec<_>>(),
            vec![Some(48), Some(96)]
        );
        assert_eq!(report.peak_memory_kb(), Some(96));
        assert_eq!(report.peak_memory_growth_kb(), report.peak_memory_kb());
        assert_eq!(report.process_peak_memory_kb(), Some(1_096));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn memory_peak_sampler_uses_the_first_post_startup_sample_as_its_baseline() {
        use std::collections::VecDeque;
        use std::sync::{Arc, Mutex};

        // Queue: [80=startup warmup, 100=baseline-on-Begin, 140, 120, ...]
        // After exhaustion the reader returns 120 forever, so peak stays 140.
        let samples = Arc::new(Mutex::new(VecDeque::from([
            Some(80_u64),
            Some(100_u64),
            Some(140_u64),
            Some(120_u64),
        ])));
        let reader_samples = Arc::clone(&samples);
        let reader = Arc::new(move || {
            reader_samples
                .lock()
                .expect("sample queue")
                .pop_front()
                .unwrap_or(Some(120))
        });

        let sampler = PersistentMemorySampler::start_with_reader(reader).expect("sampler");
        assert!(sampler.begin_window());
        let peak = sampler.end_window().expect("peak memory");

        assert_eq!(
            peak,
            ProcessMemoryPeak {
                growth_kb: 40,
                process_peak_kb: 140,
            }
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn persistent_memory_sampler_does_not_queue_result_when_begin_fails() {
        use std::collections::VecDeque;
        use std::sync::{Arc, Mutex};

        // Queue: [80=startup warmup, None=failed first baseline,
        // 100=second baseline, 130=final sample].
        let samples = Arc::new(Mutex::new(VecDeque::from([
            Some(80_u64),
            None,
            Some(100_u64),
            Some(130_u64),
        ])));
        let reader_samples = Arc::clone(&samples);
        let reader = Arc::new(move || {
            reader_samples
                .lock()
                .expect("sample queue")
                .pop_front()
                .unwrap_or(Some(130))
        });

        let sampler = PersistentMemorySampler::start_with_reader(reader).expect("sampler");
        assert!(!sampler.begin_window());
        assert!(sampler.begin_window());
        let peak = sampler
            .end_window()
            .expect("second window should receive its own result");

        assert_eq!(
            peak,
            ProcessMemoryPeak {
                growth_kb: 30,
                process_peak_kb: 130,
            }
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn persistent_memory_sampler_waits_for_baseline_before_begin_returns() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};

        let call_count = Arc::new(Mutex::new(0_u32));
        let (baseline_entered_tx, baseline_entered_rx) = mpsc::sync_channel(1);
        let (baseline_release_tx, baseline_release_rx) = mpsc::sync_channel(1);
        let baseline_release_rx = Arc::new(Mutex::new(baseline_release_rx));
        let baseline_released = Arc::new(AtomicBool::new(false));

        let reader_calls = Arc::clone(&call_count);
        let reader_release = Arc::clone(&baseline_release_rx);
        let reader = Arc::new(move || {
            let mut calls = reader_calls.lock().expect("call count");
            *calls += 1;
            let current = *calls;
            drop(calls);

            if current == 2 {
                baseline_entered_tx.send(()).expect("baseline entered");
                reader_release
                    .lock()
                    .expect("baseline release")
                    .recv()
                    .expect("release baseline");
                return Some(100);
            }

            Some(120)
        });

        let released = Arc::clone(&baseline_released);
        let release_handle = thread::spawn(move || {
            baseline_entered_rx.recv().expect("baseline read started");
            thread::sleep(Duration::from_millis(20));
            released.store(true, Ordering::SeqCst);
            baseline_release_tx.send(()).expect("release baseline");
        });

        let sampler = PersistentMemorySampler::start_with_reader(reader).expect("sampler");
        assert!(sampler.begin_window());
        assert!(
            baseline_released.load(Ordering::SeqCst),
            "begin_window returned before the baseline sample completed"
        );
        release_handle.join().expect("join release thread");

        let peak = sampler.end_window().expect("peak memory");
        assert_eq!(peak.growth_kb, 20);
        assert_eq!(peak.process_peak_kb, 120);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn persistent_memory_sampler_supports_multiple_windows() {
        use std::collections::VecDeque;
        use std::sync::{Arc, Mutex};

        // First window baseline=200 peak=260 (growth=60).
        // Second window baseline=190 peak=250 (growth=60).
        let samples = Arc::new(Mutex::new(VecDeque::from([
            Some(50_u64),  // startup warmup
            Some(200_u64), // window 1 baseline
            Some(260_u64), // window 1 peak
            Some(190_u64), // window 2 baseline
            Some(250_u64), // window 2 peak
        ])));
        let reader_samples = Arc::clone(&samples);
        let reader = Arc::new(move || {
            reader_samples
                .lock()
                .expect("sample queue")
                .pop_front()
                .unwrap_or(Some(0))
        });

        let sampler = PersistentMemorySampler::start_with_reader(reader).expect("sampler");

        assert!(sampler.begin_window());
        let first = sampler.end_window().expect("first peak");
        assert_eq!(first.process_peak_kb, 260);
        assert_eq!(first.growth_kb, 60);

        assert!(sampler.begin_window());
        let second = sampler.end_window().expect("second peak");
        assert_eq!(second.process_peak_kb, 250);
        assert_eq!(second.growth_kb, 60);
    }

    #[test]
    fn bench_report_deserializes_legacy_payload_without_phases_or_timeline() {
        // Wire format produced by mobench <= 0.1.34 (no phases / timeline /
        // resource fields). Adding these fields to BenchReport must not
        // break consumers that still emit the older shape.
        let legacy = r#"{
            "spec": { "name": "legacy", "iterations": 2, "warmup": 0 },
            "samples": [
                { "duration_ns": 100 },
                { "duration_ns": 200 }
            ]
        }"#;

        let report: BenchReport = serde_json::from_str(legacy).expect("legacy report parses");
        assert_eq!(report.samples.len(), 2);
        assert!(report.phases.is_empty());
        assert!(report.timeline.is_empty());
        assert!(report.samples[0].cpu_time_ms.is_none());
        assert!(report.samples[0].peak_memory_kb.is_none());
        assert!(report.samples[0].process_peak_memory_kb.is_none());

        // Round-trip the parsed report and confirm the empty optional
        // collections are skipped from the serialized output.
        let json = serde_json::to_string(&report).expect("serialize");
        assert!(!json.contains("\"phases\""));
        assert!(!json.contains("\"timeline\""));
    }

    #[test]
    fn run_with_setup_calls_setup_once() {
        use std::sync::atomic::{AtomicU32, Ordering};

        static SETUP_COUNT: AtomicU32 = AtomicU32::new(0);
        static RUN_COUNT: AtomicU32 = AtomicU32::new(0);

        let spec = BenchSpec::new("test", 5, 2).unwrap();
        let report = run_closure_with_setup(
            spec,
            || {
                SETUP_COUNT.fetch_add(1, Ordering::SeqCst);
                vec![1, 2, 3]
            },
            |data| {
                RUN_COUNT.fetch_add(1, Ordering::SeqCst);
                std::hint::black_box(data.len());
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(SETUP_COUNT.load(Ordering::SeqCst), 1); // Setup called once
        assert_eq!(RUN_COUNT.load(Ordering::SeqCst), 7); // 2 warmup + 5 iterations
        assert_eq!(report.samples.len(), 5);
    }

    #[test]
    fn run_with_setup_per_iter_calls_setup_each_time() {
        use std::sync::atomic::{AtomicU32, Ordering};

        static SETUP_COUNT: AtomicU32 = AtomicU32::new(0);

        let spec = BenchSpec::new("test", 3, 1).unwrap();
        let report = run_closure_with_setup_per_iter(
            spec,
            || {
                SETUP_COUNT.fetch_add(1, Ordering::SeqCst);
                vec![1, 2, 3]
            },
            |data| {
                std::hint::black_box(data);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(SETUP_COUNT.load(Ordering::SeqCst), 4); // 1 warmup + 3 iterations
        assert_eq!(report.samples.len(), 3);
    }

    #[test]
    fn run_with_setup_teardown_calls_both() {
        use std::sync::atomic::{AtomicU32, Ordering};

        static SETUP_COUNT: AtomicU32 = AtomicU32::new(0);
        static TEARDOWN_COUNT: AtomicU32 = AtomicU32::new(0);

        let spec = BenchSpec::new("test", 3, 1).unwrap();
        let report = run_closure_with_setup_teardown(
            spec,
            || {
                SETUP_COUNT.fetch_add(1, Ordering::SeqCst);
                "resource"
            },
            |_resource| Ok(()),
            |_resource| {
                TEARDOWN_COUNT.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap();

        assert_eq!(SETUP_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(TEARDOWN_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(report.samples.len(), 3);
    }

    #[test]
    fn bench_report_serializes_exact_harness_timeline() {
        let spec = BenchSpec::new("timeline", 2, 1).unwrap();
        let report = run_closure_with_setup_teardown(
            spec,
            || {
                std::thread::sleep(Duration::from_millis(1));
                "resource"
            },
            |_resource| {
                std::thread::sleep(Duration::from_millis(1));
                Ok(())
            },
            |_resource| {
                std::thread::sleep(Duration::from_millis(1));
            },
        )
        .unwrap();

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["timeline"][0]["phase"], "setup");
        assert_eq!(json["timeline"][1]["phase"], "warmup-benchmark");
        assert_eq!(json["timeline"][2]["phase"], "measured-benchmark");
        assert_eq!(json["timeline"][3]["phase"], "measured-benchmark");
        assert_eq!(json["timeline"][4]["phase"], "teardown");
    }
}
