//! Compile-time receipt for the published v0.1.43 `mobench-sdk` facade.

use mobench_sdk::{
    BenchError, BenchFunction, BenchSample, BenchSpec, BenchSummary, BenchmarkBuilder, BuildConfig,
    BuildProfile, BuildResult, FfiBackend, HarnessTimelineSpan, InitConfig, MobenchBuf,
    NativeLibraryArtifact, RunnerReport, SemanticPhase, Target, TimingError,
};

#[mobench_sdk::benchmark]
fn public_macro_fixture() {
    mobench_sdk::black_box(42_u64);
}

mobench_sdk::debug_benchmarks!();

#[test]
fn v0_1_43_root_types_remain_nameable() {
    let _: Option<BenchError> = None;
    let _: Option<BenchFunction> = None;
    let _: Option<BenchSample> = None;
    let _: Option<BenchSpec> = None;
    let _: Option<BenchSummary> = None;
    let _: Option<BenchmarkBuilder> = None;
    let _: Option<BuildConfig> = None;
    let _: Option<BuildProfile> = None;
    let _: Option<BuildResult> = None;
    let _: Option<FfiBackend> = None;
    let _: Option<HarnessTimelineSpan> = None;
    let _: Option<InitConfig> = None;
    let _: Option<MobenchBuf> = None;
    let _: Option<NativeLibraryArtifact> = None;
    let _: Option<RunnerReport> = None;
    let _: Option<SemanticPhase> = None;
    let _: Option<Target> = None;
    let _: Option<TimingError> = None;
}

#[test]
fn v0_1_43_functions_modules_and_macros_remain_reachable() {
    let _ = mobench_sdk::discover_benchmarks;
    let _ = mobench_sdk::find_benchmark;
    let _ = mobench_sdk::list_benchmark_names;
    let _ = mobench_sdk::run_benchmark;
    assert_eq!(mobench_sdk::profile_phase("contract", || 1_u8), 1);
    let _ = mobench_sdk::run_closure::<fn() -> Result<(), TimingError>>;
    let _: Option<mobench_sdk::ffi::BenchReportFfi> = None;
    let _: Option<mobench_sdk::uniffi_types::BenchReportTemplate> = None;
    let _ = mobench_sdk::native_c_abi::mobench_run_benchmark_json_impl;
    let _ = mobench_sdk::codegen::generate_project;
    let _ = mobench_sdk::builders::common::create_bench_meta;
    let _ = mobench_sdk::builders::common::run_command;
    let _ = mobench_sdk::VERSION;

    let discovered = mobench_sdk::list_benchmark_names();
    assert!(
        discovered
            .iter()
            .any(|name| name.ends_with("public_macro_fixture"))
    );
}
