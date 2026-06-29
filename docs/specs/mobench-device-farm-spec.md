# mobench Device Farm Provider Spec

Status: draft
Updated: 2026-04-30

This spec describes a `mobench` provider for running mobile benchmarks on a
private fleet of real Android and iOS devices. The provider is intended for
devices that are unavailable or unsuitable in hosted device-cloud services.

The device farm is narrower than a general-purpose mobile testing cloud. It
installs already-built benchmark apps, runs the generated Espresso or XCUITest
harness, collects logs and device metadata, and returns benchmark results
through an async API.

## Goals

- Run `mobench` benchmarks on managed physical Android and iOS devices.
- Add a `mobench`-native provider API alongside existing providers.
- Support async multi-device runs from day one.
- Return parsed benchmark results and raw debug artifacts.
- Integrate with local development and CI without requiring the farm to build
  mobile artifacts.
- Provide enough operator tooling to run and maintain a small unattended device
  fleet.

## Non-Goals

V1 does not attempt to:

- Simulate mobile OS APIs such as wallet payments, biometrics, camera, GPS, or
  sensors.
- Provide arbitrary remote device interaction.
- Execute arbitrary shell commands supplied by API callers.
- Replace all hosted device-cloud usage.
- Build or sign mobile artifacts inside the farm.
- Provide native flamegraph/profiling capture.
- Ship a full web UI before the API and operator CLI exist.

## Provider Model

The farm should fit the existing `mobench` lifecycle:

1. Build platform artifacts locally or in CI.
2. Upload artifacts to the provider.
3. Schedule a provider run on one or more devices.
4. Poll until completion.
5. Fetch logs, artifacts, and parsed benchmark results.
6. Write normalized `mobench` outputs such as `summary.json`, `summary.md`, and
   `results.csv`.

The API should be native to `mobench`; it should not copy another provider's
endpoint names, payload quirks, or product terminology.

## Architecture

Use one unified control-plane API with platform-specific rack agents.

Components:

- Control plane API: REST JSON API for artifacts, runs, sessions, devices,
  pools, identities, and admin operations.
- Scheduler: validates requests, reserves devices, creates per-device sessions,
  and assigns sessions to agents.
- Object storage: stores uploaded apps, test bundles, raw logs, screenshots,
  result bundles, and large artifacts.
- Relational database: stores projects, identities, agents, devices, pools,
  runs, sessions, leases, events, metrics, and normalized result rows.
- Android agents: controller processes connected to Android phones over USB,
  using `adb` and Android test tooling.
- iOS agents: controller processes connected to iPhones over USB, using Xcode
  command-line tooling for install and XCUITest execution.
- Operator CLI: scriptable fleet-management tool for device inspection,
  quarantine, logs, and run cancellation.
- `mobench` provider adapter: CLI integration selected by `--provider`.

Recommended physical split:

- Linux controllers for Android devices.
- macOS controllers for iOS devices.
- Wired Ethernet for controllers.
- Stable powered USB hubs for phones.
- Outbound-only agent connections to the control plane.

## Device Procurement

The pilot should optimize for end-to-end reliability before broad coverage.

Target device classes should be defined by measurable properties instead of only
model names.

Example Android tail-end class:

- 2 GB RAM.
- Low-end CPU representative of the target userbase.
- 32-bit ARM userspace or 32-bit-capable SoC where relevant.
- Android version capable of running the generated app and Espresso harness.
- 16-32 GB storage.
- Enough devices for multi-device runs plus at least one spare.

Example iOS tail-end class:

- Lowest-end arm64 iPhone models still compatible with the supported iOS target.
- Reliable signed app installation and XCUITest execution.
- Enough devices for multi-device runs plus at least one spare.

`mobench` may need platform-support work for older devices:

- Android jobs targeting 32-bit devices should build and declare
  `armeabi-v7a`.
- Android SDK and dependency defaults may need to remain configurable.
- iOS 32-bit devices are out of scope; low-end iOS means older supported arm64
  devices.

## Physical Inventory

Every physical element should map to device inventory.

Example labeling scheme:

```text
controller: android-rack-01, ios-rack-01
USB hub: android-rack-01-hub-a
port: android-rack-01-hub-a-p03
device: android-low-001
asset tag / serial: stored in inventory
```

Each device record should expose enough information for an operator to find and
service the device:

