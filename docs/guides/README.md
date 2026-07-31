# mobench Guides

Current published release line: **0.1.47**. Web commands documented in this
branch belong to the unreleased 0.2 parity candidate.

These guides cover the current mobench CLI, SDK, generated mobile runners,
BrowserStack execution, CI outputs, artifact fetching, and local native
profiling.

## Guides

- [sdk-integration.md](sdk-integration.md): add `mobench-sdk`, annotate
  benchmarks, choose `uniffi` or `native-c-abi`, and run benchmarks.
- [examples.md](examples.md): minimal benchmark, setup/teardown,
  native C ABI export, CI output, and programmatic integration patterns.
- [build.md](build.md): Android and iOS prerequisites, build outputs,
  WebAssembly bundles, project resolution, generated runner backends, and
  troubleshooting.
- [testing.md](testing.md): host tests, CLI checks, local smoke tests,
  Android/iOS validation, BrowserStack smoke tests, and profiling checks.
- [browserstack-ci.md](browserstack-ci.md): BrowserStack credentials,
  deterministic device resolution, `ci run`, split-sample merging, PR reporting,
  baselines, and artifact fetching.
- [browserstack-metrics.md](browserstack-metrics.md): timing/resource metrics,
  CSV columns, BrowserStack log parsing, and local profiling boundaries.
- [fetch-results.md](fetch-results.md): `--fetch`, `mobench fetch`, and
  `mobench summary` workflows for remote artifacts.
- [profiling.md](profiling.md): local native capture with `android-native` and
  `ios-instruments`, `rust-tracing` trace contracts, semantic phases, and
  profile diffs.
- [release.md](release.md): release checklist for the workspace crates and docs.
- [reusable-workflow-security.md](reusable-workflow-security.md): the
  secretless prepare and credentialed prebuilt-only trust boundary.

For the full behavior and API contract, see
[../specs/mobench-current-spec.md](../specs/mobench-current-spec.md).
