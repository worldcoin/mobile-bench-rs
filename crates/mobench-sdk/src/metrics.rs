//! Custom benchmark metrics captured alongside the timed samples.
//!
//! The timing harness deliberately owns the clock. Benchmark functions can
//! record small scalar outputs, such as serialized proof length, and the
//! native JSON ABI attaches them to the report after timing has completed.

use {
    serde::Serialize,
    std::{cell::RefCell, collections::BTreeMap},
};

/// Additional scalar metrics emitted by one native benchmark run.
#[derive(Debug, Default, Serialize)]
pub struct CustomMetrics {
    /// One value per warmup or measured invocation, in execution order.
    pub sample_u64: BTreeMap<String, Vec<u64>>,
    /// Run-wide values such as a deduplicated proving-payload size.
    pub run_u64: BTreeMap<String, u64>,
}

thread_local! {
    static CUSTOM_METRICS: RefCell<CustomMetrics> =
        RefCell::new(CustomMetrics::default());
}

/// Records an unsigned scalar for the current benchmark invocation.
///
/// Values are retained in invocation order, including warmups. Recording uses
/// thread-local storage so it does not acquire a process-wide lock. Call this
/// near the end of the benchmark function after producing the measured output.
pub fn record_sample_u64(name: impl Into<String>, value: u64) {
    CUSTOM_METRICS.with(|metrics| {
        metrics
            .borrow_mut()
            .sample_u64
            .entry(name.into())
            .or_default()
            .push(value);
    });
}

/// Records or replaces an unsigned run-wide scalar.
///
/// This is intended for setup code that runs outside the measured region.
pub fn record_run_u64(name: impl Into<String>, value: u64) {
    CUSTOM_METRICS.with(|metrics| {
        metrics.borrow_mut().run_u64.insert(name.into(), value);
    });
}

#[cfg(feature = "registry")]
pub(crate) fn clear() {
    CUSTOM_METRICS.with(|metrics| {
        *metrics.borrow_mut() = CustomMetrics::default();
    });
}

#[cfg(feature = "registry")]
pub(crate) fn take() -> CustomMetrics {
    CUSTOM_METRICS.with(|metrics| std::mem::take(&mut *metrics.borrow_mut()))
}

impl CustomMetrics {
    #[cfg(feature = "registry")]
    pub(crate) fn is_empty(&self) -> bool {
        self.sample_u64.is_empty() && self.run_u64.is_empty()
    }
}
