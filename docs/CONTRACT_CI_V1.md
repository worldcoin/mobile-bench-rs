# Mobench CI Contract v1

## Status

- Version: `v1`
- Stability: Frozen for v1 consumers
- Effective date: 2026-02-16

## Scope

This document defines the stable contract for `cargo mobench ci run`.

It covers:
- Input contract (CLI/environment inputs accepted by `ci run`)
- Output contract (`summary.json`, `summary.md`, `results.csv`)
- Error taxonomy categories used by CI-focused validation commands

## Default Repo Integration

Stateless GitHub Actions is the default v1 integration for this repository.

The default workflow model is:
- `compile-gate.yml` compiles the exact same-repo PR head SHA with `pull_request`.
- `mobile-bench-after-ci.yml` dispatches benchmarks after a successful compile gate when the PR still has the `bench` label.
- `mobile-bench-pr-auto.yml` handles bench-label events without checking out PR code and dispatches immediately only when the compile gate is already green for that exact SHA.
- `mobile-bench-pr-command.yml` handles trusted `/mobench ...` PR comments without checking out PR code and dispatches only after the compile gate is green for that exact SHA.
- `mobile-bench.yml` remains the single benchmark runner and delegates execution to `reusable-bench.yml`.

Security constraints for the default path:
- Stateless v1 is restricted to same-repo PRs. Fork PRs are ignored.
- Controller workflows are metadata-only and must never checkout PR code.
- `pull_request_target` is used only for metadata-only label handling and not for benchmark execution.

## Input Contract

### Required CLI inputs

- `--target <android|ios|both>`
- `--function <fully-qualified benchmark function>`

### Optional execution inputs

- Iteration controls: `--iterations`, `--warmup`
- Device selection: `--devices`, `--device-matrix`, `--device-tags`
- Runtime mode: `--local-only`, `--release`, `--fetch`
- iOS artifacts: `--ios-app`, `--ios-test-suite`
- Regression mode: `--baseline`, `--regression-threshold-pct`
- Output path: `--output-dir`

Behavior notes:
- In config-driven runs (`--config`), `--device-matrix` overrides the matrix path from config when both are provided.
- If `--baseline` resolves to the same file as the candidate output, mobench snapshots the previous baseline file before writing the candidate summary so regression comparison remains valid.

### Optional metadata inputs

Metadata can be provided via flags or CI environment discovery:

- `requested_by` (`--requested-by`, `MOBENCH_REQUESTED_BY`, `GITHUB_ACTOR`)
- `pr_number` (`--pr-number`, `MOBENCH_PR_NUMBER`, `PR_NUMBER`, `GITHUB_PR_NUMBER`, `GITHUB_PULL_REQUEST_NUMBER`, or parsed from `GITHUB_REF`)
- `base_ref` (`--base-ref`, `MOBENCH_BASE_REF`, `GITHUB_BASE_REF`)
- `request_command` (`--request-command`, fallback to argv)
- `mobench_ref` (`--mobench-ref`, `MOBENCH_REF`, `GITHUB_SHA`, `GITHUB_REF`)
- `mobench_version` (derived from package version)
- `trigger_source` (optional, for example `label`, `pr_comment`, `workflow_dispatch`, `check_rerequest`)
- `dispatch_id` (optional UUID used to correlate external dispatch state; empty in stateless GitHub Actions mode)

`request_command` may come from a trusted PR comment command such as `/mobench platform=both iterations=30 warmup=5 device_profile=low-spec`.

## Output Contract

Default directory: `target/mobench/ci/`

Required files:
- `summary.json`
- `summary.md`
- `results.csv`

`summary.json` MUST include:
- run summary data
- `ci.metadata` object with:
  - `requested_by`
  - `pr_number` (optional)
  - `base_ref` (optional)
  - `request_command`
  - `mobench_ref` (optional)
  - `mobench_version`
  - `trigger_source` (optional)
  - `dispatch_id` (optional)
- `ci.outputs` object with:
  - `summary_json`
  - `summary_md`
  - `results_csv`

Canonical history ingest is defined separately by `docs/schemas/history-manifest-v1.schema.json`. That bundle is uploaded as `mobench-history-v1` and is the durable bridge between workflow-owned benchmark execution, stateless baseline reuse, and any optional downstream history ingestion.

Machine-readable schema artifacts:
- `docs/schemas/summary-v1.schema.json`
- `docs/schemas/ci-contract-v1.schema.json`

## Error Taxonomy

The following categories are used for contract-aligned checks:
- `config_error`
- `preflight_error`
- `provider_error`
- `build_error`
- `benchmark_error`

Current command mapping:
- `cargo mobench doctor` and `cargo mobench config validate` emit category-aligned issues for config/preflight/provider failures.
- Build/benchmark failures remain surfaced by run/build/report commands and can be mapped by callers into the same taxonomy.

## Breaking-Change Policy (v1)

A change is breaking if it modifies or removes:
- required output filenames
- required metadata keys
- required schema fields/types

Breaking changes require:
1. New versioned contract docs/schema files.
2. Backward-compatibility note in release notes.
3. Migration guidance update.

## Compatibility Window

- `v1` outputs and metadata are maintained for at least one minor release window after any successor is introduced.
- Additive fields are allowed in `summary.json` as long as required keys remain stable.

## Non-goals

- Defining provider-specific BrowserStack API payload formats.
- Real-time dashboard protocols.
- Enforcing thresholds by default in v1 reporting.
