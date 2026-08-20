use mobench_domain::{BoundRunReportV2, ReportCounts};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Canonical compatibility summary consumed by every report adapter.
///
/// `T` is the caller's platform type. Keeping it generic lets the report
/// Module remain independent of CLI parsing while preserving the released
/// serialized target representation.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SummaryReport<T> {
    pub generated_at: String,
    pub generated_at_unix: u64,
    pub target: T,
    pub function: String,
    pub iterations: u32,
    pub warmup: u32,
    pub devices: Vec<String>,
    pub device_summaries: Vec<DeviceSummary>,
}

/// Results attributed to one device.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeviceSummary {
    pub device: String,
    pub benchmarks: Vec<BenchmarkStats>,
}

/// Canonical statistics and diagnostics for one benchmark function.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BenchmarkStats {
    pub function: String,
    pub samples: usize,
    pub mean_ns: Option<u64>,
    pub median_ns: Option<u64>,
    pub p95_ns: Option<u64>,
    pub min_ns: Option<u64>,
    pub max_ns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_usage: Option<BenchmarkResourceUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<BenchmarkFailureStats>,
}

/// Stable failure projection used by compatibility and CI reports.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BenchmarkFailureStats {
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_reason: Option<String>,
}

/// Canonical resource statistics associated with a benchmark.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct BenchmarkResourceUsage {
    pub cpu_total_ms: Option<u64>,
    pub cpu_median_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_cpu_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affinity_cpu_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rayon_num_threads_env: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_cpu_cores_median: Option<f64>,
    /// Legacy alias for `peak_memory_growth_kb`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_memory_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_memory_growth_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_peak_memory_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_pss_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_dirty_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_heap_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_heap_kb: Option<u64>,
}

impl BenchmarkResourceUsage {
    #[must_use]
    pub fn peak_memory_growth_or_legacy_kb(&self) -> Option<u64> {
        self.peak_memory_growth_kb.or(self.peak_memory_kb)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cpu_total_ms.is_none()
            && self.cpu_median_ms.is_none()
            && self.logical_cpu_count.is_none()
            && self.affinity_cpu_count.is_none()
            && self.rayon_num_threads_env.is_none()
            && self.effective_cpu_cores_median.is_none()
            && self.peak_memory_kb.is_none()
            && self.peak_memory_growth_kb.is_none()
            && self.process_peak_memory_kb.is_none()
            && self.total_pss_kb.is_none()
            && self.private_dirty_kb.is_none()
            && self.native_heap_kb.is_none()
            && self.java_heap_kb.is_none()
    }
}

/// Command-level terminal state included in the canonical v2 report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunOutcome {
    Complete {
        expected_sessions: usize,
        successful_sessions: usize,
    },
    Partial {
        expected_sessions: usize,
        successful_sessions: usize,
    },
    Failed {
        expected_sessions: usize,
        successful_sessions: usize,
    },
}

impl RunOutcome {
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete { .. })
    }
}

/// Canonical authenticated report published as `summary.v2.json`.
#[derive(Debug, Serialize)]
pub struct CanonicalSummaryV2<'a, T> {
    pub schema_version: &'static str,
    pub run_id: &'a str,
    pub target: T,
    pub function_id: &'a str,
    pub requested: ReportCounts,
    pub lifecycle: RunOutcome,
    pub reports: &'a [BoundRunReportV2],
}

impl<'a, T> CanonicalSummaryV2<'a, T> {
    #[must_use]
    pub fn new(
        run_id: &'a str,
        target: T,
        function_id: &'a str,
        requested: ReportCounts,
        lifecycle: RunOutcome,
        reports: &'a [BoundRunReportV2],
    ) -> Self {
        Self {
            schema_version: "mobench.summary/v2",
            run_id,
            target,
            function_id,
            requested,
            lifecycle,
            reports,
        }
    }
}

/// Deterministic comparison between two canonical summaries.
#[derive(Debug, Serialize, Clone)]
pub struct CompareReport {
    pub baseline: PathBuf,
    pub candidate: PathBuf,
    pub rows: Vec<CompareRow>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CompareRow {
    pub device: String,
    pub function: String,
    pub baseline_median_ns: Option<u64>,
    pub candidate_median_ns: Option<u64>,
    pub median_delta_pct: Option<f64>,
    pub median_label: String,
    pub baseline_p95_ns: Option<u64>,
    pub candidate_p95_ns: Option<u64>,
    pub p95_delta_pct: Option<f64>,
    pub p95_label: String,
}

/// One metric whose candidate value crosses the configured regression gate.
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionFinding {
    pub device: String,
    pub function: String,
    pub metric: String,
    pub delta_pct: f64,
}
