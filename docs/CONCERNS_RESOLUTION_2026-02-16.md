# Concerns Resolution (2026-02-16)

This document records the disposition of concerns previously tracked in the old codebase-planning backlog artifact that lived under `.planning/codebase/`.

Disposition labels:
- `fixed`: implemented in code on `codex/ci-devex`
- `accepted`: reviewed and explicitly accepted as non-blocking for current release scope
- `deferred`: valid concern, intentionally deferred to follow-up engineering work

## Tech Debt

- Large monolithic build modules (`ios.rs`, `android.rs`): `deferred`
- Unwrap calls in production code paths (`ios.rs` packaging flows): `fixed`
- Path canonicalization fallback in `IosBuilder::new`: `fixed` (now emits explicit warning on fallback)
- String parsing of cargo metadata target dir (`common.rs`): `fixed` (now parses JSON payload)
- Generated code validation before use: `deferred`

## Known Bugs

- iOS XCUITest packaging with complex paths/spaces: `fixed` (OsStr-safe command args)
- Cargo metadata fallback behavior in workspaces: `fixed` (JSON parse path; clearer fallback behavior)
- Template variable name collision (`sample_fns`): `deferred`

## Security Considerations

- BrowserStack credentials in verbose output: `accepted` (current code logs command invocations, not secret env values)
- User path validation / traversal guardrails: `deferred`
- ZIP/command execution with unchecked path length/characters: `deferred`
- Secret redaction wrapper types for config/env credentials: `deferred`

## Performance Bottlenecks

- Sequential multi-target compilation: `deferred`
- Cargo metadata invocation on every build (no cache): `deferred`
- Manual xcframework directory construction overhead: `deferred`

## Fragile Areas

- Manual xcframework plist generation: `deferred`
- String-substitution template rendering: `deferred`
- Gradle artifact validation after build: `deferred`
- CLI inter-argument validation coverage: `deferred`

## Scaling Limits

- APK size and BrowserStack upload limit behavior: `accepted` (documented operational constraint)
- Large benchmark registry scaling profile: `deferred`
- Large device matrix/session limit handling: `deferred`

## Dependencies at Risk

- Rustls backend feature fragility: `accepted` (monitoring required)
- UniFFI version lock consistency: `deferred`
- Embedded template updatability via `include_dir`: `accepted`

## Missing Features

- Resume/restart after partial build failure: `deferred`
- Artifact correctness validation beyond build success: `deferred`
- Incremental rebuild strategy: `deferred`
- Android compatibility matrix validation guidance: `deferred`

## Test Coverage Gaps

- Builder error-path tests: `deferred`
- Template rendering edge-case tests: `deferred`
- BrowserStack end-to-end integration tests: `deferred`
- CLI invalid-combination tests: `deferred`
- Cross-platform path handling tests: `deferred`

## Notes

- This file replaces the previous codebase-planning concerns backlog artifact.
- Resolved items in this pass include safer iOS packaging command construction, cargo metadata JSON parsing, and canonicalization fallback visibility.
