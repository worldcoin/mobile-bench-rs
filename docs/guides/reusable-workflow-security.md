# Reusable Workflow Security

Current secure workflow release: **0.1.47** (older releases remain immutable).

The reusable BrowserStack workflow is secure by default for pull requests,
including fork pull requests. Its core invariant is:

> Pull-request-controlled code never executes in a job that has BrowserStack
> credentials or write-capable repository credentials.

An authorized maintainer's `/mobench` comment authorizes a run. It does not make
the pull request's commit, dependencies, build scripts, hooks, fixtures, or
benchmark binaries trusted.

## Two-Stage Architecture

The workflow separates code construction from provider access:

1. `validate-request` accepts only a 40-character commit SHA and confirms it is
   still the current head SHA of the requested pull request. Transient GitHub
   API failures receive five bounded retries; exhausted requests and SHA
   mismatches fail closed.
2. `prepare-android` and `prepare-ios` check out that exact SHA, generate
   fixtures, compile/package the mobile runners, and upload a run-scoped handoff.
3. `run-android` and `run-ios` download and verify the handoff,
   upload the prebuilt artifacts, run them on BrowserStack devices, and fetch
   results. They do not check out the pull request.
4. `summarize` combines read-only result artifacts.
5. `report` publishes the sticky PR comment or check result without checking
   out or executing pull-request code.
6. Plot-branch publication is a separate manual workflow,
   `mobile-bench-publish-plots.yml`. It is the only workflow allowed to request
   `contents: write` and is isolated behind the `mobench-plots` environment.

```mermaid
flowchart LR
    request["PR number + exact head SHA"]
    validate["validate-request\nread only"]
    prepare["prepare Android/iOS\nuntrusted code, no secrets"]
    handoff["enumerated artifacts\n+ manifest"]
    provider["BrowserStack jobs\nsecrets, no caller checkout"]
    device["BrowserStack devices\nmobile code executes here"]
    results["untrusted result artifacts"]
    summarize["summarize\nread only"]
    report["report\nPR/check write only"]

    request --> validate --> prepare --> handoff --> provider --> device
    device --> results --> summarize --> report
```

## Job Permissions And Secrets

The workflow-level permission set is empty. Each job receives only its required
permission:

| Job | Caller checkout or execution | GitHub permission | BrowserStack secrets | Protected environment |
| --- | --- | --- | --- | --- |
| `validate-request` | No | `contents: read`, `pull-requests: read` | No | No |
| `prepare-android`, `prepare-ios` | Exact PR head; untrusted | `contents: read` | No | No |
| `run-android`, `run-ios` | No | `actions: read`, `pull-requests: read` | Upload/run/fetch step only | `browserstack` defense in depth |
| `summarize` | No | `actions: read` | No | No |
| `report` | No | `pull-requests: write` and/or `checks: write` | No | No |
| separate plot workflow | No | `actions: read`, `contents: write` | No | Protected manual opt-in |

BrowserStack values must be passed by name. `secrets: inherit` is not supported
as a safe caller pattern. Environment approval can limit accidental execution,
but artifact verification and the no-caller-execution rule are the primary
security boundary.

## Untrusted Prepare Phase

The prepare jobs deliberately allow the pull request to run Rust build scripts,
dependencies, fixture generators, Gradle/Xcode builds, hooks, and benchmark
code needed to produce the mobile packages. Those processes receive no
BrowserStack variables, no protected environment, and no write-capable GitHub
token.

Prepare uses:

```bash
cargo mobench ci prepare \
  --target android \
  --ffi-backend native-c-abi \
  --source-sha "$PR_HEAD_SHA" \
  --output-dir target/mobench/prebuilt/android \
  --manifest target/mobench/prebuilt/android/manifest.json
```

Use the corresponding `--target ios` invocation for the IPA and XCUITest suite.
The workflow uploads only the artifact roles explicitly allowed for that
platform and the generated manifest. It does not upload a caller workspace,
arbitrary glob, build cache, or executable helper for later runner execution.