```json
{
  "device_id": "android-low-001",
  "controller_id": "android-rack-01",
  "usb_path": "android-rack-01-hub-a-p03",
  "serial_number": "...",
  "asset_tag": "...",
  "physical_label": "android-low-001"
}
```

Power recommendations:

- Use stable powered USB hubs.
- Prefer managed hubs with per-port power switching.
- At minimum, use controllable power at hub/controller level.
- Do not block the first pilot on per-port power switching if procurement is
  uncertain.

Network recommendations:

- Controllers should use wired Ethernet.
- Agents should initiate outbound HTTPS long-polling to the control plane.
- The farm should not require inbound access to rack controllers.

## Device Setup

Android phones should be preconfigured once:

- Developer options enabled.
- USB debugging enabled.
- Stay-awake while charging enabled.
- Animations disabled.
- Lock screen disabled where possible.
- Battery optimization disabled for the test app where possible.
- Connected to stable power.
- Enrolled in inventory with serial, model, OS version, ABI list, RAM, storage,
  and physical label.

iPhones should be preconfigured once:

- Device IDs recorded in inventory.
- Trusted by the macOS controller.
- Prepared for command-line install/test workflows.
- Enrolled in the minimum device/provisioning setup needed for reliable signed
  app installation.
- No dependency on personal developer accounts during normal farm operation.

## Data Model

Use a relational database for control-plane state and object storage for blobs.

Core tables:

- `projects`
- `identities`
- `identity_policies`
- `agents`
- `agent_heartbeats`
- `devices`
- `device_facts`
- `device_labels`
- `pools`
- `pool_memberships`
- `artifacts`
- `runs`
- `sessions`
- `leases`
- `session_events`
- `session_artifacts`
- `device_snapshots`
- `metric_samples`
- `benchmark_reports`
- `benchmark_summary_rows`

Database records should store:

- Run/session status transitions.
- Lease events.
- Failure reasons.
- Device facts, labels, snapshots, and health.
- Small metric samples.
- Parsed benchmark summaries.
- JSON copies of benchmark report payloads.
- Artifact metadata and object keys.

Object storage should store:

- App and test artifacts.
- Full device logs.
- Test runner and platform-tool output.
- Instrumentation logs.
- Screenshots/videos if enabled.
- Complete `bench-report.json` payloads.
- Zipped session artifact bundles.

## Device Inventory and Pools

Expose both raw device inventory and curated pools/profiles.

API:

```text
GET /v1/devices
GET /v1/devices/{device_id}
GET /v1/pools
GET /v1/pools/{pool_id}
```

Device example:

```json
{
  "id": "android-low-001",
  "display_name": "Android Low-End #1",
  "target": "android",
  "model": "Example Low-End Phone",
  "os_version": "13",
  "abi_list": ["armeabi-v7a"],
  "ram_mb": 2048,
  "storage_gb": 32,
  "agent_id": "android-rack-01",
  "usb_path": "android-rack-01-hub-a-p03",
  "state": "available",
  "pool_ids": ["android-tail-2gb-32bit"],
  "labels": {
    "tier": "tail-end"
  }
}
```

Pool example:

```json
{
  "pool_id": "android-tail-2gb-32bit",
  "target": "android",
  "description": "2 GB RAM 32-bit low-end Android devices",
  "available_sessions": 3,
  "capabilities": {
    "ram_gb_max": 2,
    "abi": "armeabi-v7a",
    "tier": "tail-end"
  }
}
```

Pool membership should combine:

- automatic facts detected by agents
- durable pool definitions from reviewed configuration
- manual labels for benchmark intent and operational state

Durable pool and policy definitions should be configuration-managed. Temporary
state such as quarantine, maintenance notes, and debug labels should live in the
API/admin plane.

## Device State Machine

Device states:

```text
available
leased
running
recovering
quarantined
maintenance
retired
unknown
```

Recovery ladder:

1. Kill stale runner/app processes.
2. Uninstall app/test packages.
3. Reconnect through platform tooling.
4. Reboot device.
5. Power-cycle USB port, hub, or controller-managed power if available.
6. Quarantine after repeated failures.

Quarantined devices are excluded from normal pool scheduling but may remain
targetable by exact ID for diagnostics when caller policy allows exact-device
use.

## Scheduling

The API supports both:

- curated pools/capability selectors
- exact physical device IDs

CI should use pools or selectors. Exact IDs are for debugging, reproduction,
maintenance, and quarantine validation.

