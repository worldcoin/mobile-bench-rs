# Mobench Webhook Server Design

> Note: This document describes an alternative stateful webhook/App design. The default v1 path in this repository is the stateless GitHub Actions flow described in `docs/CONTRACT_CI_V1.md` and `docs/MIGRATION_GUIDE.md`.

## Status
Design - 2026-03-06

## Overview
A stateful `axum` service in `mobile-bench-rs` that:

1. receives GitHub App webhooks,
2. durably records webhook deliveries before acknowledging GitHub,
3. auto-dispatches benchmark workflows on `bench` labels,
4. owns mobench GitHub Check Runs,
5. ingests canonical benchmark result bundles from completed workflows, and
6. serves historical benchmark data from PostgreSQL for trend analysis and regression detection.

## Problem
Today, mobile benchmarks are triggered manually via `/mobench` PR comments or `workflow_dispatch`. Results are summarized in GitHub, but they are not stored in a queryable history system. That prevents:

- trend analysis on `main`,
- stable PR vs baseline comparisons over time,
- exact replay of prior benchmark configurations from a Check Run rerun, and
- downstream tooling that needs benchmark history as data instead of logs.

## Goals
1. Auto-trigger benchmarks when a PR is labeled `bench`.
2. Re-dispatch the exact prior benchmark configuration when a server-owned Check Run is re-requested.
3. Ingest and store benchmark results from completed `mobile-bench.yml` workflow runs.
4. Serve historical benchmark data via a query API for trend analysis and regression detection.
5. Preserve existing manual triggers (`/mobench`, `workflow_dispatch`) and ingest their results too.
6. Route `/mobench ...` PR comment dispatches through the GitHub App so dispatch state, reruns, and ingest all flow through the service database.

## Non-goals
- Dashboard UI.
- Slack or GitHub notification policy for regressions.
- Replacing the existing `/mobench` command syntax.
- Public query API authentication for v1.
- Real-time streaming updates while benchmarks are still running.

---

## Architecture

```
                      public ingress                         private ingress
                  +------------------+                    +------------------+
GitHub ---------->| POST /webhook    |                    | GET /api/*       |
                  | GET /healthz     |                    | GET /api/healthz |
                  +---------+--------+                    +---------+--------+
                            |                                           |
                            v                                           v
                    +----------------------------------------------+
                    | mobench-webhook (axum + worker)             |
                    |                                              |
                    |  verify signature                            |
                    |  persist delivery                            |
                    |  background worker                           |
                    |   - dispatch workflow                        |
                    |   - fetch canonical result bundle            |
                    |   - create/update Check Runs                 |
                    |   - write Postgres history                   |
                    +-------------------+--------------------------+
                                        |
                                        v
                               PostgreSQL (RDS)
                                        |
                                        v
                                   GitHub API
```

The service has two responsibilities:

- **Webhook ingress**: public endpoint for GitHub App events.
- **History API**: private endpoint for internal consumers.

Default deployment is one Fargate service with two ingress paths:

- a public ALB exposing only `POST /webhook` and `GET /healthz`,
- an internal ALB or private service discovery endpoint exposing `/api/*`.

This removes the "public webhook but private query API" contradiction.

---

## Required Workflow Changes

The server design depends on small but explicit changes in `mobile-bench-rs/.github/workflows/reusable-bench.yml`:

1. The workflow must upload one canonical artifact bundle named `mobench-history-v1`.
2. The bundle must be produced by the summarize job after per-platform results are downloaded.
3. The workflow must accept an optional `dispatch_id` input and write it into `manifest.json`.
4. The workflow must accept `trigger_source` and `request_command` inputs and write them into `manifest.json`.
5. The workflow must stop creating GitHub Check Runs directly. The webhook service owns Check Run creation and updates.
6. Sticky PR comments may stay in the workflow. They are independent of the history service.

### Canonical Artifact Bundle

Artifact name:

```text
mobench-history-v1
```

Bundle layout:

```text
manifest.json
ios/
  summary.json
  summary.md
  results.csv
android/
  summary.json
  summary.md
  results.csv
```

