# Testing Patterns

**Analysis Date:** 2026-01-21

## Test Framework

**Runner:**
- Cargo built-in test runner (no external test framework)
- Run all tests: `cargo test --all`
- Run specific crate: `cargo test -p mobench-sdk`
- Watch mode: Not configured (no test-watching framework)
- Coverage: Not configured (no coverage tool integration)

**Assertion Library:**
- Standard Rust `assert!()`, `assert_eq!()`, `assert_ne!()`
- Pattern matching: `assert!(matches!(result, Err(TimingError::NoIterations)))`

**Run Commands:**
```bash
# Run all tests in workspace
cargo test --all

# Run tests for specific crate
cargo test -p mobench-sdk
cargo test -p mobench

# Run with output captured
cargo test -- --nocapture

# List all tests without running
cargo test --all -- --list
```

## Test File Organization

**Location:**
- Co-located with source code, not in separate `tests/` directory
- Test modules at end of each source file
- Conditional compilation: `#[cfg(test)]` wrapping test modules

**Naming:**
- Test function prefix: `test_` or descriptive name
- Crate test modules: Named `tests` consistently
- Test utilities: Defined within test module using helper functions

**Structure:**

```
src/
├── timing.rs          # Source + inline #[cfg(test)] mod tests
├── registry.rs        # Source + inline #[cfg(test)] mod tests
├── builders/
│   ├── android.rs     # Source + inline #[cfg(test)] mod tests
│   └── ios.rs         # Source + inline #[cfg(test)] mod tests
```

## Test Structure

**Suite Organization:**

From `crates/mobench-sdk/src/timing.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_benchmark() {
        let spec = BenchSpec::new("noop", 3, 1).unwrap();
        let report = run_closure(spec, || Ok(())).unwrap();

        assert_eq!(report.samples.len(), 3);
        let non_zero = report.samples.iter().filter(|s| s.duration_ns > 0).count();
        assert!(non_zero >= 1);
    }

    #[test]
    fn rejects_zero_iterations() {
        let result = BenchSpec::new("test", 0, 10);
        assert!(matches!(result, Err(TimingError::NoIterations)));
    }
}
```

**Patterns:**
- `use super::*;` to import all items from parent module
- Setup: Direct instantiation in each test (no shared fixtures)
- Execution: Call the function under test
- Assertion: Use `assert!()`, `assert_eq!()`, or pattern matching
- No teardown required (Rust handles memory cleanup)

## Mocking

**Framework:** No external mocking library used

**Patterns:**
- Manual test doubles and stubs
- Trait-based design for dependency injection in production code
- Example from `crates/mobench-sdk/src/builders/android.rs` - builder pattern with customizable output:
```rust
#[test]
fn test_android_builder_custom_output_dir() {
    let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile")
        .output_dir("/custom/output");
    assert_eq!(builder.output_dir, PathBuf::from("/custom/output"));
}
```

**What to Mock:**
- Not used in this codebase; tests use concrete types with customizable configuration
- Builder pattern preferred over dependency injection for tests

**What NOT to Mock:**
- Core timing functionality (tested with actual measurements)
- Type conversion logic (uses direct instantiation)
- Registry operations (tested with actual inventory collection)

## Fixtures and Factories

**Test Data:**
No factory pattern or fixture framework found. Tests create minimal setup inline:

```rust
#[test]
fn test_parse_output_metadata_unsigned() {
    let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile");
    let metadata = r#"{"version":3,"artifactType":{"type":"APK",...}}"#;
    let result = builder.parse_output_metadata(metadata);
    assert_eq!(result, Some("app-release-unsigned.apk".to_string()));
}
```

**Location:**
- Test-specific data defined inline or as constants in test module
- Constants: `CHECKSUM_INPUT` in `crates/sample-fns/src/lib.rs`

**Pattern:**
```rust
const CHECKSUM_INPUT: [u8; 1024] = [1; 1024];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum() {
        // Use CHECKSUM_INPUT directly
    }
}
```

## Coverage

**Requirements:** Not enforced

**View Coverage:** No coverage command configured

**Observed Coverage:**
- `timing` module: 7 unit tests covering core functionality
- `registry` module: 3 tests covering discovery and lookup
- `builders` (android): 6+ tests covering builder construction and metadata parsing
- `builders` (ios): Tests present but not extensively reviewed

## Test Types

**Unit Tests:**
- Scope: Individual functions and methods
- Approach: Direct instantiation, no external dependencies
- Examples:
  - `test_rejects_zero_iterations()` - validates error handling
  - `test_allows_zero_warmup()` - boundary condition
  - `test_serializes_to_json()` - serialization correctness

**Integration Tests:**
- Scope: Multiple components working together
- Approach: Full builder workflow with real file operations
- Examples:
  - Builder type construction and method chaining
  - Metadata parsing from JSON
  - Benchmark specification validation

**E2E Tests:**
- Framework: None (not applicable; this is a library)
- Device testing: Handled by mobile app frameworks (XCUITest, Espresso)
- Host testing: No end-to-end test suite observed

## Common Patterns

**Async Testing:**
No async tests found (Rust blocking I/O used throughout)

**Error Testing:**

```rust
#[test]
fn rejects_zero_iterations() {
    let result = BenchSpec::new("test", 0, 10);
    assert!(matches!(result, Err(TimingError::NoIterations)));
}
```

Pattern: Use `matches!()` for pattern matching on Result/Option types

**Builder Method Testing:**

```rust
#[test]
fn test_android_builder_verbose() {
    let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile")
        .verbose(true);
    assert!(builder.verbose);
}
```

**Successful Operation Testing:**

```rust
#[test]
fn runs_benchmark() {
    let spec = BenchSpec::new("noop", 3, 1).unwrap();
    let report = run_closure(spec, || Ok(())).unwrap();
    assert_eq!(report.samples.len(), 3);
}
```

**JSON Serialization Testing:**

```rust
#[test]
fn serializes_to_json() {
    let spec = BenchSpec::new("test", 10, 2).unwrap();
    let report = run_closure(spec, || Ok(())).unwrap();

    let json = serde_json::to_string(&report).unwrap();
    let restored: BenchReport = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.spec.name, "test");
    assert_eq!(restored.samples.len(), 10);
}
```

**File Path Testing:**

```rust
#[test]
fn test_android_builder_creation() {
    let builder = AndroidBuilder::new("/tmp/test-project", "test-bench-mobile");
    assert_eq!(
        builder.output_dir,
        PathBuf::from("/tmp/test-project/target/mobench")
    );
}
```

## Test Characteristics

**Independence:**
- Each test creates its own test data and configuration
- No shared state between tests
- Tests can run in any order

**Determinism:**
- All tests are deterministic (no randomness)
- Timing tests accept variance (check `non_zero >= 1`, not exact values)

**Readability:**
- Clear test names describing what is tested
- Simple, direct assertion patterns
- No test setup boilerplate

**Performance:**
- Tests run quickly (unit tests < 100ms each)
- No external network calls
- No file I/O except in builder tests (using /tmp paths)

---

*Testing analysis: 2026-01-21*
