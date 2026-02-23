# Coding Conventions

**Analysis Date:** 2026-01-21

## Naming Patterns

**Files:**
- Rust source files: `lowercase_with_underscores.rs`
- Module structure: `mod.rs` for module files, `filename.rs` for single-item modules
- Examples: `crates/mobench-sdk/src/timing.rs`, `crates/mobench-sdk/src/builders/android.rs`

**Functions:**
- Public functions and private helpers: `snake_case`
- Builder methods: `snake_case`, returning `Self` for chaining
- Test functions: `test_<purpose>()` (e.g., `test_rejects_zero_iterations()`)
- Examples: `run_closure()`, `discover_benchmarks()`, `find_benchmark()`, `get_cargo_target_dir()`

**Variables:**
- Local variables and parameters: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- Field names in structs: `snake_case`
- Example: `CHECKSUM_INPUT` for const, `project_root` for variables

**Types:**
- Struct and enum names: `PascalCase`
- Type parameters: Single uppercase letters or `PascalCase` (e.g., `T`, `F`)
- Examples: `BenchSpec`, `BenchSample`, `BenchError`, `AndroidBuilder`, `IosBuilder`, `BenchFunction`

## Code Style

**Formatting:**
- Edition: Rust 2021 (specified in workspace `Cargo.toml` as `edition = "2024"`)
- Indentation: 4 spaces (Rust standard)
- Line length: No enforced limit observed; examples use 80-100 character average
- Trailing commas: Used in multi-line collections and match expressions

**Linting:**
- No explicit linter config found (no `.clippy.toml` or `rust-clippy.toml`)
- Implicitly follows Rust conventions through code style
- Error types use `#[error("...")]` from `thiserror` crate for custom messages

## Import Organization

**Order:**
1. Crate imports (`use crate::...`)
2. Standard library imports (`use std::...`)
3. External crate imports (`use external_crate::...`)
4. Re-exports and module declarations (`pub use ...`, `mod ...`)

**Pattern Examples:**

From `crates/mobench-sdk/src/timing.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use thiserror::Error;
```

From `crates/mobench-sdk/src/builders/android.rs`:
```rust
use crate::types::{BenchError, BuildConfig, BuildProfile, BuildResult, Target};
use super::common::{get_cargo_target_dir, host_lib_path, run_command, validate_project_root};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
```

**Path Aliases:**
- No path aliases configured (no `[paths]` in `Cargo.toml`)
- Uses standard module hierarchy

## Error Handling

**Patterns:**
- `Result<T, E>` return types for fallible operations
- Custom error enum `BenchError` with `#[derive(Debug, thiserror::Error)]`
- Error variants use `#[error("message")]` for display formatting
- `#[from]` for automatic conversion from underlying error types
- `Ok(value)?` syntax for error propagation
- `anyhow::Result` and `anyhow::Context` in CLI code (`mobench/src/lib.rs`)

**Error Handling in Core SDK:**
```rust
#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    #[error("benchmark runner error: {0}")]
    Runner(#[from] crate::timing::TimingError),

    #[error("unknown benchmark function: {0}...")]
    UnknownFunction(String),

    #[error("I/O error: {0}. Check file paths and permissions")]
    Io(#[from] std::io::Error),
}
```

**Validation Pattern:**
- Early returns for validation failures
- Detailed error messages with fix suggestions
- Example from `crates/mobench-sdk/src/builders/common.rs`:
```rust
if !project_root.exists() {
    return Err(BenchError::Build(format!(
        "Project root does not exist: {}\n\n\
         Ensure you are running from the correct directory or specify --project-root.",
        project_root.display()
    )));
}
```

## Logging

**Framework:** `eprintln!()` macro for error output to stderr

**Patterns:**
- No centralized logging framework used
- Simple stderr output for errors: `eprintln!("{err:#}")`
- Example from `crates/mobench/src/main.rs`:
```rust
fn main() {
    if let Err(err) = mobench::run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}
```

**Verbose Output:**
- Controlled via `--verbose` or `-v` CLI flag
- Verbose mode prints full command execution details

## Comments

**When to Comment:**
- Complex algorithms or non-obvious logic
- FFI boundary details and platform-specific behavior
- Workarounds for tooling limitations
- Not required for simple, self-documenting code

**JSDoc/TSDoc:**
- Uses Rust documentation comments (`//!` for modules, `///` for items)
- Triple-slash `///` comments appear before all public items
- Module-level documentation with `//!` at file start
- Examples provided in doc comments for complex APIs

**Documentation Comment Pattern:**
```rust
/// Marks a function as a benchmark for mobile execution.
///
/// This attribute macro registers the function in the global benchmark registry,
/// making it discoverable and executable by the mobench runtime.
///
/// # Usage
///
/// ```ignore
/// #[benchmark]
/// fn fibonacci_bench() { ... }
/// ```
///
/// # Requirements
///
/// The annotated function must:
/// - Take no parameters
/// - Return `()` (unit type)
```

## Function Design

**Size:**
- Range from 5-60 lines for typical functions
- Simple builders and utilities: 3-20 lines
- Complex builders with multiple validation steps: 100-200 lines
- Core timing function `run_closure()`: 25 lines

**Parameters:**
- Builder methods accept `impl Into<T>` for flexibility
- Generic closures with trait bounds
- Result returns for fallible operations
- Example: `pub fn new(name: impl Into<String>, iterations: u32, warmup: u32) -> Result<Self, TimingError>`

**Return Values:**
- `Result<T, E>` for fallible operations
- `Option<T>` for optional lookups
- `Self` for builder chaining
- Direct values for infallible operations

## Module Design

**Exports:**
- Public types and functions: `pub` keyword explicitly
- Conditional exports with `#[cfg(feature = "...")]`
- Re-exports for convenience at crate root level
- Example from `crates/mobench-sdk/src/lib.rs`:
```rust
#[cfg(feature = "full")]
pub use registry::{BenchFunction, discover_benchmarks, find_benchmark, list_benchmark_names};
pub use timing::{run_closure, TimingError};
```

**Barrel Files:**
- `mod.rs` files organize module exports
- Example: `crates/mobench-sdk/src/builders/mod.rs` exports all builder types
- Re-export pattern: `pub use self::android::AndroidBuilder; pub use self::ios::IosBuilder;`

**Feature-Gated Modules:**
- Full feature includes: `builders`, `codegen`, `registry`, `runner`, macros
- Runner-only feature: Minimal `timing` module only
- Conditional compilation: `#[cfg(feature = "full")]`

## Derive Macros

**Common Patterns:**
- `#[derive(Debug, Clone)]` for shared types
- `#[derive(Serialize, Deserialize)]` for serializable types
- `#[derive(Error)]` from `thiserror` for error enums
- `#[derive(uniffi::Record)]` for FFI-exposed types (in `sample-fns`)

**Example:**
```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchSpec {
    pub name: String,
    pub iterations: u32,
    pub warmup: u32,
}
```

## Builder Pattern

**Implementation:**
- Builder struct with private fields
- `new()` constructor with required parameters
- Chainable builder methods returning `Self`
- Terminal method (e.g., `build()`) that performs action
- Example: `AndroidBuilder::new(...).verbose(true).output_dir(...).build()`

**Defaults:**
- Sensible defaults in constructor
- AndroidBuilder: `verbose: false`, `output_dir: "target/mobench"`, `dry_run: false`

---

*Convention analysis: 2026-01-21*
