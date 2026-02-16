# ADR 0001: Mobench CI Contract v1

- Date: 2026-02-16
- Status: Accepted
- Decision owners: Engineering / Mobench

## Context

Mobench CI integrations need a stable interface for inputs, outputs, and error categories so that workflows and PR reporting do not break across iterative CLI improvements.

## Decision

Adopt `v1` CI contract documented in `docs/CONTRACT_CI_V1.md` and versioned schemas under `docs/schemas/`.

### Scope boundaries

Included:
- `cargo mobench ci run` contract
- output files and metadata fields
- error taxonomy categories for CI-oriented validation

Excluded (non-goals):
- provider-specific API payloads
- dashboard/event streaming protocols
- default threshold gating policy changes

### Versioning strategy

- Contract artifacts are versioned as `vN`.
- Output schema and required metadata are append-only within a major contract version.
- Any removal/rename/type-breaking change requires a new contract version.

### Action interface versioning

- GitHub Action references use semantic tags plus immutable SHAs.
- Repository-local action examples must map to the same required output contract.

### Deprecation policy

- When introducing a successor to `v1`, keep `v1` compatibility for at least one minor release window.
- Mark deprecated fields/artifacts in docs before removal.

### Reporting defaults

- v1 default reporting mode is descriptive-only.
- Threshold gating remains explicit and opt-in.

### Baseline default (v1.1 planning)

- Default baseline source is previous successful run, with pinned artifacts supported explicitly.

### Minimum supported CI environments/toolchains

- Linux: Ubuntu latest runner with Rust stable.
- macOS: macOS latest runner with Rust stable.
- Required Rust targets are documented in workflow templates.

## Consequences

Positive:
- Integrators get stable artifact paths and metadata contract.
- CI tooling can rely on fixed machine-readable schema files.

Tradeoffs:
- Requires explicit version bumps for contract evolution.
- Maintainers must update schema/tests/docs together.

## Links

- Contract: `docs/CONTRACT_CI_V1.md`
- Schemas: `docs/schemas/summary-v1.schema.json`, `docs/schemas/ci-contract-v1.schema.json`
- Migration guide placeholder: `docs/MIGRATION_GUIDE.md`