Rules:

- Each platform directory is optional. At least one must exist for a successful ingest.
- Each `summary.json` must conform to the current mobench CI contract v1 for a single platform result.
- `dispatch_id` may be omitted for manual or legacy callers. When absent, ingest still succeeds; only pre-run dedupe and dispatch correlation are skipped.
- If the reusable workflow continues to fan out per-function JSON files internally, the summarize job materializes this bundle before upload.

### `manifest.json`

Good default schema:

```json
{
  "schema_version": "mobench-history-v1",
  "repo": "worldcoin/world-id-protocol",
  "workflow": {
    "name": "Mobile Benchmarks",
    "run_id": 123456789,
    "run_attempt": 1
  },
  "git": {
    "head_sha": "abc123",
    "head_ref": "feature/branch",
    "base_ref": "main"
  },
  "request": {
    "dispatch_id": "2b5ab4f5-c3a6-4d78-9d62-d1c3f7188d21",
    "trigger_source": "label|pr_comment|workflow_dispatch|check_rerequest",
    "requested_by": "octocat",
    "pr_number": 123,
    "request_command": "/mobench platform=both iterations=30 warmup=5 device_profile=low-spec"
  },
  "mobench": {
    "version": "0.1.15",
    "ref": "refs/tags/v0.1.15"
  },
  "platform_runs": [
    {
      "platform": "ios",
      "check_run_name": "Mobench - ios",
      "workflow_inputs": {
        "platform": "ios",
        "device_profile": "low-spec",
        "ios_device": "",
        "ios_os_version": "",
        "android_device": "",
        "android_os_version": "",
        "iterations": "30",
        "warmup": "5",
        "pr_number": "123",
        "requested_by": "octocat"
      },
      "resolved_device": {
        "device_name": "iPhone 14",
        "os_version": "16.0"
      }
    }
  ]
}
```

Why this default:

- it matches the current workflow contract,
- it captures enough data for exact reruns,
- it lets the webhook service correlate a dispatch request with the later workflow run,
- it records `mobench_version`, `mobench_ref`, and `request_command` for longitudinal comparisons, and
- it supports one workflow run producing multiple platform results.

---

## Check Run Ownership

The webhook service owns mobench Check Runs.

That means:

- `reusable-bench.yml` no longer calls `cargo-mobench ci check-run`,
- the history service creates or updates Check Runs after ingesting `mobench-history-v1`,
- the service stores `check_run_id` on each platform run,
- `check_run.rerequested` can be replayed exactly by reading stored `workflow_inputs` for that platform run.

This is the cleanest way to make Check Run reruns deterministic. The workflow already knows how to produce results; the service should own history and GitHub state.

---

## Event Handling

### Durable Webhook Flow

```text
POST /webhook
  -> verify X-Hub-Signature-256
  -> read X-GitHub-Delivery, X-GitHub-Event
  -> insert github_webhook_deliveries row with unique delivery_id
  -> return 202 Accepted

background worker loop
  -> claim next pending delivery using FOR UPDATE SKIP LOCKED
  -> route by (event, action)
  -> mark delivery processed or failed
```

The request path does not do heavy work after acknowledging GitHub. Durability comes from persisting the delivery before returning `202`.

### Event Routing

| Event | Action | Behavior |
|---|---|---|
| `pull_request` | `labeled` | If label is `bench`, PR is open, and the head repo matches the base repo, dispatch `mobile-bench.yml` with normalized workflow inputs |
| `issue_comment` | `created` | If the comment is on a PR, starts with `/mobench`, and the actor is trusted, parse overrides, create a dispatch row, and dispatch `mobile-bench.yml` through the GitHub App |
| `check_run` | `rerequested` | If the check run belongs to this app and `check_run_id` maps to a stored platform run, re-dispatch with the exact stored `workflow_inputs` for that platform |
| `workflow_run` | `completed` | For `mobile-bench.yml`, fetch `mobench-history-v1`, ingest results, and create or update Check Runs |
| anything else | any | Mark delivery ignored and return success |

### Idempotency and Deduplication

