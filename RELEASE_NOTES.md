# Release Notes

`mobench`, `mobench-sdk`, and `mobench-macros` were published rapidly during
bring-up. Only the current release line should be treated as supported. Every
earlier crates.io publish is retained here for auditability, but unless noted
otherwise it should be treated as a test build and should not be used for new
integrations.

Crates.io release history:
- [mobench](https://crates.io/crates/mobench)
- [mobench-sdk](https://crates.io/crates/mobench-sdk)
- [mobench-macros](https://crates.io/crates/mobench-macros)

## Support Policy

- `v0.1.26` is the current supported release.
- Every earlier published version is a historical test build and should not be
  used.
- Yanked versions are explicitly called out below.

## Published Version History

| Version | Published | Published crates | Status |
|---------|-----------|------------------|--------|
| `v0.1.26` | 2026-03-28 | `mobench 0.1.26`, `mobench-sdk 0.1.26`, `mobench-macros 0.1.26` | Current supported release |
| `v0.1.25` | 2026-03-26 | `mobench 0.1.25`, `mobench-sdk 0.1.25`, `mobench-macros 0.1.25` | Test build. Do not use. |
| `v0.1.24` | 2026-03-26 | `mobench 0.1.24`, `mobench-sdk 0.1.24`, `mobench-macros 0.1.24` | Test build. Do not use. |
| `v0.1.23` | 2026-03-26 | `mobench 0.1.23`, `mobench-sdk 0.1.23`, `mobench-macros 0.1.23` | Test build. Do not use. |
| `v0.1.22` | 2026-03-25 | `mobench 0.1.22`, `mobench-sdk 0.1.22`, `mobench-macros 0.1.22` | Test build. Do not use. |
| `v0.1.21` | 2026-03-24 | `mobench 0.1.21`, `mobench-sdk 0.1.21`, `mobench-macros 0.1.21` | Test build. Do not use. |
| `v0.1.20` | 2026-03-24 | `mobench 0.1.20`, `mobench-sdk 0.1.20`, `mobench-macros 0.1.20` | Test build. Do not use. |
| `v0.1.19` | 2026-03-24 | `mobench 0.1.19`, `mobench-sdk 0.1.19`, `mobench-macros 0.1.19` | Test build. Do not use. |
| `v0.1.18` | 2026-03-24 | `mobench 0.1.18`, `mobench-sdk 0.1.18`, `mobench-macros 0.1.18` | Test build. Do not use. |
| `v0.1.17` | 2026-03-23 | `mobench 0.1.17`, `mobench-sdk 0.1.17`, `mobench-macros 0.1.17` | Test build. Do not use. |
| `v0.1.16` | 2026-03-18 | `mobench 0.1.16`, `mobench-sdk 0.1.16`, `mobench-macros 0.1.16` | Test build. Do not use. |
| `v0.1.15-patch-1` | 2026-03-12 | `mobench 0.1.15-patch-1` | Test build. Do not use. |
| `v0.1.15` | 2026-03-06 | `mobench 0.1.15`, `mobench-sdk 0.1.15`, `mobench-macros 0.1.15` | Test build. Do not use. |
| `v0.1.14` | 2026-02-16 | `mobench 0.1.14`, `mobench-sdk 0.1.14`, `mobench-macros 0.1.14` | Test build. Do not use. |
| `v0.1.13` | 2026-01-21 | `mobench 0.1.13`, `mobench-sdk 0.1.13`, `mobench-macros 0.1.13` | Test build. Do not use. |
| `v0.1.12` | 2026-01-20 | `mobench 0.1.12`, `mobench-sdk 0.1.12`, `mobench-macros 0.1.12` | Test build. Do not use. |
| `v0.1.11` | 2026-01-19 | `mobench 0.1.11`, `mobench-sdk 0.1.11`, `mobench-macros 0.1.11` | Test build. Do not use. |
| `v0.1.10` | 2026-01-19 | `mobench 0.1.10`, `mobench-sdk 0.1.10`, `mobench-macros 0.1.10` | Test build. Do not use. |
| `v0.1.9` | 2026-01-19 | `mobench 0.1.9`, `mobench-sdk 0.1.9`, `mobench-macros 0.1.9` | Test build. Do not use. |
| `v0.1.8` | 2026-01-19 | `mobench 0.1.8`, `mobench-sdk 0.1.8`, `mobench-macros 0.1.8` | Test build. Do not use. |
| `v0.1.7` | 2026-01-19 | `mobench 0.1.7`, `mobench-sdk 0.1.7`, `mobench-macros 0.1.7` | Test build. Do not use. |
| `v0.1.6` | 2026-01-16 | `mobench 0.1.6`, `mobench-sdk 0.1.6`, `mobench-macros 0.1.6` | Test build. Do not use. |
| `v0.1.5` | 2026-01-15 | `mobench 0.1.5`, `mobench-sdk 0.1.5` | Test build. Do not use. |
| `v0.1.4` | 2026-01-14 | `mobench 0.1.4`, `mobench-sdk 0.1.4`, `mobench-macros 0.1.4` | Test build. Do not use. |
| `v0.1.3` | 2026-01-14 | `mobench 0.1.3`, `mobench-sdk 0.1.3`, `mobench-macros 0.1.3` | Yanked test build. Do not use. |
| `v0.1.2` | 2026-01-14 | `mobench 0.1.2`, `mobench-sdk 0.1.2`, `mobench-macros 0.1.2` | Yanked test build. Do not use. |
| `v0.1.1` | 2026-01-13 | `mobench 0.1.1`, `mobench-sdk 0.1.1` | Yanked test build. Do not use. |
| `v0.1.0` | 2026-01-13 | `mobench 0.1.0`, `mobench-sdk 0.1.0`, `mobench-macros 0.1.0` | Yanked test build. Do not use. |

## v0.1.26

Status: current supported release.

- Published a synchronized `mobench`, `mobench-sdk`, and `mobench-macros`
  release so the registry dependency graph matches the current profiling and
  packaging APIs.
- Moved release history out of the root README into this standalone
  `RELEASE_NOTES.md` file and backfilled the published crate history.
- Cleaned up obsolete planning and contract docs from the repository-facing
  docs surface.
- Normalized crate README references so published crate pages link back to the
  correct GitHub-hosted schema and release-history sources.

## v0.1.25

Status: test build. Do not use.

- Clarified that profiling remains local-first in this release; BrowserStack
  native profiling is explicitly unsupported with actionable error text and a
  visible capability matrix.
- Split `profile run` into target resolution, capture planning, and capture
  execution seams so planned manifests no longer imply that native capture
  actually ran.
- Added device-selection inputs to `profile run` (`--device`, `--os-version`,
  `--profile`, `--device-matrix`) by reusing the existing deterministic
  device-resolution flow.
- Added real local iOS native capture via simulator-host `sample`, with
  `sample.txt`, `stacks.folded`, `native-report.txt`, and `flamegraph.html`
  written into the normalized profile session layout.
- Added regression coverage for profile help text, BrowserStack unsupported
  execution, dry-run planning semantics, and direct device target resolution.
- Added `cargo mobench profile run|summarize` commands for a normalized local
  profiling session contract across Android and iOS.
- Added the interactive dual-view flamegraph viewer plus full and focused SVG
  artifacts for local native profile runs.
- Profile sessions now write run-scoped artifacts under
  `target/mobench/profile/<run-id>/` and refresh top-level latest-session
  `profile.json` and `summary.md` convenience files.
- Profile manifests now preserve the selected provider and requested output
  format, and the CLI rejects unsupported format/backend combinations
  explicitly instead of silently planning the wrong artifacts.
- Updated the profiling smoke-test docs to use working
  `cargo run -p mobench --bin mobench -- ...` invocations from the repo root.
- Stabilized the SDK timing test suite by removing a timer-resolution
  assumption from the noop benchmark test.

## v0.1.24

Status: test build. Do not use.

- Switched BrowserStack device discovery to the unified
  `app-automate/devices.json` inventory for Android, iOS, and combined device
  listing.
- Filtered unified BrowserStack inventory results locally by OS so Espresso
  resolution stays Android-only and XCUITest resolution stays iOS-only.
- Added regression coverage for mixed Android+iOS BrowserStack inventories used
  by device-resolution commands.

## v0.1.23

Status: test build. Do not use.

- Added Sina-style per-function device comparison plots to local summaries:
  `cargo mobench ci run --plots <auto|off|require>` and
  `cargo mobench report summarize --plots <auto|off|require>`.
- Rendered one SVG plot per benchmark function in the `Device Comparison Plots`
  section of local markdown summaries.
- Switched summary resource reporting to `cpu_total_ms` and `peak_memory_kb`,
  and preserved BrowserStack-derived peak memory while backfilling CPU from raw
  benchmark results.
- Enabled BrowserStack app profiling on Android and iOS runs, including App
  Profiling v2 parsing for iOS peak-memory enrichment.
- Added baseline artifact download in the reusable CI workflow so
  `ci check-run` can compare PR results against the latest successful
  default-branch run.

## v0.1.22

Status: test build. Do not use.

- Fixed BrowserStack result fetching so `cargo mobench ci run --fetch` falls
  back to downloaded session artifacts when live device logs do not expose
  benchmark JSON.
- Unified benchmark extraction across live logs, `bench-report.json`, iOS
  marker logs, and Android `BENCH_JSON` logs so per-function CI summaries are
  written with populated benchmark data.
- Fixed merged CI output generation to preserve every function under each
  target and emit a top-level `summary` for single-target runs.
- Fixed `cargo-mobench ci summarize` to read merged `{targets, ci}` outputs,
  recurse through nested target and function result directories, and fall back
  to raw `bench-report.json` when needed.

## v0.1.21

Status: test build. Do not use.

- Added a shared config-first project resolver across `build`, `run`,
  packaging, `list`, and `verify`.
- Added `--project-root` and `--crate-path` parity across the main CLI commands
  for custom repository layouts.
- `build --progress` now respects `mobench.toml` instead of assuming
  `bench-mobile`.
- Dotenv loading now follows the resolved project root and config path.
- `list` now discovers benchmarks from configured external crates instead of
  only legacy sample layouts.
- `verify --smoke-test` now reports external-crate smoke tests as unsupported
  instead of failing with an empty benchmark list.

## v0.1.20

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.19

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.18

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.17

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.16

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.15-patch-1

Status: test build. Do not use.

- Published only for `mobench`; there was no matching `mobench-sdk` release for
  this version tag.
- No supported release notes were maintained for this build.

## v0.1.15

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.14

Status: test build. Do not use.

- Added CI contract-oriented commands and workflows:
  `cargo mobench ci run`, `cargo mobench config validate`,
  `cargo mobench devices resolve`, `cargo mobench fixture init|build|verify|cache-key`,
  and `cargo mobench report summarize|github`.
- Standardized CI outputs under `target/mobench/ci/` with schema-backed
  metadata.
- Added baseline comparison source support (`path|url|artifact:<path>`) and
  regression labels.
- Improved local action safety for workflow input handling and sticky PR comment
  publishing.
- Fixed iOS CI target setup (`x86_64-apple-ios`) and preserved CI outputs on
  regression exit.

## v0.1.13

Status: test build. Do not use.

- Added setup and teardown support to `#[benchmark]` via `setup`, `teardown`,
  and `per_iteration` attributes.
- Added `cargo mobench check`, `cargo mobench verify`, `cargo mobench summary`,
  and `cargo mobench devices`.
- Added `--progress` output for `build` and `run`.
- Consolidated `mobench-runner` into `mobench-sdk`.
- Improved SDK compile-time validation and benchmark debug helpers.
- Improved BrowserStack credential, upload, and device-matching UX.
- Fixed the iOS XCUITest BrowserStack `only-testing` filter to use
  `testLaunchAndCaptureBenchmarkReport`.

## v0.1.12

Status: test build. Do not use.

- Fixed iOS XCUITest BrowserStack detection by adding `Info.plist` to the
  UITests target template.
- Increased BrowserStack post-benchmark delay to improve video capture of
  benchmark results.
- Added visible “Running benchmarks...” feedback during iOS benchmark runs.
- Synchronized top-level iOS and Android templates with the SDK-embedded
  templates.

## v0.1.11

Status: test build. Do not use.

- Initial public release with `--release` flag support.
- Added `package-xcuitest` for iOS BrowserStack testing.
- Updated mobile timing display and documentation.

## v0.1.10

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.9

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.8

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.7

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.6

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.5

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.4

Status: test build. Do not use.

- Published to crates.io as part of the pre-release validation cycle.
- No supported release notes were maintained for this build.

## v0.1.3

Status: yanked test build. Do not use.

- Yanked from crates.io.
- No supported release notes were maintained for this build.

## v0.1.2

Status: yanked test build. Do not use.

- Yanked from crates.io.
- No supported release notes were maintained for this build.

## v0.1.1

Status: yanked test build. Do not use.

- Yanked from crates.io.
- No supported release notes were maintained for this build.

## v0.1.0

Status: yanked test build. Do not use.

- Yanked from crates.io.
- No supported release notes were maintained for this build.