A run can fan out across multiple physical devices. The control plane models
this as:

- parent `run`
- child `session` per physical device

Use all-or-nothing reservation for v1:

- all requested devices must be reserved before sessions start
- queue timeout controls how long a run waits for capacity
- partial capacity produces `capacity_timeout`

One physical device runs one session at a time. No shared-device parallelism.

## Run and Session States

Run statuses:

```text
created
validating
queued
leasing
running
collecting
completed
failed
canceled
expired
```

Session statuses:

```text
created
queued
leased
preparing_device
installing
running_test
collecting
completed
failed
recovering
quarantined_device
canceled
```

Failure codes:

```text
artifact_invalid
capacity_timeout
install_failed
test_timeout
no_bench_report_found
device_disconnected
device_unhealthy
agent_lost
abi_mismatch
os_mismatch
runner_failed
result_parse_failed
internal_error
```

Timeouts should be phase-specific:

```json
{
  "timeouts": {
    "queue_secs": 900,
    "device_prepare_secs": 180,
    "install_secs": 300,
    "test_secs": 900,
    "collect_secs": 120,
    "overall_secs": 1800
  }
}
```

Cancellation:

```text
POST /v1/runs/{run_id}/cancel
```

Agents must stop the runner if possible, collect final logs, clean up app/test
packages, release leases, and mark sessions `canceled`.

## Artifact Flow

Use presigned object-storage URLs. Large binaries should not flow through the
control plane API process.

Flow:

```text
POST /v1/artifacts/initiate
PUT  presigned_upload_url
POST /v1/artifacts/{id}/complete
POST /v1/runs
agent claims session and receives presigned download URLs
```

Artifact upload is generic, but run creation uses platform-specific roles.

Android:

```json
{
  "target": "android",
  "artifacts": {
    "app_apk": "artifact_123",
    "test_apk": "artifact_456"
  },
  "runner": {
    "kind": "espresso"
  }
}
```

iOS:

```json
{
  "target": "ios",
  "artifacts": {
    "app_ipa": "artifact_789",
    "xcuitest_zip": "artifact_abc"
  },
  "runner": {
    "kind": "xcuitest",
    "only_testing": [
      "BenchRunnerUITests/BenchRunnerUITests/testLaunchAndCaptureBenchmarkReport"
    ]
  }
}
```

V1 signing/build policy:

- CI or local `mobench` builds and signs artifacts before upload.
- The farm validates artifacts and installs them.
- The farm does not own mobile signing or build pipelines in v1.
- Server-side build/signing can be added later as a separate build service.

## API Contract

Use REST JSON with explicit versioning:

```text
/v1/...
```

Payloads include:

```json
{
  "api_version": "2026-04-30",
  "result_schema": "mobench-farm-result-v1",
  "bench_report_schema": "mobench-bench-report-v1"
}
```

Compatibility rules:

- v1 clients ignore unknown fields
- server does not remove or rename existing v1 fields
- breaking changes require `/v2` or a new schema name

Core endpoints:

```text
POST /v1/artifacts/initiate
POST /v1/artifacts/{artifact_id}/complete
GET  /v1/artifacts/{artifact_id}

POST /v1/runs
GET  /v1/runs/{run_id}
POST /v1/runs/{run_id}/cancel
GET  /v1/runs/{run_id}/sessions
GET  /v1/runs/{run_id}/results
GET  /v1/runs/{run_id}/artifacts

GET  /v1/sessions/{session_id}
GET  /v1/sessions/{session_id}/results
GET  /v1/sessions/{session_id}/artifacts

GET  /v1/devices
GET  /v1/devices/{device_id}
GET  /v1/pools
GET  /v1/pools/{pool_id}

POST /v1/agents/{agent_id}/heartbeat
POST /v1/agents/{agent_id}/leases/claim
POST /v1/sessions/{session_id}/events
POST /v1/sessions/{session_id}/artifacts
POST /v1/sessions/{session_id}/metrics
```

Async completion:

- polling is required in v1
- webhooks are a possible extension

Polling endpoints:

```text
GET /v1/runs/{run_id}
GET /v1/runs/{run_id}/results
GET /v1/runs/{run_id}/artifacts
```

Future webhook shape:

```json
{
  "webhook": {
    "url": "https://ci.example/hook",
    "events": ["run.completed", "run.failed"],
    "secret_ref": "..."
  }
}
```

## Run Creation Examples