- `github_webhook_deliveries.delivery_id` is unique. Redeliveries become no-ops.
- Label-trigger dispatches are deduplicated on `(repo, head_sha, normalized workflow_inputs)` while a matching run is still queued or in progress.
- Workflow-run ingest is idempotent on `workflow_run_id`.
- Check-run updates are idempotent on `(workflow_run_id, platform)`.

### Trigger Guards

Defaults for `pull_request.labeled`:

- only `label.name == "bench"` triggers a run,
- fork PRs are ignored,
- closed PRs are ignored,
- if a matching run for the same SHA and normalized inputs is already queued or running, do not dispatch again.

This preserves the current fork safety in `.github/workflows/mobile-bench-pr-command.yml`.

Defaults for `issue_comment.created`:

- only PR comments are eligible,
- only comments beginning with `/mobench` are parsed,
- only trusted `author_association` values (`OWNER`, `MEMBER`, `COLLABORATOR`) may dispatch,
- parsed overrides are normalized into the same workflow input shape used by label dispatch,
- the service persists the dispatch row before calling GitHub Actions.

---

## Data Model

Good default model:

```sql
CREATE TABLE github_webhook_deliveries (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    delivery_id         TEXT NOT NULL UNIQUE,
    event               TEXT NOT NULL,
    action              TEXT,
    payload             JSONB NOT NULL,
    status              TEXT NOT NULL DEFAULT 'pending',
    attempts            INTEGER NOT NULL DEFAULT 0,
    last_error          TEXT,
    received_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    available_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at          TIMESTAMPTZ,
    processed_at        TIMESTAMPTZ
);

CREATE TABLE benchmark_dispatches (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dispatch_id         UUID NOT NULL UNIQUE,
    repo_owner          TEXT NOT NULL,
    repo_name           TEXT NOT NULL,
    head_sha            TEXT NOT NULL,
    head_ref            TEXT NOT NULL,
    pr_number           INTEGER,
    trigger_source      TEXT NOT NULL,
    requested_by        TEXT,
    workflow_inputs     JSONB NOT NULL,
    status              TEXT NOT NULL DEFAULT 'queued',
    workflow_run_id     BIGINT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at        TIMESTAMPTZ
);

CREATE TABLE benchmark_workflow_runs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_run_id     BIGINT NOT NULL UNIQUE,
    workflow_run_attempt INTEGER NOT NULL,
    repo_owner          TEXT NOT NULL,
    repo_name           TEXT NOT NULL,
    workflow_name       TEXT NOT NULL,
    head_sha            TEXT NOT NULL,
    head_ref            TEXT NOT NULL,
    base_ref            TEXT,
    pr_number           INTEGER,
    trigger_source      TEXT NOT NULL,
    requested_by        TEXT,
    request_command     TEXT,
    mobench_version     TEXT,
    mobench_ref         TEXT,
    conclusion          TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at        TIMESTAMPTZ
);

CREATE TABLE benchmark_platform_runs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_run_uuid   UUID NOT NULL REFERENCES benchmark_workflow_runs(id),
    platform            TEXT NOT NULL,
    check_run_id        BIGINT UNIQUE,
    check_run_name      TEXT NOT NULL,
    workflow_inputs     JSONB NOT NULL,
    device_profile      TEXT,
    device_name         TEXT NOT NULL,
    os_version          TEXT NOT NULL,
    iterations          INTEGER NOT NULL,
    warmup              INTEGER NOT NULL,
    status              TEXT NOT NULL DEFAULT 'completed',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at        TIMESTAMPTZ,
    UNIQUE (workflow_run_uuid, platform)
);

CREATE TABLE benchmark_results (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    platform_run_uuid   UUID NOT NULL REFERENCES benchmark_platform_runs(id),
    function_name       TEXT NOT NULL,
    function_label      TEXT NOT NULL,
    avg_ms              DOUBLE PRECISION NOT NULL,
    median_ms           DOUBLE PRECISION,
    p95_ms              DOUBLE PRECISION,
    best_ms             DOUBLE PRECISION NOT NULL,
    worst_ms            DOUBLE PRECISION NOT NULL,
    std_dev_ms          DOUBLE PRECISION,
    cpu_avg_percent     DOUBLE PRECISION,
    cpu_peak_percent    DOUBLE PRECISION,
    ram_avg_mb          DOUBLE PRECISION,
    ram_peak_mb         DOUBLE PRECISION,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (platform_run_uuid, function_name)
);

CREATE INDEX idx_platform_runs_sha_platform
    ON benchmark_workflow_runs (head_sha, created_at DESC);

CREATE INDEX idx_platform_runs_pr
    ON benchmark_workflow_runs (pr_number)
    WHERE pr_number IS NOT NULL;

CREATE INDEX idx_results_function_device
    ON benchmark_results (function_name, platform_run_uuid);

CREATE INDEX idx_dispatches_head_sha_status
    ON benchmark_dispatches (head_sha, status, created_at DESC);
```

