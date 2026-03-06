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
- `request_command` (`--request-command`, fallback to argv)
- `mobench_ref` (`--mobench-ref`, `MOBENCH_REF`, `GITHUB_SHA`, `GITHUB_REF`)
- `mobench_version` (derived from package version)
- `trigger_source` (optional, for example `label`, `pr_comment`, `workflow_dispatch`, `check_rerequest`)
- `dispatch_id` (optional UUID used to correlate GitHub App dispatch with later ingest)

`request_command` may come from a GitHub App owned PR comment command such as `/mobench platform=both iterations=30 warmup=5 device_profile=low-spec`.

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
  - `request_command`
  - `mobench_ref` (optional)
  - `mobench_version`
  - `trigger_source` (optional)
  - `dispatch_id` (optional)
- `ci.outputs` object with:
  - `summary_json`
  - `summary_md`
  - `results_csv`

Canonical history ingest is defined separately by `docs/schemas/history-manifest-v1.schema.json`. That bundle is uploaded as `mobench-history-v1` and is the durable bridge between GitHub App dispatch, workflow completion, and server-side ingest.

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