Pool-based Android run:

```json
{
  "target": "android",
  "device_request": {
    "pool": "android-tail-2gb-32bit",
    "count": 3,
    "selector": {
      "abi": "armeabi-v7a",
      "ram_mb_max": 2300
    }
  },
  "artifacts": {
    "app_apk": "artifact_app",
    "test_apk": "artifact_test"
  },
  "runner": {
    "kind": "espresso",
    "instrumentation_args": {
      "class": "dev.world.bench.MainActivityTest"
    }
  },
  "scheduling": {
    "strategy": "all_or_nothing",
    "queue_timeout_secs": 900
  },
  "timeouts": {
    "queue_secs": 900,
    "device_prepare_secs": 180,
    "install_secs": 300,
    "test_secs": 900,
    "collect_secs": 120,
    "overall_secs": 1800
  }
}
```

Exact-device iOS run:

```json
{
  "target": "ios",
  "device_request": {
    "device_ids": ["ios-low-001", "ios-low-002"]
  },
  "artifacts": {
    "app_ipa": "artifact_app",
    "xcuitest_zip": "artifact_test"
  },
  "runner": {
    "kind": "xcuitest",
    "only_testing": [
      "BenchRunnerUITests/BenchRunnerUITests/testLaunchAndCaptureBenchmarkReport"
    ]
  },
  "scheduling": {
    "strategy": "all_or_nothing",
    "queue_timeout_secs": 900
  }
}
```

## Runner Security

V1 supports fixed runner kinds only:

- `espresso` for Android
- `xcuitest` for iOS

Allowed arguments must be controlled and allowlisted. Do not expose arbitrary
shell execution through caller-facing APIs.

The farm may run any APK/IPA/test bundle that matches the runner contract.
`mobench` metadata is preferred but not required for installation/execution.

Result policy:

- if `mobench` markers exist, parse and normalize results
- if markers do not exist, complete the run with raw logs and set parsed result
  status to `no_bench_report_found`
- projects may require parsed `mobench` results for benchmark-gating workflows

## Agent Behavior

One cross-platform agent codebase should have platform-specific executors:

- shared agent core: auth, heartbeat, lease claim, event upload, metric upload,
  artifact download/upload, status handling
- Android executor: wraps `adb`, logcat, package install/uninstall, device facts,
  and Espresso execution
- iOS executor: wraps Xcode/device tooling, install/uninstall where available,
  device logs, and XCUITest execution

Agents use polling or long-polling:

```text
POST /v1/agents/{agent_id}/heartbeat
POST /v1/agents/{agent_id}/leases/claim
POST /v1/sessions/{session_id}/events
POST /v1/sessions/{session_id}/artifacts
POST /v1/sessions/{session_id}/metrics
```

Agent failure:

- agents heartbeat frequently
- if heartbeat expires, active sessions become `agent_lost`
- leases are released only after a safety timeout
- devices managed by the lost agent become `unknown`
- devices return to scheduling only after rediscovery and health checks

## Session Lifecycle

Each benchmark session should start from a clean install/state.

Typical Android lifecycle:

1. Verify device is reachable through `adb`.
2. Capture pre-run snapshot.
3. Apply pre-run gates.
4. Uninstall old app/test packages if present.
5. Install app APK.
6. Install test APK.
7. Start logcat capture.
8. Run Espresso test.
9. Collect logs, instrumentation output, and screenshots if configured.
10. Parse or upload benchmark report artifacts.
11. Uninstall or leave installed based on maintenance policy.
12. Capture post-run snapshot.
13. Release lease.

Typical iOS lifecycle:

1. Verify device is visible to Xcode/device tooling.
2. Capture pre-run snapshot.
3. Apply pre-run gates.
4. Install signed app IPA.
5. Prepare XCUITest bundle.
6. Start device log capture.
7. Run XCUITest with `only_testing`.
8. Collect logs, test output, and screenshots if configured.
9. Parse or upload benchmark report artifacts.
10. Clean up where platform permits.
11. Capture post-run snapshot.
12. Release lease.

Installation reuse is debug-only and disallowed for CI benchmark sessions.

## Pre-Run Gates and Metrics

Normalize device condition with hard gates where possible and record everything
else.

Hard gates:

- battery >= 40% or externally powered
- thermal state not hot/critical where observable
- screen unlocked and automation-ready
- free storage above configured threshold
- no active previous session processes
- device reachable through platform tooling
- expected ABI/OS/platform matches request
- artifact compatibility checks pass

