# mobench-sdk

Mobile benchmarking SDK for Rust - run benchmarks on real Android and iOS devices.

Transform your Rust project into a mobile benchmarking suite. This SDK provides everything you need to benchmark your Rust code on real mobile devices via BrowserStack or local emulators/simulators.

## Features

- **`#[benchmark]` macro**: Mark functions for mobile benchmarking
- **Automatic registry**: Compile-time function discovery
- **Mobile app generation**: Create Android/iOS apps from templates
- **Build automation**: Cross-compile and package for mobile platforms
- **Statistical analysis**: Mean, median, stddev, percentiles
- **BrowserStack integration**: Test on real devices in the cloud
- **UniFFI bindings**: Automatic FFI generation for mobile platforms

## Quick Start

Add mobench-sdk to your project:

```toml
[dependencies]
mobench-sdk = "0.1"
```

Mark functions to benchmark:

```rust
use mobench_sdk::benchmark;

#[benchmark]
fn fibonacci() {
    let result = fib(30);
    std::hint::black_box(result);
}

#[benchmark]
fn json_parsing() {
    let data = serde_json::from_str::<MyStruct>(JSON_DATA).unwrap();
    std::hint::black_box(data);
}

fn fib(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fib(n - 1) + fib(n - 2),
    }
}
```

Run programmatically:

```rust
use mobench_sdk::{run_benchmark, BenchSpec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = BenchSpec::new("fibonacci", 100, 10)?;
    let report = run_benchmark(spec)?;

    println!("Mean: {} ns", report.mean_ns());
    println!("Median: {} ns", report.median_ns());
    println!("Std dev: {} ns", report.stddev_ns());

    Ok(())
}
```

## Project Setup

### 1. Initialize Mobile Benchmarking

