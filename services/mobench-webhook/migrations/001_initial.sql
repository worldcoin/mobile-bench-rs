CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE github_webhook_deliveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    delivery_id TEXT NOT NULL UNIQUE,
    event TEXT NOT NULL,
    action TEXT,
    payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ,
    processed_at TIMESTAMPTZ
);

CREATE TABLE benchmark_dispatches (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dispatch_id UUID NOT NULL UNIQUE,
    repo_owner TEXT NOT NULL,
    repo_name TEXT NOT NULL,
    head_sha TEXT NOT NULL,
    head_ref TEXT NOT NULL,
    pr_number INTEGER,
    trigger_source TEXT NOT NULL,
    requested_by TEXT,
    workflow_inputs JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    workflow_run_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);

CREATE TABLE benchmark_workflow_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_run_id BIGINT NOT NULL UNIQUE,
    workflow_run_attempt INTEGER NOT NULL,
    repo_owner TEXT NOT NULL,
    repo_name TEXT NOT NULL,
    workflow_name TEXT NOT NULL,
    head_sha TEXT NOT NULL,
    head_ref TEXT NOT NULL,
    base_ref TEXT,
    pr_number INTEGER,
    trigger_source TEXT NOT NULL,
    requested_by TEXT,
    request_command TEXT,
    mobench_version TEXT,
    mobench_ref TEXT,
    conclusion TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);

CREATE TABLE benchmark_platform_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_run_uuid UUID NOT NULL REFERENCES benchmark_workflow_runs(id),
    platform TEXT NOT NULL,
    check_run_id BIGINT UNIQUE,
    check_run_name TEXT NOT NULL,
    workflow_inputs JSONB NOT NULL,
    device_profile TEXT,
    device_name TEXT NOT NULL,
    os_version TEXT NOT NULL,
    iterations INTEGER NOT NULL,
    warmup INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'completed',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    UNIQUE (workflow_run_uuid, platform)
);

CREATE TABLE benchmark_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    platform_run_uuid UUID NOT NULL REFERENCES benchmark_platform_runs(id),
    function_name TEXT NOT NULL,
    function_label TEXT NOT NULL,
    avg_ms DOUBLE PRECISION NOT NULL,
    median_ms DOUBLE PRECISION,
    p95_ms DOUBLE PRECISION,
    best_ms DOUBLE PRECISION NOT NULL,
    worst_ms DOUBLE PRECISION NOT NULL,
    std_dev_ms DOUBLE PRECISION,
    cpu_avg_percent DOUBLE PRECISION,
    cpu_peak_percent DOUBLE PRECISION,
    ram_avg_mb DOUBLE PRECISION,
    ram_peak_mb DOUBLE PRECISION,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
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