This fixes the previous schema bug. The storage hierarchy is:

```text
workflow run -> platform run -> benchmark result
```

That matches the current reusable workflow, which can emit both iOS and Android results from one GitHub workflow run.

### Key Queries

- Trend: benchmark + platform + device on `main` over the last N platform runs.
- PR comparison: one `platform_run_id` vs the latest successful baseline on `main`.
- Regression detection: compare one platform run against rolling average or latest successful baseline for the same platform and device.

---

## Query API

Private API only.

```text
GET /api/healthz

GET /api/workflow-runs
    ?branch=main&pr_number=123&limit=20

GET /api/workflow-runs/:workflow_run_id

GET /api/platform-runs/:platform_run_id

GET /api/trends
    ?function=bench_nullifier_proving_only
    &platform=ios
    &device_name=iPhone+14
    &branch=main
    &limit=50

GET /api/compare
    ?platform_run_id=<uuid>
    &baseline_branch=main
    &threshold_pct=5.0
```

Defaults:

- comparisons are explicit by `platform_run_id`, not by `pr_number`,
- trend and compare are scoped by platform and resolved device,
- `threshold_pct` defaults to `5.0`.

This removes ambiguity when one PR has multiple mobench runs.

---

## GitHub App Authentication

The service authenticates as the GitHub App:

```text
App private key (PEM)
  -> sign JWT (RS256, exp <= 10m)
  -> create installation token
  -> cache token in-memory until ~10m before expiry
  -> use token for:
       - workflow dispatch
       - artifact download
       - Check Run create/update
       - PR lookup for label-trigger guards
```

In-memory token caching is sufficient for v1 because the service does not require cross-instance coordination.

---

## Dispatch Path

When a `pull_request.labeled` event arrives with label `bench`:

1. Worker loads the PR from GitHub.
2. If the PR head repo is a fork, ignore.
3. Normalize workflow inputs using current world-id-protocol defaults:
   - `platform=both`
   - `device_profile=low-spec`
   - `iterations=30`
   - `warmup=5`
   - custom device overrides empty
4. Check `benchmark_dispatches` for an existing queued/running dispatch with the same SHA and normalized inputs.
5. If none exists:
   - create a `benchmark_dispatches` row with a new `dispatch_id`,
   - pass that `dispatch_id` into the workflow inputs,
   - dispatch `mobile-bench.yml` on the PR head branch.

When a `check_run.rerequested` event arrives:

1. Look up `benchmark_platform_runs.check_run_id`.
2. Load stored `workflow_inputs`.
3. Override:
   - `requested_by` with the rerun actor login if present,
   - `platform` with the platform run being retried.
4. Create a new `benchmark_dispatches` row for the replay request.
5. Dispatch `mobile-bench.yml` on the original `head_ref`.

This gives exact replay semantics with sensible attribution.

---

## Ingestion Path

When a `workflow_run.completed` event arrives for `mobile-bench.yml`:

1. Fetch the `mobench-history-v1` artifact for that workflow run.
2. Parse `manifest.json`.
3. If `manifest.request.dispatch_id` is present, update the matching `benchmark_dispatches` row with `workflow_run_id`.
4. Upsert `benchmark_workflow_runs`.
5. For each platform directory present:
   a. parse `summary.json`,
   b. upsert `benchmark_platform_runs`,
   c. replace or upsert `benchmark_results`,
   d. compute regression status against the latest successful baseline on `main` for the same platform and device,
   e. create or update the GitHub Check Run for that platform and store `check_run_id`.
6. Mark the workflow run as ingested and mark the matching dispatch row completed if one exists.

### Baseline Default

Good default:

- compare against the latest successful platform run on `base_ref` if present,
- otherwise compare against the latest successful platform run on `main`,
- if no baseline exists, publish a success Check Run without regression annotations.

This is easy to explain and stable for first-run behavior.

---

## Error Handling

| Scenario | Behavior |
|---|---|
| Signature verification fails | `401`, log warning, do not persist delivery |
| Duplicate `X-GitHub-Delivery` | return `202`, do not create a second row |
| Worker crashes mid-processing | delivery stays pending or is retried after claim timeout |
| GitHub API rate limited | honor `Retry-After` or `X-RateLimit-Reset`; reschedule delivery with backoff |
| Canonical artifact missing | mark workflow run failed to ingest, keep retryable until max attempts |
| Manifest parse fails | mark delivery failed, keep payload and artifact metadata for debugging |
| Summary parse fails for one platform | mark that platform run failed, continue ingesting other platform directories |
| DB write fails | rollback transaction, increment attempts, retry later |
| Unknown event type | mark delivery ignored |
| `check_run.rerequested` for unknown `check_run_id` | log and ignore; do not guess inputs |

Retry default:

- exponential backoff with jitter,
- cap at 5 attempts for transient failures,
- leave terminal failures queryable in the database.

---

## Observability

Structured logs via `tracing`:

```text
webhook.received         {delivery_id, event, action}
webhook.signature_error  {source_ip}
delivery.persisted       {delivery_id}
delivery.claimed         {delivery_id, attempts}
dispatch.triggered       {workflow, head_sha, pr_number, trigger_source}
dispatch.skipped         {reason, head_sha, pr_number}
dispatch.failed          {error, head_sha}
ingest.started           {workflow_run_id}
ingest.bundle_fetched    {workflow_run_id, artifact_name}
ingest.platform_done     {workflow_run_id, platform, benchmarks}
ingest.completed         {workflow_run_id, platform_runs}
ingest.failed            {workflow_run_id, error}
check_run.upserted       {platform_run_id, check_run_id}
```

Optional v1 metrics if easy:

- pending deliveries,
- delivery retry count,
- ingest latency,
- dispatched runs,
- ingested platform runs.

If metrics are skipped, logs are still sufficient for v1 operations.

---

## Project Structure

Lives in `mobile-bench-rs`:

```text
services/mobench-webhook/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── router.rs
│   ├── worker.rs
│   ├── webhook/
│   │   ├── mod.rs
│   │   ├── verify.rs
│   │   ├── receive.rs
│   │   └── handlers.rs
│   ├── github/
│   │   ├── mod.rs
│   │   ├── auth.rs
│   │   ├── workflows.rs
│   │   ├── artifacts.rs
│   │   └── checks.rs
│   ├── ingest/
│   │   ├── mod.rs
│   │   ├── manifest.rs
│   │   ├── summary.rs
│   │   └── compare.rs
│   ├── api/
│   │   ├── mod.rs
│   │   ├── workflow_runs.rs
│   │   ├── platform_runs.rs
│   │   ├── trends.rs
│   │   └── compare.rs
│   └── db/
│       ├── mod.rs
│       ├── models.rs
│       ├── deliveries.rs
│       ├── dispatches.rs
│       ├── runs.rs
│       └── results.rs
├── migrations/
│   └── 001_initial.sql
└── tests/
    ├── webhook_delivery_test.rs
    ├── label_dispatch_test.rs
    ├── ingest_bundle_test.rs
    └── check_rerun_test.rs
```

---

## Configuration