Caches used by pull-request preparation are either disabled or scoped so they
cannot become a trusted build input. Trusted jobs do not restore build outputs
created by untrusted revisions.

Callers may set `rust_toolchain` to the exact channel used for mobile builds,
for example `nightly-2026-03-04`; it defaults to `stable`. The Android and iOS
targets are installed for that toolchain only in the secretless prepare jobs.
The trusted `cargo-mobench` control plane remains compiled separately with its
own pinned `stable` toolchain and never reads caller toolchain files.

The optional `ffi_backend` input is validated and passed directly to
`ci prepare` as `--ffi-backend`; the workflow never rewrites `mobench.toml`.
If omitted, project configuration and then Mobench's documented default apply.
UniFFI generator discovery uses the Cargo workspace containing `crate_path`;
native-C-ABI callers skip generator installation entirely.

### Generic preparation hook

Projects that need code generation or toolchain setup before packaging may pass
`prepare_script`, for example `.github/scripts/prepare-mobench.sh`. Mobench does
not embed project-specific Noir, ProveKit, circuit, or fixture commands.

The hook path must be a normalized repository-relative POSIX path. Absolute
paths, `.` or `..` components, repeated separators, backslashes, controls,
missing targets, directories, and symlinks resolving outside the exact caller
checkout are rejected. The resolved file is invoked through `bash` after
checkout and before `cargo-mobench ci prepare`.

The hook receives `MOBENCH_CI_PREPARE=1` and `MOBENCH_PLATFORM=ios` or
`android`; packaging also receives `MOBENCH_CI_PREPARE=1`. These commands run
only in the matching secretless prepare job. Any failure stops the job before a
manifest or mobile package can be uploaded.

### Platform functions and devices

`functions_ios` and `functions_android` use the same JSON-array or
comma-separated syntax as `functions`. An empty platform-specific input falls
back to the shared list. The normalized effective list is passed to both
preparation and `run-prebuilt`, binding the verified manifest to the trusted
request.

`ios_devices` and `android_devices` accept strict JSON arrays:

```json
[
  {"device": "iPhone 15", "os_version": "17"},
  {"device": "iPhone 14", "os_version": "16"}
]
```

The structured input takes precedence over the existing single-device fields
and `device_profile` fallback. Values remain quoted data and are never evaluated
as commands. Each function is built once per platform; `run-prebuilt` schedules
all selected devices and requires exactly one result for every function/device
combination. Missing, unexpected, or duplicate shards fail before canonical
outputs are written. BrowserStack diagnostics are fetched first so a partial
provider failure remains diagnosable without being presented as complete.

When both platforms are selected, the credentialed iOS and Android jobs run
serially. This limits BrowserStack concurrency without moving caller checkout,
build commands, or mobile execution into credentialed GitHub runner jobs.

## Artifact Manifest Boundary

The machine-readable manifest identifies every allowed file with:

- normalized relative path;
- artifact role and platform;
- exact byte size;
- lowercase SHA-256 digest;
- benchmark ABI name/version and compatibility metadata.

Before credentials are exposed, `run-prebuilt` rejects:

- absolute paths, `..` traversal, links, duplicate paths, and paths outside the
  downloaded artifact root;
- missing files, extra platform payloads, unexpected artifact roles, and
  invalid manifest or ABI versions;
- empty, oversized, or size-mismatched files;
- malformed or mismatched SHA-256 hashes.

The trusted phase then uses only the verified APK and Android test APK, or the
verified IPA and XCUITest package. Those mobile artifacts are uploaded as
opaque bytes; they are never launched, imported, sourced, unpacked for
execution, or otherwise run on the credentialed GitHub runner.

## Credentialed Prebuilt Phase

The trusted release is pinned to an immutable commit SHA. The credentialed jobs
may run only that trusted mobench binary and fixed workflow commands:

```bash
cargo mobench ci run-prebuilt \
  --manifest prebuilt/android/manifest.json \
  --expected-source-sha "$PR_HEAD_SHA" \
  --expected-platform android \
  --expected-functions '["benchmarks::critical_path"]' \
  --expected-iterations 30 \
  --expected-warmup 5 \
  --max-completion-timeout-secs 1800 \
  --devices "Google Pixel 7-13.0" \
  --output-dir target/mobench/ci/android
```

