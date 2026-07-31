//! Small, provider-free runtime primitives shared by Mobench surfaces.

use std::sync::OnceLock;

/// Maximum accepted measured or warmup iteration count at runtime boundaries.
pub const MAX_BENCHMARK_COUNT: u32 = 1_000_000;

/// Saturate a `u128` value into the public `u64` wire range.
#[must_use]
pub fn saturating_u128_to_u64(value: u128) -> u64 {
    value.min(u128::from(u64::MAX)) as u64
}

/// Saturate an in-memory collection length into the public `u32` wire range.
#[must_use]
pub fn saturating_usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Sum `u64` values with a `u128` accumulator and saturate at `u64::MAX`.
#[must_use]
pub fn saturating_sum_u64(values: impl IntoIterator<Item = u64>) -> u64 {
    saturating_u128_to_u64(values.into_iter().fold(0_u128, |total, value| {
        total.saturating_add(u128::from(value))
    }))
}

/// Round `part / total` to the nearest whole percent without intermediate overflow.
#[must_use]
pub fn rounded_percent_u64(part: u64, total: u64) -> Option<u64> {
    if total == 0 {
        return None;
    }
    let rounded = (u128::from(part) * 100 + (u128::from(total) / 2)) / u128::from(total);
    Some(saturating_u128_to_u64(rounded))
}

/// Calculate the released SDK's floating mean over an allocation-free iterator.
#[must_use]
pub fn sdk_v1_mean_u64(values: impl IntoIterator<Item = u64>) -> f64 {
    let (sum, count) = values
        .into_iter()
        .fold((0_u128, 0_usize), |(sum, count), value| {
            (sum.saturating_add(u128::from(value)), count + 1)
        });
    if count == 0 {
        0.0
    } else {
        sum as f64 / count as f64
    }
}

/// Calculate the released SDK's sample standard deviation without allocating.
#[must_use]
pub fn sdk_v1_std_dev_u64<I>(values: I) -> f64
where
    I: Iterator<Item = u64> + Clone,
{
    let (sum, count) = values
        .clone()
        .fold((0_u128, 0_usize), |(sum, count), value| {
            (sum.saturating_add(u128::from(value)), count + 1)
        });
    if count < 2 {
        return 0.0;
    }
    let mean = sum as f64 / count as f64;
    let variance = values
        .map(|value| {
            let difference = value as f64 - mean;
            difference * difference
        })
        .sum::<f64>()
        / (count - 1) as f64;
    variance.sqrt()
}

/// A timing distribution used by compatibility-specific summaries.
///
/// Sorting is lazy, so mean-only SDK calls do not pay an allocation or sort.
#[derive(Debug)]
pub struct Distribution<'a> {
    samples: DistributionSamples<'a>,
    sorted: OnceLock<Vec<u64>>,
    sum: u128,
}

#[derive(Debug)]
enum DistributionSamples<'a> {
    Borrowed(&'a [u64]),
    Sorted(Vec<u64>),
}

/// The released CLI's integer summary semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliV1Summary {
    pub mean_ns: u64,
    pub median_ns: u64,
    pub p95_ns: u64,
    pub min_ns: u64,
    pub max_ns: u64,
}

/// The released SDK's floating-point summary semantics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SdkV1Summary {
    pub mean_ns: f64,
    pub median_ns: f64,
    pub std_dev_ns: f64,
    pub min_ns: u64,
    pub max_ns: u64,
    pub p95_ns: f64,
    pub p99_ns: f64,
}

/// Optional resource measurements for one benchmark iteration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResourceSample {
    pub cpu_time_ms: Option<u64>,
    pub peak_memory_growth_kb: Option<u64>,
    pub process_peak_memory_kb: Option<u64>,
}

/// Aggregated resource measurements across the samples that provided them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResourceAggregate {
    pub cpu_total_ms: Option<u64>,
    pub cpu_median_ms: Option<u64>,
    pub peak_memory_growth_kb: Option<u64>,
    pub process_peak_memory_kb: Option<u64>,
}

/// Incrementally aggregate partial resource measurements without overflow.
#[derive(Debug, Default)]
pub struct ResourceAccumulator {
    cpu_samples: Vec<u64>,
    cpu_total_ms: u128,
    peak_memory_growth_kb: Option<u64>,
    process_peak_memory_kb: Option<u64>,
}

impl ResourceAccumulator {
    /// Create an empty resource accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the fields present for one benchmark iteration.
    pub fn record(&mut self, sample: ResourceSample) {
        if let Some(cpu_time_ms) = sample.cpu_time_ms {
            self.cpu_samples.push(cpu_time_ms);
            self.cpu_total_ms = self.cpu_total_ms.saturating_add(u128::from(cpu_time_ms));
        }
        if let Some(peak_memory_growth_kb) = sample.peak_memory_growth_kb {
            self.peak_memory_growth_kb = Some(
                self.peak_memory_growth_kb
                    .map_or(peak_memory_growth_kb, |current| {
                        current.max(peak_memory_growth_kb)
                    }),
            );
        }
        if let Some(process_peak_memory_kb) = sample.process_peak_memory_kb {
            self.process_peak_memory_kb = Some(
                self.process_peak_memory_kb
                    .map_or(process_peak_memory_kb, |current| {
                        current.max(process_peak_memory_kb)
                    }),
            );
        }
    }

