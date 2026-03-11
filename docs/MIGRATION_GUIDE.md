# Mobench CI Migration Guide

This guide migrates custom BrowserStack benchmark CI flows to the standardized `mobench` v1 contract.

## Goals

- One-command orchestration via `cargo mobench ci run`
- Stable contract outputs: `summary.json`, `summary.md`, `results.csv`
- Workflow-owned GitHub checks, sticky PR comments, and deterministic matrix resolution
- Stateless GitHub Actions automation keyed to compile success for the exact PR head SHA

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
| CI orchestration | `cargo mobench ci run --target both --function sample_fns::fibonacci --local-only` |
| Summary markdown | `cargo mobench report summarize --summary target/mobench/ci/summary.json` |
| Sticky PR comment | `cargo mobench report github --pr 123 --summary target/mobench/ci/summary.json --publish` |

## Default Repository Automation

The default v1 repository flow is stateless GitHub Actions, not the webhook/App service.

Workflow layout:
- `compile-gate.yml` is the authoritative compile gate and runs on `pull_request` for the exact PR head SHA.
- `mobile-bench-after-ci.yml` dispatches `mobile-bench.yml` after a successful compile gate when the PR is same-repo, open, and still labeled `bench`.
- `mobile-bench-pr-auto.yml` handles `bench` label events with `pull_request_target`, but it stays metadata-only and never checks out PR code.
- `mobile-bench-pr-command.yml` handles trusted `/mobench ...` PR comments and posts an explanatory sticky comment when the compile gate is not green yet.
- `mobile-bench.yml` remains the only benchmark runner and delegates execution to `reusable-bench.yml`.

Security constraints:
- Stateless v1 only supports same-repo PRs. Fork PRs are ignored.
- Controller workflows must never run or checkout PR head code.
- `pull_request_target` is used only for metadata checks and workflow dispatch.

## Minimal Reference Workflow

Use `.github/workflows/mobile-bench-action-example.yml` as the copy-paste baseline when you want the local action in another repository or a standalone setup. The default automation in this repo is the controller + single-runner model above.

Minimal action-based form:

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
          targets: aarch64-linux-android,armv7-linux-androideabi,x86_64-linux-android
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
- In the stateless repository flow, `dispatch_id` stays empty, `base_ref` is provided by the controller workflows for baseline resolution, and `head_sha` is provided so the runner checks out and reports on the exact commit that passed the compile gate.

## Compatibility Notes

- Contract docs: `docs/CONTRACT_CI_V1.md`
- ADR: `docs/adr/0001-mobench-ci-contract-v1.md`
- Schemas: `docs/schemas/summary-v1.schema.json`, `docs/schemas/ci-contract-v1.schema.json`

Any change to required output files or metadata keys requires a contract-version bump.