Use the [mobench CLI](https://crates.io/crates/mobench) to scaffold your project:

```bash
cargo install mobench
cargo mobench init --target android  # or ios, or both
```

This creates:
- `bench-mobile/` - FFI wrapper crate
- `android/` or `ios/` - Mobile app projects
- `bench-sdk.toml` - Configuration file

### 2. Add Benchmarks

```rust
use mobench_sdk::benchmark;

#[benchmark]
fn my_benchmark() {
    // Your code here
}
```

### 3. Build for Mobile

```bash
cargo mobench build --target android
```

### 4. Run on Devices

Local device workflow (builds artifacts and writes the run spec; launch the app manually):
```bash
cargo mobench run --target android --function my_benchmark
```

BrowserStack:
```bash
export BROWSERSTACK_USERNAME=your_username
export BROWSERSTACK_ACCESS_KEY=your_key

cargo mobench run --target android --function my_benchmark --devices "Pixel 7-13"
```

## API Documentation

### Core Functions

#### `run_benchmark`

Run a registered benchmark by name:

```rust
use mobench_sdk::{run_benchmark, BenchSpec};

let spec = BenchSpec::new("my_function", 50, 5)?;
let report = run_benchmark(spec)?;
```

#### `BenchmarkBuilder`

Fluent API for building and running benchmarks:

```rust
use mobench_sdk::BenchmarkBuilder;

let report = BenchmarkBuilder::new("my_function")
    .iterations(100)
    .warmup(10)
    .run()?;
```

### Types

#### `BenchSpec`

Benchmark specification:

```rust
pub struct BenchSpec {
    pub name: String,
    pub iterations: u32,
    pub warmup: u32,
}
```

#### `RunnerReport`

Benchmark results with statistical analysis:

```rust
pub struct RunnerReport {
    pub spec: BenchSpec,
    pub samples: Vec<BenchSample>,
}

impl RunnerReport {
    pub fn mean_ns(&self) -> f64;
    pub fn median_ns(&self) -> u64;
    pub fn min_ns(&self) -> u64;
    pub fn max_ns(&self) -> u64;
    pub fn stddev_ns(&self) -> f64;
    pub fn percentile(&self, p: f64) -> u64;
}
```

### Build API

#### Generate Mobile Projects

```rust
use mobench_sdk::{InitConfig, Target, generate_project};

let config = InitConfig {
    project_name: "my-benchmarks".to_string(),
    output_dir: PathBuf::from("./bench-output"),
    target: Target::Both,  // Android + iOS
    generate_examples: true,
};

let project_path = generate_project(&config)?;
```

#### Build for Android

```rust
use mobench_sdk::AndroidBuilder;

let builder = AndroidBuilder::new(PathBuf::from("."), "debug")?;
let apk = builder.build_apk()?;
println!("Built APK: {:?}", apk);
```

#### Build for iOS

```rust
use mobench_sdk::IosBuilder;

let builder = IosBuilder::new(PathBuf::from("."), "release")?;
let xcframework = builder.build_xcframework()?;
println!("Built xcframework: {:?}", xcframework);
```

## Examples

### Crypto Benchmarks

```rust
use mobench_sdk::benchmark;
use sha2::{Sha256, Digest};
use aes::Aes256;

#[benchmark]
fn sha256_1kb() {
    let data = vec![0u8; 1024];
    let hash = Sha256::digest(&data);
    std::hint::black_box(hash);
}

#[benchmark]
fn aes256_encrypt() {
    let key = [0u8; 32];
    let cipher = Aes256::new(&key.into());
    // ... encryption code
    std::hint::black_box(cipher);
}
```

### JSON Parsing Benchmarks

```rust
use mobench_sdk::benchmark;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct User {
    name: String,
    email: String,
    age: u32,
}

const JSON_DATA: &str = r#"{"name":"Alice","email":"alice@example.com","age":30}"#;

#[benchmark]
fn parse_json_small() {
    let user: User = serde_json::from_str(JSON_DATA).unwrap();
    std::hint::black_box(user);
}

#[benchmark]
fn serialize_json_small() {
    let user = User {
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        age: 30,
    };
    let json = serde_json::to_string(&user).unwrap();
    std::hint::black_box(json);
}
```

### Data Structure Benchmarks

```rust
use mobench_sdk::benchmark;
use std::collections::{HashMap, BTreeMap};

#[benchmark]
fn hashmap_insert_1000() {
    let mut map = HashMap::new();
    for i in 0..1000 {
        map.insert(i, i * 2);
    }
    std::hint::black_box(map);
}

#[benchmark]
fn btreemap_insert_1000() {
    let mut map = BTreeMap::new();
    for i in 0..1000 {
        map.insert(i, i * 2);
    }
    std::hint::black_box(map);
}
```

## Architecture

### Workflow

1. **Development**: Write benchmarks with `#[benchmark]`
2. **Compilation**: Benchmarks registered at compile time via `inventory`
3. **FFI Generation**: UniFFI creates type-safe Kotlin/Swift bindings
4. **Mobile Build**: Cross-compile to mobile platforms
5. **Execution**: Run on real devices or emulators
6. **Analysis**: Collect and analyze timing data

### Components

```
┌─────────────────────────────────────────┐
│ Your Rust Code + #[benchmark]           │
└──────────────┬──────────────────────────┘
               │
               ↓
┌─────────────────────────────────────────┐
│ mobench-sdk (Registry + Build Tools)    │
└──────────────┬──────────────────────────┘
               │
               ↓
┌─────────────────────────────────────────┐
│ UniFFI (FFI Bindings Generation)        │
└──────────────┬──────────────────────────┘
               │
       ┌───────┴───────┐
       ↓               ↓
┌─────────────┐ ┌─────────────┐
│ Android APK │ │  iOS IPA    │
└──────┬──────┘ └──────┬──────┘
       │               │
       └───────┬───────┘
               ↓
    ┌──────────────────────┐
    │ Real Mobile Devices  │
    │ (BrowserStack/Local) │
    └──────────────────────┘
```

## Configuration

### `bench-sdk.toml`

```toml
[project]
name = "my-benchmarks"
target = "both"  # android, ios, or both

[build]
profile = "release"  # or "debug"

[browserstack]
username = "${BROWSERSTACK_USERNAME}"
access_key = "${BROWSERSTACK_ACCESS_KEY}"
project = "my-project-benchmarks"

[[devices]]
name = "Pixel 7"
os = "android"
os_version = "13.0"

[[devices]]
name = "iPhone 14"
os = "ios"
os_version = "16"
```

## Requirements

### For Android

- Android NDK
- `cargo-ndk`: `cargo install cargo-ndk`
- Android SDK (API level 24+)

### For iOS

- macOS with Xcode
- Rust targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`
- `xcodegen`: `brew install xcodegen`

## Part of mobench

This is the core SDK of the mobench ecosystem:

- **[mobench](https://crates.io/crates/mobench)** - CLI tool (recommended for most users)
- **[mobench-sdk](https://crates.io/crates/mobench-sdk)** - This crate (SDK library)
- **[mobench-macros](https://crates.io/crates/mobench-macros)** - Proc macros
- **[mobench-runner](https://crates.io/crates/mobench-runner)** - Timing harness

## See Also

- [CLI Documentation](https://crates.io/crates/mobench) for command-line usage
- [UniFFI Documentation](https://mozilla.github.io/uniffi-rs/) for FFI details
- [BrowserStack App Automate](https://www.browserstack.com/app-automate) for device testing

## License

Licensed under the MIT License. See [LICENSE.md](../../LICENSE.md) for details.

Copyright (c) 2026 World Foundation