```text
DATABASE_URL              = postgres://...
PUBLIC_HTTP_ADDR          = 0.0.0.0:8080
PRIVATE_HTTP_ADDR         = 0.0.0.0:8081
GITHUB_APP_ID             = 123456
GITHUB_APP_PRIVATE_KEY    = <PEM contents or file path>
GITHUB_WEBHOOK_SECRET     = <random secret>
GITHUB_INSTALLATION_ID    = 789012
GITHUB_OWNER              = worldcoin
GITHUB_REPO               = world-id-protocol
GITHUB_WORKFLOW_ID        = mobile-bench.yml
DELIVERY_RETRY_LIMIT      = 5
DELIVERY_CLAIM_TIMEOUT_SECS = 300
```

---

## Dependencies

| Crate | Purpose |
|---|---|
| `axum` | HTTP server |
| `sqlx` | Postgres and worker queue queries |
| `reqwest` | GitHub API calls |
| `jsonwebtoken` | GitHub App JWT generation |
| `hmac` + `sha2` | Webhook signature verification |
| `serde` + `serde_json` | Manifest and summary parsing |
| `tokio` | async runtime and worker loop |
| `uuid` | row ids |
| `tracing` + `tracing-subscriber` | logs |
| `wiremock` | GitHub API mocking in tests |

---

## Testing

- Unit tests:
  - signature verification,
  - manifest parsing,
  - summary parsing,
  - baseline comparison logic,
  - exact `workflow_inputs` replay.
- Integration tests:
  - persist delivery then acknowledge,
  - duplicate `X-GitHub-Delivery` handling,
  - label-trigger guard behavior for forks and duplicates,
  - ingest canonical bundle into workflow/platform/result tables,
  - create/update Check Runs after ingest,
  - `check_run.rerequested` dispatches stored inputs.
- Fixture strategy:
  - store one or more `mobench-history-v1` fixture bundles in test data,
  - mock GitHub artifact download with `wiremock`.

No full GitHub end-to-end test in CI is required for v1.

---

## GitHub App Blueprint

```json
{
  "name": "Mobench CI Automation",
  "url": "https://github.com/worldcoin/mobile-bench-rs",
  "description": "Dispatches mobench workflows, ingests benchmark history, and owns mobench check runs",
  "public": false,
  "hook_attributes": {
    "url": "https://<PUBLIC_WEBHOOK_DOMAIN>/webhook"
  },
  "default_permissions": {
    "actions": "write",
    "checks": "write",
    "contents": "read",
    "issues": "read",
    "metadata": "read",
    "pull_requests": "read"
  },
  "default_events": [
    "pull_request",
    "issue_comment",
    "workflow_run",
    "check_run"
  ]
}
```

Replace `<PUBLIC_WEBHOOK_DOMAIN>` with the public webhook ALB domain.

---

## Decisions Log

| Decision | Choice | Reasoning |
|---|---|---|
| Delivery durability | Persist webhook deliveries before ack | Prevents silent data loss on crash or deploy |
| Dispatch tracking | Persist dispatch rows separately from workflow runs | Enables dedupe before a workflow run exists and gives exact replay metadata |
| Ingestion source | Canonical `mobench-history-v1` bundle | Aligns ingest with a stable artifact contract instead of ad hoc workflow internals |
| Storage hierarchy | `workflow_run -> platform_run -> benchmark_result` | Matches current `platform=both` workflow fan-out |
| Check Run ownership | Webhook service owns Check Runs | Makes reruns deterministic and keeps GitHub state with the history system |
| Trigger defaults | Keep `/mobench` and `workflow_dispatch`; add label auto-trigger | Preserves current ergonomics while moving PR comment dispatch ownership into the GitHub App service |
| Baseline default | Latest successful run on `base_ref`, fallback `main` | Good default for PR comparisons without extra configuration |
| Query API ingress | Private only | Webhook must be public; query API should not be |
| Query API identity | Explicit `platform_run_id` for compare | Avoids ambiguity when one PR has multiple runs |

## Estimated Scope

- Service: medium microservice, likely 2,500-3,500 lines of Rust including tests
- Workflow changes: moderate, mostly summarize-job artifact materialization and removing workflow-owned Check Run creation
