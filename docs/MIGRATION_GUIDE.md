# Mobench CI Migration Guide

This guide migrates custom BrowserStack benchmark CI flows to the standardized `mobench` v1 contract.

## Goals

- One-command orchestration via `cargo mobench ci run`
- Stable contract outputs: `summary.json`, `summary.md`, `results.csv`
- Optional local plot artifacts under `plots/*.svg`
- Optional sticky PR comments and deterministic matrix resolution

## Old -> New Mapping

| Legacy pattern | New command/workflow |
| --- | --- |
| Custom prereq scripts | `cargo mobench doctor --target both` |
| Ad-hoc TOML/YAML validation | `cargo mobench config validate --config bench-config.toml` |
| Custom matrix/tag resolution | `cargo mobench devices resolve --platform <android|ios> --profile <tag>` |
| Manual build/upload/fetch orchestration | `cargo mobench ci run --target <android|ios|both> --function <fn>` |
| Custom artifact naming | Standard output dir `target/mobench/ci/` |
| Custom PR comment scripts | `cargo mobench report github --pr <n> --publish` |
| Hand-rolled cache keys | `cargo mobench fixture cache-key --config bench-config.toml` |

## Command Matrix

| Concern | Command |
| --- | --- |
| Preflight | `cargo mobench doctor --target both --config bench-config.toml --device-matrix device-matrix.yaml` |
| Config contract | `cargo mobench config validate --config bench-config.toml --format json` |
| Device resolution | `cargo mobench devices resolve --platform android --profile default --device-matrix device-matrix.yaml` |
| Fixture setup | `cargo mobench fixture init` |
| Fixture verify | `cargo mobench fixture verify --config bench-config.toml` |
| Fixture cache key | `cargo mobench fixture cache-key --config bench-config.toml --format json` |
| CI orchestration | `cargo mobench ci run --target both --function sample_fns::fibonacci --local-only --plots auto` |
| Summary markdown | `cargo mobench report summarize --summary target/mobench/ci/summary.json --plots auto` |
| Sticky PR comment | `cargo mobench report github --pr 123 --summary target/mobench/ci/summary.json --publish` |

## Minimal Reference Workflow

Use `.github/workflows/mobile-bench-action-example.yml` as the copy-paste baseline. Minimal form:

```yaml
name: Mobench CI (minimal)

on:
  workflow_dispatch:

permissions:
  contents: read
  pull-requests: write

jobs:
  mobench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-linux-android
      - uses: ./.github/actions/mobench
        with:
          command: cargo mobench ci run
          run-args: |
            --target android
            --function sample_fns::fibonacci
            --iterations 20
            --warmup 5
            --local-only
          pr-comment: true
          github-token: ${{ github.token }}
```

### Action input notes

- `command` is allow-listed to `cargo mobench ci run` and `cargo mobench run`.
- `ci` only appends `--ci` when `command: cargo mobench run`.
- Prefer multiline `run-args` with explicit quoting for values containing spaces.
- If you call `.github/workflows/reusable-bench.yml` directly, the caller workflow should grant `actions: read` in addition to any PR-comment permissions so baseline artifact lookup can read prior workflow runs.

### Summary output notes

- `summary.json`, `summary.md`, and `results.csv` remain the stable required outputs.
- Android CI defaults to `arm64-v8a`; add extra ABIs explicitly in `mobench.toml` or caller workflows only when needed.
- `plots/*.svg` is additive and only appears when local plot rendering is enabled and a Python + Matplotlib runtime is available, or when `--plots require` is used successfully.
- Local markdown summaries now include `cpu_total_ms` and `peak_memory_kb` instead of percentage/average-RAM columns.
- The reusable workflow attempts to compare against the latest successful default-branch run by downloading its per-platform `summary.json` artifacts before calling `ci check-run`.

## Compatibility Notes

- Versioned schemas: `docs/schemas/summary-v1.schema.json`, `docs/schemas/ci-contract-v1.schema.json`
- Current release history and support status: `RELEASE_NOTES.md`
- Current implementation reference: `README.md` and `docs/codebase/`

Any change to required output files or metadata keys requires updating the
versioned schemas and documenting the compatibility impact in `RELEASE_NOTES.md`.