`run-prebuilt` verifies the manifest and performs BrowserStack upload, session
creation, polling, fetch, and output normalization. It never invokes Cargo,
Gradle, Xcode, caller hooks, dependencies, fixture generators, benchmark
binaries, or other files from a caller checkout on the GitHub runner.

`max_completion_timeout_secs` is a trusted workflow input with a 1,800-second
default and a 21,600-second hard maximum. The trusted binary bounds every
manifest-provided completion timeout to this value. Generated iOS runners use
heartbeat interactions and validated accessibility report channels for long
sessions; native Android runners emit structured worker-exit and linkage-error
diagnostics. These controls affect mobile/provider reliability only and do not
expand the caller-controlled execution surface of credentialed jobs.

## Reporting Boundary

BrowserStack responses and downloaded JSON, CSV, Markdown, filenames, device
names, and benchmark names remain untrusted. The workflow and report commands:

- pass values as data rather than interpolating them into shell programs;
- reject unsafe paths and control characters;
- prevent GitHub workflow-command interpretation;
- escape untrusted Markdown/HTML content and preserve the sticky-comment marker;
- do not use report-provided filenames as upload, checkout, branch, or command
  paths.

The reporting job consumes only validated result artifacts. It never checks out
the pull request or executes a file from those artifacts.

## Caller Migration

Update the reusable workflow reference to immutable commit
`1ac54adaf2bd97c6ca303705e1e0471257716f48` for the `v0.1.47`
release and pass secrets explicitly:

```yaml
permissions:
  actions: read
  contents: read
  pull-requests: write
  checks: write

jobs:
  mobench:
    uses: worldcoin/mobile-bench-rs/.github/workflows/reusable-bench.yml@1ac54adaf2bd97c6ca303705e1e0471257716f48
    with:
      pr_number: ${{ github.event.pull_request.number }}
      head_sha: ${{ github.event.pull_request.head.sha }}
      crate_path: crates/benchmarks
      functions: '["benchmarks::critical_path"]'
      functions_ios: '["benchmarks::ios_critical_path"]'
      prepare_script: .github/scripts/prepare-mobench.sh
      rust_toolchain: nightly-2026-03-04
      ffi_backend: native-c-abi
      android_devices: '[{"device":"Google Pixel 7","os_version":"13.0"}]'
      max_completion_timeout_secs: 7200
      platform: both
    secrets:
      BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}
      BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}
```

Do not replace the immutable SHA with a mutable branch or tag and do not use
`secrets: inherit`. The caller-level write grants allow the reusable workflow's
isolated `report` job to update the sticky comment and check run; its prepare
and BrowserStack jobs still downscope their own tokens to read-only permissions.
The PR command path requires both `pr_number` and a full `head_sha`, including
for fork PRs. A non-PR release self-test must opt out explicitly with
`allow_non_pr: true`; it is not the secure default. Existing function,
iteration, warmup, platform, device,
artifact collection, summary, and sticky-comment inputs continue through the
secure split workflow; callers do not need to duplicate its internal jobs.

## Validation Expectations

Release validation includes:

- hostile fixture coverage in which `build.rs`, a fixture hook, a dependency,
  and benchmark code attempt to read BrowserStack variables and write through
  the GitHub token; the executable harness asserts empty secret captures and
  intercepts each denied repository push;
- static workflow tests proving credentialed jobs have no caller checkout or
  caller-controlled process execution;
- fail-closed tests covering all five exact-head API retry sites;
- manifest path/hash/size/platform/ABI rejection tests;
- report-field escaping and workflow-command injection tests;
- `actionlint`, workspace workflow/self-tests, and one Android plus one iOS
  BrowserStack run through the prebuilt path.

The live Android and iOS BrowserStack runs are service-gated release checks and
must be reported separately from host-side or static validation. The 0.1.47
candidate passed the complete ProveKit age-check, fragmented age-check, and OPRF
matrix on both platforms with two measured samples and one warmup per device.
