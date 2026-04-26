use criterion::{Criterion, criterion_group, criterion_main};
use mobench::bench_support;

const RUN_CONFIG: &str = r#"
target = "android"
function = "sample_fns::fibonacci"
iterations = 100
warmup = 10
device_matrix = "device-matrix.yaml"
device_tags = ["default", "high-spec"]

[browserstack]
app_automate_username = "${BROWSERSTACK_USERNAME}"
app_automate_access_key = "${BROWSERSTACK_ACCESS_KEY}"
project = "mobench"
"#;

const DEVICE_MATRIX: &str = r#"
devices:
  - name: Google Pixel 8
    os: android
    os_version: "14.0"
    tags: ["default", "high-spec"]
  - name: iPhone 16 Pro
    os: ios
    os_version: "18"
    tags: ["default", "high-spec"]
"#;

const SUMMARY_JSON: &str = include_str!("../../../examples/fixtures/basic/summary.json");

const PROFILE_JSON: &str = r#"
{
  "run_id": "android-sample-fns--fibonacci",
  "target": "android",
  "function": "sample_fns::fibonacci",
  "provider": "local",
  "backend": "android-native",
  "format": "both",
  "native_capture": {
    "status": "captured",
    "raw_artifacts": [],
    "processed_artifacts": [],
    "symbolization": {
      "status": "captured",
      "tool": "simpleperf",
      "resolved_frames": 42,
      "unresolved_frames": 3,
      "notes": []
    },
    "viewer_hint": "Open artifacts/processed/flamegraph.html"
  },
  "semantic_profile": {
    "status": "captured",
    "phases": [
      {
        "name": "fibonacci",
        "duration_ns": 100000,
        "percent_total": 100
      }
    ],
    "spans_path": "artifacts/semantic/phases.json",
    "harness_timeline": [],
    "timeline_path": "artifacts/semantic/timeline.json"
  },
  "capture_metadata": {
    "device": "Google Pixel 8",
    "os": "Android 14",
    "sample_duration_secs": 10,
    "benchmark_iterations": 100,
    "benchmark_warmup": 10,
    "warmup_mode": "warm",
    "capture_method": "simpleperf/app_profiler.py",
    "warnings": []
  }
}
"#;

const BROWSERSTACK_LOGS: &str = r#"
04-26 10:00:00.000 I/BenchRunner: starting benchmark
04-26 10:00:01.000 I/BenchRunner: BENCH_JSON {"function":"sample_fns::fibonacci","samples":[{"duration_ns":95000000},{"duration_ns":98000000}],"mean_ns":96500000}
04-26 10:00:02.000 I/BenchRunner: finished benchmark
"#;

fn host_contract_benchmarks(c: &mut Criterion) {
    c.bench_function("config/parse_run_config", |b| {
        b.iter(|| bench_support::parse_run_config(RUN_CONFIG).expect("parse run config"))
    });

    c.bench_function("config/parse_device_matrix", |b| {
        b.iter(|| bench_support::parse_device_matrix(DEVICE_MATRIX).expect("parse matrix"))
    });

    c.bench_function("summary/render_markdown", |b| {
        b.iter(|| {
            bench_support::render_summary_markdown_from_json(SUMMARY_JSON)
                .expect("render summary markdown")
        })
    });

    c.bench_function("summary/render_csv", |b| {
        b.iter(|| {
            bench_support::render_summary_csv_from_json(SUMMARY_JSON).expect("render summary csv")
        })
    });

    c.bench_function("profile/render_markdown", |b| {
        b.iter(|| {
            bench_support::render_profile_markdown_from_json(PROFILE_JSON)
                .expect("render profile markdown")
        })
    });

    c.bench_function("browserstack/extract_results", |b| {
        b.iter(|| {
            bench_support::extract_browserstack_results_from_logs(BROWSERSTACK_LOGS)
                .expect("extract benchmark results")
        })
    });
}

criterion_group!(benches, host_contract_benchmarks);
criterion_main!(benches);