Recorded metadata:

- battery percentage
- charging state
- thermal state
- uptime
- available RAM/storage
- OS version/build
- controller ID
- USB hub/port
- pre-run CPU/load snapshot
- device model and ABI list

Metrics model:

- `device_snapshot`: pre-run and post-run facts
- `metric_samples`: periodic samples during prepare/install/test, every 5-10
  seconds by default
- `benchmark_resource_usage`: benchmark-emitted resource numbers remain
  authoritative for performance comparison

Keep fleet/device health metrics separate from benchmark metrics.

## Result Model

Store:

- original `BenchReport` JSON in object storage
- JSON copy in the relational database
- normalized summary rows in the relational database
- mobench-compatible aggregate grouped by device

Minimum normalized fields:

```text
session_id
device_id
function
iterations
warmup
sample_count
mean_ns
median_ns
p95_ns
min_ns
max_ns
cpu_total_ms
cpu_median_ms
peak_memory_kb
process_peak_memory_kb
created_at
```

Per-session result endpoint:

```text
GET /v1/sessions/{session_id}/results
```

Run aggregate endpoint:

```text
GET /v1/runs/{run_id}/results
```

Example aggregate:

```json
{
  "run_id": "run_123",
  "result_schema": "mobench-farm-result-v1",
  "benchmark_results": {
    "android-low-001": [
      {
        "function": "sample_fns::fibonacci",
        "samples": [],
        "mean_ns": 123,
        "median_ns": 120,
        "min_ns": 110,
        "max_ns": 150
      }
    ]
  },
  "devices": [
    {
      "id": "android-low-001",
      "display_name": "Android Low-End #1",
      "model": "Example Low-End Phone",
      "os": "android",
      "os_version": "13",
      "pool_ids": ["android-tail-2gb-32bit"]
    }
  ]
}
```

API payloads use stable device IDs as primary identifiers and include
human-readable labels for display.

## Authentication and Authorization

Support distinct identity classes:

- CI identities
- internal scoped API tokens
- external scoped API tokens
- rack agent credentials
- human/admin identities

External tokens are first-class principals. They can have explicit limits or
unbounded access when intentionally granted.

Identity policy example:

```json
{
  "project_id": "example-project",
  "allowed_pools": ["android-tail", "ios-tail"],
  "allowed_targets": ["android", "ios"],
  "max_concurrent_runs": 2,
  "max_concurrent_sessions": 6,
  "max_queue_seconds": 1800,
  "priority": "normal",
  "retention_days": 30,
  "can_use_exact_device_ids": false,
  "can_create_unbounded_runs": false,
  "expires_at": "2026-12-31T23:59:59Z"
}
```

Authorization is project-scoped from v1:

- project owns runs, artifacts, and results
- project can access selected device pools
- project has concurrency quotas
- project has usage quotas, optionally unlimited
- project has artifact retention policy
- project has allowed CI identities and API tokens

The same physical fleet can be shared across internal and external users.
Policies should live on auth identities and projects rather than requiring
separate hardware.

## Retention

Suggested defaults:

- raw logs/artifacts: 30 days
- parsed benchmark summaries: longer-lived, project-policy controlled
- failed-run debug bundles: 30 days unless project policy shortens or extends
- external tokens may default to shorter retention unless explicitly raised

Retention should be enforced through object-storage lifecycle policies plus
database cleanup jobs.

## mobench CLI Integration

Add a general provider model.

Example CLI:

```bash
cargo mobench run \
  --provider private_farm \
  --target android \
  --function sample_fns::fibonacci \
  --pool android-tail-2gb-32bit \
  --count 3 \
  --release \
  --fetch \
  --output target/mobench/results.json
```

Keep `--devices` compatibility, but add farm-native selectors:

```bash
cargo mobench run --provider private_farm --target android --pool android-tail-2gb-32bit --count 3
cargo mobench run --provider private_farm --target android --device-id android-low-001
cargo mobench run --provider private_farm --target android --selector abi=armeabi-v7a,ram_mb_max=2300 --count 2
```

Provider config:

```toml
[providers.browserstack]
kind = "browserstack"
username_env = "BROWSERSTACK_USERNAME"
access_key_env = "BROWSERSTACK_ACCESS_KEY"

[providers.private_farm]
kind = "mobench-farm"
base_url = "https://mobench-farm.example"
token_env = "MOBENCH_FARM_TOKEN"

[run]
provider = "private_farm"
```