    /// Finish aggregation, consuming the accumulator.
    #[must_use]
    pub fn finish(mut self) -> ResourceAggregate {
        self.cpu_samples.sort_unstable();
        let cpu_median_ms = match self.cpu_samples.len() {
            0 => None,
            len if len % 2 == 1 => Some(self.cpu_samples[len / 2]),
            len => {
                let lower = u128::from(self.cpu_samples[(len / 2) - 1]);
                let upper = u128::from(self.cpu_samples[len / 2]);
                Some(saturating_u128_to_u64((lower + upper) / 2))
            }
        };

        ResourceAggregate {
            cpu_total_ms: (!self.cpu_samples.is_empty())
                .then_some(saturating_u128_to_u64(self.cpu_total_ms)),
            cpu_median_ms,
            peak_memory_growth_kb: self.peak_memory_growth_kb,
            process_peak_memory_kb: self.process_peak_memory_kb,
        }
    }
}

impl<'a> Distribution<'a> {
    /// Borrow nanosecond samples and prepare safe shared accumulation.
    #[must_use]
    pub fn from_slice(samples: &'a [u64]) -> Self {
        let sum = samples
            .iter()
            .fold(0_u128, |total, sample| total + u128::from(*sample));
        Self {
            samples: DistributionSamples::Borrowed(samples),
            sorted: OnceLock::new(),
            sum,
        }
    }

    /// Own and eagerly sort nanosecond samples for one-allocation order statistics.
    #[must_use]
    pub fn from_vec(mut samples: Vec<u64>) -> Self {
        let sum = samples
            .iter()
            .fold(0_u128, |total, sample| total + u128::from(*sample));
        samples.sort_unstable();
        Self {
            samples: DistributionSamples::Sorted(samples),
            sorted: OnceLock::new(),
            sum,
        }
    }

    /// Summarize with the released CLI's floor and nearest-rank rules.
    #[must_use]
    pub fn cli_v1_summary(&self) -> Option<CliV1Summary> {
        let len = self.values().len();
        if len == 0 {
            return None;
        }
        let sorted = self.sorted();

        let median_ns = if len % 2 == 1 {
            sorted[len / 2]
        } else {
            let lower = u128::from(sorted[(len / 2) - 1]);
            let upper = u128::from(sorted[len / 2]);
            saturating_u128_to_u64((lower + upper) / 2)
        };
        let p95_rank = (95_u128 * len as u128).div_ceil(100) as usize;

        Some(CliV1Summary {
            mean_ns: saturating_u128_to_u64(self.sum / len as u128),
            median_ns,
            p95_ns: sorted[p95_rank.saturating_sub(1)],
            min_ns: sorted[0],
            max_ns: sorted[len - 1],
        })
    }

    /// Summarize with the released SDK's floating-point and rounded-index rules.
    #[must_use]
    pub fn sdk_v1_summary(&self) -> SdkV1Summary {
        if self.values().is_empty() {
            return SdkV1Summary {
                mean_ns: 0.0,
                median_ns: 0.0,
                std_dev_ns: 0.0,
                min_ns: 0,
                max_ns: 0,
                p95_ns: 0.0,
                p99_ns: 0.0,
            };
        }

        SdkV1Summary {
            mean_ns: self.sdk_v1_mean(),
            median_ns: self.sdk_v1_median(),
            std_dev_ns: self.sdk_v1_std_dev(),
            min_ns: self.min().unwrap_or(0),
            max_ns: self.max().unwrap_or(0),
            p95_ns: self.sdk_v1_percentile(95.0),
            p99_ns: self.sdk_v1_percentile(99.0),
        }
    }

    /// Return the SDK's floating-point mean, or zero for no samples.
    #[must_use]
    pub fn sdk_v1_mean(&self) -> f64 {
        if self.values().is_empty() {
            0.0
        } else {
            sdk_v1_mean_u64(self.values().iter().copied())
        }
    }

    /// Return the SDK's floating-point median, or zero for no samples.
    #[must_use]
    pub fn sdk_v1_median(&self) -> f64 {
        let len = self.values().len();
        if len == 0 {
            return 0.0;
        }
        let sorted = self.sorted();
        if len % 2 == 1 {
            sorted[len / 2] as f64
        } else {
            (sorted[(len / 2) - 1] as f64 + sorted[len / 2] as f64) / 2.0
        }
    }

    /// Return the SDK's sample standard deviation, or zero below two samples.
    #[must_use]
    pub fn sdk_v1_std_dev(&self) -> f64 {
        sdk_v1_std_dev_u64(self.values().iter().copied())
    }

    /// Return the minimum sample without sorting.
    #[must_use]
    pub fn min(&self) -> Option<u64> {
        self.values().iter().copied().min()
    }

    /// Return the maximum sample without sorting.
    #[must_use]
    pub fn max(&self) -> Option<u64> {
        self.values().iter().copied().max()
    }

    /// Select a percentile with the SDK's clamped, rounded `(n - 1)` index.
    #[must_use]
    pub fn sdk_v1_percentile(&self, percentile: f64) -> f64 {
        if self.values().is_empty() {
            return 0.0;
        }
        let sorted = self.sorted();
        let percentile = percentile.clamp(0.0, 100.0) / 100.0;
        let index = (percentile * (sorted.len() - 1) as f64).round() as usize;
        sorted[index.min(sorted.len() - 1)] as f64
    }

    fn sorted(&self) -> &[u64] {
        match &self.samples {
            DistributionSamples::Sorted(samples) => samples,
            DistributionSamples::Borrowed(samples) => self.sorted.get_or_init(|| {
                let mut sorted = samples.to_vec();
                sorted.sort_unstable();
                sorted
            }),
        }
    }

    fn values(&self) -> &[u64] {
        match &self.samples {
            DistributionSamples::Borrowed(samples) => samples,
            DistributionSamples::Sorted(samples) => samples,
        }
    }
}
