//! JSON bridge for browser-hosted WebAssembly benchmark runners.
//!
//! The bridge deliberately accepts and returns strings so browser automation
//! can use the same [`crate::BenchSpec`] and [`crate::RunnerReport`] JSON
//! contract as native runners without depending on JavaScript object
//! conversion details.

use crate::{BenchError, BenchSpec};
use thiserror::Error;

/// Errors produced by the browser JSON bridge.
#[derive(Debug, Error)]
pub enum BrowserRunnerError {
    /// The input was not a valid serialized [`BenchSpec`].
    #[error("failed to parse BenchSpec JSON: {0}")]
    InvalidSpec(serde_json::Error),
    /// Benchmark lookup or execution failed.
    #[error("failed to run benchmark: {0}")]
    Benchmark(#[from] BenchError),
    /// The resulting [`crate::RunnerReport`] could not be serialized.
    #[error("failed to serialize RunnerReport JSON: {0}")]
    SerializeReport(serde_json::Error),
}

/// Runs a registered benchmark from a serialized [`BenchSpec`].
///
/// On success, the returned string is a serialized [`crate::RunnerReport`].
/// Browser bundles can expose this as `window.mobench.run(spec)` and pass the
/// result directly back through WebDriver.
pub fn run_benchmark_json(spec_json: &str) -> Result<String, BrowserRunnerError> {
    let spec =
        serde_json::from_str::<BenchSpec>(spec_json).map_err(BrowserRunnerError::InvalidSpec)?;
    let report = crate::run_benchmark(spec)?;
    serde_json::to_string(&report).map_err(BrowserRunnerError::SerializeReport)
}

/// `wasm-bindgen` export used by generated browser bundles.
///
/// JavaScript name: `runBenchmarkJson`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = runBenchmarkJson)]
pub fn run_benchmark_json_wasm(spec_json: &str) -> Result<String, wasm_bindgen::JsValue> {
    run_benchmark_json(spec_json)
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::{BenchReport, TimingError, run_closure};

    fn run_test_benchmark(spec: BenchSpec) -> Result<BenchReport, TimingError> {
        run_closure(spec, || {
            std::hint::black_box(21_u64.saturating_mul(2));
            Ok(())
        })
    }

    inventory::submit! {
        crate::registry::BenchFunction {
            name: "mobench_sdk::web::tests::browser_json_contract",
            runner: run_test_benchmark,
        }
    }

    #[test]
    fn returns_runner_report_json() {
        let result =
            run_benchmark_json(r#"{"name":"browser_json_contract","iterations":2,"warmup":1}"#)
                .expect("browser benchmark should run");
        let report: crate::RunnerReport =
            serde_json::from_str(&result).expect("valid RunnerReport JSON");

        assert_eq!(report.spec.name, "browser_json_contract");
        assert_eq!(report.samples.len(), 2);
    }

    #[test]
    fn rejects_invalid_spec_json() {
        let error = run_benchmark_json("{not json}").expect_err("invalid JSON should fail");

        assert!(matches!(error, BrowserRunnerError::InvalidSpec(_)));
        assert!(error.to_string().contains("failed to parse BenchSpec JSON"));
    }

    #[test]
    fn rejects_out_of_bounds_spec() {
        let error = run_benchmark_json(
            r#"{"name":"browser_json_contract","iterations":1000001,"warmup":0}"#,
        )
        .expect_err("oversized iteration count should fail during deserialization");

        assert!(matches!(error, BrowserRunnerError::InvalidSpec(_)));
        assert!(error.to_string().contains("must not exceed"));
    }

    #[test]
    fn reports_unknown_benchmark() {
        let error = run_benchmark_json(
            r#"{"name":"definitely_missing_browser_benchmark","iterations":1,"warmup":0}"#,
        )
        .expect_err("unknown function should fail");

        assert!(matches!(
            error,
            BrowserRunnerError::Benchmark(BenchError::UnknownFunction(_, _))
        ));
    }
}
