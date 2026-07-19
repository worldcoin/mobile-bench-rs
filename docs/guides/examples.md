# Examples

Current release: **0.1.46**.

Use these examples as copy-paste starting points for benchmark crates and CI
invocations.

## Minimal Benchmark

```rust
use mobench_sdk::benchmark;

#[benchmark]
pub fn fibonacci_30() {
    let result = fibonacci(30);
    std::hint::black_box(result);
}

fn fibonacci(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}
```

Run it with the repository example:

```bash
cargo mobench list --crate-path examples/basic-benchmark
cargo mobench run \
  --target android \
  --function basic_benchmark::bench_fibonacci \
  --crate-path examples/basic-benchmark \
  --local-only \
  --iterations 20 \
  --warmup 5 \
  --output target/mobench/results.json
```

## Setup, Per-Iteration Setup, And Teardown

Setup runs outside measured iterations:

```rust
use mobench_sdk::benchmark;

fn create_input() -> Vec<u8> {
    vec![42; 1024 * 1024]
}

#[benchmark(setup = create_input)]
pub fn checksum(input: &Vec<u8>) {
    let sum: u64 = input.iter().map(|b| *b as u64).sum();
    std::hint::black_box(sum);
}
```

Per-iteration setup gives each sample a fresh value:

```rust
use mobench_sdk::benchmark;

fn unsorted_vec() -> Vec<i32> {
    (0..1000).rev().collect()
}

#[benchmark(setup = unsorted_vec, per_iteration)]
pub fn sort_vec(mut data: Vec<i32>) {
    data.sort();
    std::hint::black_box(data);
}
```

Teardown cleans up setup resources after measured work:

```rust
use mobench_sdk::benchmark;

fn setup_temp_file() -> TempFile {
    TempFile::new()
}

fn cleanup_temp_file(file: TempFile) {
    file.remove();
}

#[benchmark(setup = setup_temp_file, teardown = cleanup_temp_file)]
pub fn read_temp_file(file: &TempFile) {
    std::hint::black_box(file.read_all());
}
```

## Native C ABI Runner

Use `native-c-abi` when you want generated Android and iOS runners to call the
mobench JSON C ABI directly.

`mobench.toml`:

```toml
[project]
crate = "my-bench-crate"
library_name = "my_bench_crate"
ffi_backend = "native-c-abi"
```

Crate root:

```rust
mobench_sdk::export_native_c_abi!();
```

The generated runners call `mobench_run_benchmark_json` and free returned
buffers with `mobench_free_buf`.

## CI Output Example

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --plots auto \
  --output-dir target/mobench/ci
```

The output directory contains:

- `summary.json`
- `summary.md`
- `results.csv`
- `plots/*.svg` when plot rendering is available

`results.csv` includes timing columns such as `mean_ns`, `median_ns`, `p95_ns`,
`min_ns`, and `max_ns`. Resource columns include `cpu_total_ms`,
`cpu_median_ms`, `peak_memory_kb`, `peak_memory_growth_kb`, and
`process_peak_memory_kb` when those values are available.

## BrowserStack Android Example

```bash
export BROWSERSTACK_USERNAME="your_username"
export BROWSERSTACK_ACCESS_KEY="your_access_key"

cargo mobench run \
  --target android \
  --function sample_fns::checksum \
  --devices "Google Pixel 7-13.0" \
  --iterations 30 \
  --warmup 5 \
  --release \
  --fetch \
  --output target/mobench/results.json
```

## BrowserStack iOS Example

`mobench run` can package iOS app and XCUITest artifacts automatically when
`--ios-app` and `--ios-test-suite` are not provided:

```bash
cargo mobench run \
  --target ios \
  --function sample_fns::fibonacci \
  --devices "iPhone 14-16" \
  --iterations 20 \
  --warmup 3 \
  --release \
  --output target/mobench/results.json
```

You can also package explicitly:

```bash
cargo mobench package-ipa --method adhoc
cargo mobench package-xcuitest
```

## Programmatic Integration

For runtime integration, depend on `mobench-sdk` and call the public timing
APIs:

```rust
use mobench_sdk::{run_benchmark, BenchSpec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = BenchSpec::new("sample_fns::fibonacci", 100, 10)?;
    let report = run_benchmark(spec)?;

    println!("mean: {} ns", report.mean_ns());
    println!("median: {} ns", report.median_ns());
    Ok(())
}
```

For CI summaries, prefer the CLI/file contract over private crate modules:

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --local-only \
  --plots auto \
  --output-dir target/mobench/ci
```

Read:

- `target/mobench/ci/summary.json`
- `target/mobench/ci/summary.md`
- `target/mobench/ci/results.csv`

The full contract is described in
[../specs/mobench-current-spec.md](../specs/mobench-current-spec.md).