Provider behavior:

- Hosted providers and farm providers remain separate.
- A single run targets one provider in v1.
- Device matrices may include provider-specific profiles.
- Cross-provider comparison happens in reporting.

## CI Integration

V1 flow:

1. CI runner builds Android/iOS artifacts using existing `mobench` build paths.
2. `mobench` initiates artifact upload and receives presigned URLs.
3. CI uploads artifacts to object storage.
4. `mobench` creates a farm run.
5. `mobench --fetch` polls the farm until completion.
6. `mobench` downloads or receives aggregate results.
7. Existing outputs are written:
   - `summary.json`
   - `summary.md`
   - `results.csv`
   - optional JUnit/check-run output

CI cancellation should call farm cancellation so devices are not held until
timeout.

## Operator CLI

V1 should ship an operator CLI before a web UI.

Required commands:

```bash
farmctl agents list
farmctl devices list --pool android-tail-2gb-32bit
farmctl devices inspect android-low-001
farmctl devices quarantine android-low-001 --reason "USB disconnect loop"
farmctl devices unquarantine android-low-001
farmctl runs list --status running
farmctl runs cancel run_123
farmctl sessions logs ses_123
farmctl pools list
```

The CLI should be usable by the person physically maintaining the rack.

## Observability

Control plane:

- structured JSON logs
- request IDs
- run/session event timeline
- metrics for queue depth, run duration, failure codes, artifact volume, and
  API errors

Agents:

- structured logs
- heartbeat metrics
- device discovery events
- command duration and exit status
- recovery attempts and quarantine causes

Alerts:

- agent offline
- stuck queue
- high install/test failure rate
- repeated device disconnects
- high quarantined device count
- object storage growth above threshold
- runs stuck beyond timeout

## Security Considerations

The farm installs arbitrary mobile binaries onto real devices. The security
model must assume uploaded apps can be malicious or broken.

V1 controls:

- no arbitrary shell command execution for callers
- fixed runner kinds only
- per-project and per-identity quotas
- per-agent credentials with independent revocation
- short-lived presigned URLs
- audit logs for run creation, cancellation, artifact access, token use, and
  admin operations
- outbound-only agent connectivity
- scoped external tokens with expiration and optional IP allowlists
- secrets stored in a managed secret store

## Rollout Plan

Phase 0: local spike

- one Android phone connected to a developer machine
- manually install/run generated APK/test APK
- prove log/result extraction on the target device class
- identify `mobench` changes needed for `armeabi-v7a` or older SDK support

Phase 1: Android rack pilot

- one Android controller
- 2-3 Android phones plus spare
- minimal agent
- local or development control plane
- repeatable multi-device Espresso jobs

Phase 2: iOS rack pilot

- one macOS controller
- 1-2 iPhones plus spare
- XCUITest execution through the same control plane model
- validate signing/provisioning assumptions

Phase 3: unified provider integration

- add `--provider` model to `mobench`
- add farm provider config
- add artifact upload/run/poll/fetch adapter
- write existing CI output contract

Phase 4: CI pilot

- run farm-backed Android and iOS jobs from CI
- support cancellation
- publish summaries/checks
- monitor a soak period

Phase 5: external access pilot

- create scoped API tokens for selected external users
- validate quotas, retention, audit logs, and support process

Phase 6: fleet expansion

- add more devices matching measured target classes
- tune controller-to-phone ratios
- add per-port power switching if pilot data justifies it

## Pilot Success Criteria

The pilot is successful when:

- CI can submit Android and iOS farm runs through `mobench`.
- The farm runs multi-device jobs unattended.
- Results include parsed benchmark numbers grouped by stable device ID.
- Raw logs and session artifacts are available for failed runs.
- Common device failures trigger recovery or quarantine without blocking the
  whole fleet.
- No manual intervention is needed across a meaningful soak window.
- Operators can identify and service a physical phone from API/CLI inventory
  data.

## Open Questions

These should be resolved before purchase or production deployment:

- Which exact Android and iOS models best represent the target device classes?
- What is the acceptable budget for controllers, phones, hubs, mounts, power,
  and spares?
- What device provisioning setup should be used for iOS test devices?
- What retention and access policies should external identities receive by
  default?
- Should the control plane be private-network-only or expose a public API with
  strict auth and rate limits?

