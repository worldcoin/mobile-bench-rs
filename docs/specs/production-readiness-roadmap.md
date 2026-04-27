# Production Readiness Roadmap

Updated: 2026-04-27

## Purpose

mobench v0.1.36 is the current published supported release. This roadmap records
the quality, maintainability, documentation, and launch-readiness work completed
for that release, plus follow-up hardening after broader public promotion.

## Launch Gates

### Gate 1: Trust The Crate

Goal: make `mobench`, `mobench-sdk`, and `mobench-macros` feel like dependable
production Rust crates.

Audience: library adopters, CLI users, maintainers.

Checklist:
- [x] Audit public APIs exported from `mobench-sdk`.
- [x] Document semver and stability boundaries.
- [x] Review feature flags, especially `full` and `runner-only`.
- [x] Replace reusable-library `anyhow` surfaces with typed errors where appropriate.
- [x] Improve docs.rs module docs and examples.
- [x] Add compile-tested doc examples for core SDK usage.
- [x] Add or refine minimal library adopter examples.
- [x] Audit crate metadata, badges, readmes, categories, and keywords.
- [x] Document MSRV policy.
- [x] Enforce rustfmt, clippy, and rustdoc warnings in CI.
- [x] Run staged `cargo publish --dry-run` checks for all published crates
  during release.

Verification signals:
- `cargo test --workspace` passes.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps` passes.
- docs.rs pages render cleanly for all published crates.
- Examples build from a clean checkout.

### Gate 2: Trust The Outputs

Goal: make benchmark, CI, and profiling outputs dependable enough for users to
automate around.

Audience: CLI users, CI adopters, maintainers.

Checklist:
- [x] Add schema validation tests for `summary.json`.
- [x] Add schema validation tests for the CI contract.
- [x] Add golden fixture tests for Markdown summaries.
- [x] Add golden fixture tests for CSV outputs.
- [x] Add golden fixture tests for plots where practical.
- [x] Add golden fixture tests for profile summaries.
- [x] Add CLI snapshot tests for high-value command output.
- [x] Add BrowserStack response normalization regression tests.
- [x] Test resource metric contracts: `cpu_total_ms`, `cpu_median_ms`, `peak_memory_kb`.
- [x] Test baseline and regression comparison behavior.
- [x] Test profile manifest sections: `native_capture`, `semantic_profile`, `capture_metadata`.
- [x] Test Android and iOS template generation invariants.
- [x] Test setup, teardown, and per-iteration macro behavior.
- [x] Keep fixture verification wired into CI.
- [x] Label tests that require Android, iOS, BrowserStack, or profiling tools separately from pure host tests.

Verification signals:
- Host-only tests run without mobile toolchains.
- Mobile/tooling tests are opt-in or clearly gated.
- Existing example fixtures validate against schemas.
- Summary, CSV, plot, and profile contracts are covered by regression tests.

### Gate 3: Trust The Experience

Goal: make mobench easy to adopt, debug, explain, and promote publicly.

Audience: CLI users, library adopters, public launch readers.

Checklist:
- [x] Add structured tracing/logging to CLI flows.
- [x] Add progress spans for build, package, upload, poll, fetch, summarize, and profile steps.
- [x] Improve human-readable diagnostics with likely fixes.
- [x] Expand `doctor` coverage for Android, iOS, BrowserStack, and profile prerequisites.
- [x] Add examples for minimal benchmark usage.
- [x] Add examples for setup/teardown benchmarks.
- [x] Add examples for FFI/custom type benchmarks.
- [x] Add examples for CI-only benchmark workflows.
- [x] Add examples for profiling workflows.
- [x] Add examples for programmatic SDK usage.
- [x] Add README graphics for crate architecture.
- [x] Add README graphics for benchmark execution lifecycle.
- [x] Add README graphics for BrowserStack CI lifecycle.
- [x] Add README graphics for local profiling artifact lifecycle.
- [x] Add README graphics for SDK versus CLI responsibility boundaries.
- [x] Write concise launch copy explaining why mobench exists.
- [x] Keep release notes and migration notes current.

Verification signals:
- A new user can follow the README from install to first result.
- `doctor` catches common setup issues before long-running commands fail.
- Public diagrams explain the codebase and mobench workflows without reading source.
- Launch assets are reusable for README, tweets, and landing page.

## Later Hardening

These items are valuable but should not block the first production-ready
announcement unless they expose real launch risk.

- [ ] Profile CLI hot paths.
- [ ] Improve APK/IPA build caching.
- [ ] Parallelize independent build, fetch, and report steps.
- [x] Add host benchmarks for parser, reporting, and profile code.
- [ ] Add fuzz or property tests for config and device matrix parsing.
- [x] Add public API compatibility checks with `cargo-semver-checks`.
- [x] Add dependency and license policy checks with `cargo-deny`.
- [ ] Consider narrower crate features to reduce dependency footprint.
- [ ] Add machine-readable trace/event output for CI debugging.
- [ ] Prepare landing-page-specific assets from README diagrams.

### Later Hardening Evaluation

Updated: 2026-04-27

This bucket should be sequenced by launch risk rather than implemented as one
large polish pass. The current repository has CI caching, fixture cache keys,
host benchmark smoke coverage, `cargo-semver-checks`, and a `cargo-deny`
policy. Fuzz/property tests and dedicated build-cache profiling remain open.

P0 after first public promotion:
- Completed: add `cargo-semver-checks` for `mobench-sdk` and `mobench`.
  `mobench-macros` is a proc-macro target, so `cargo-semver-checks` does not
  treat it as a normal library API surface; keep it covered by rustdoc, clippy,
  tests, and staged publish dry-runs.
- Completed: add `cargo-deny` with explicit license/advisory/source policy.
  Current policy denies vulnerable and yanked crates, restricts registries, and
  documents the direct Inferno `CDDL-1.0` license exception.
- Completed: add host benchmarks for parser, reporting, profile manifest
  rendering, and BrowserStack log extraction. These give a baseline before
  tuning or parallelizing anything.

P1 once adoption feedback identifies slow paths:
- Profile CLI hot paths with real examples: config resolution, template
  generation, report rendering, profile diff generation, and BrowserStack
  artifact normalization.
- Improve APK/IPA build caching where profiling shows repeated template,
  binding, or packaging work dominates local iteration time.
- Parallelize independent build, fetch, and report steps only after benchmarks
  make the current bottlenecks visible.
- Add fuzz or property tests for config and device matrix parsing after the
  host benchmark harness exists, so parser behavior and parser cost are both
  tracked.

P2 launch-asset and dependency-footprint follow-up:
- Prepare landing-page-specific assets from README diagrams once the landing
  page visual direction is chosen.
- Consider narrower crate features after `cargo-deny` and host benchmarks show
  which dependencies materially affect compile time, binary size, or policy
  surface.
- Add machine-readable trace/event output for CI debugging when there is a
  concrete downstream consumer for it; the current `MOBENCH_LOG` tracing is
  enough for near-term human debugging.

Recommended first hardening PR:
1. Run the new CI policy checks and host benchmark smoke on a fresh PR.
2. Review duplicate dependency warnings from `cargo-deny`, especially `toml`,
   `getrandom`, and Windows support crates.
3. Use the benchmark results to decide whether build caching or parallelization
   should come next.

## Recommended Order

1. Gate 1 crate hygiene, because this reduces adoption risk.
2. Gate 2 output contracts, because CI users need stable automation surfaces.
3. Gate 3 experience and launch assets, because promotion should point at a stable product.
4. Later hardening, prioritized by issues found during adoption.
