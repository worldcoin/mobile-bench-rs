# Conventions

Updated: 2026-06-29. Release line: `0.1.42`.

## Naming

- Rust files, modules, functions, and variables: `snake_case`.
- Public types and enum variants: `PascalCase`.
- Constants and environment variables: `SCREAMING_SNAKE_CASE`.
- Serialized enum values: lowercase or kebab-case when already shipped.
- Artifact labels in manifests: short descriptive kebab-case strings.
- Generated runner backend values: `uniffi`, `native-c-abi`.

## Output Conventions

- Default generated output root: `target/mobench/`.
- BrowserStack fetch output root: `target/browserstack/`.
- Benchmark CI outputs: `summary.json`, `summary.md`, `results.csv`, optional
  `plots/*.svg`.
- Resource columns: `cpu_total_ms`, `cpu_median_ms`, `peak_memory_kb`,
  `peak_memory_growth_kb`, `process_peak_memory_kb`.
- Profile outputs are run-scoped and mirrored to latest-run convenience paths.
- Processed profile artifact names:
  - `stacks.folded`
  - `native-report.txt`
  - `frame-locations.json` when Android symbolization resolves file/line data
  - `flamegraph.full.svg`
  - `flamegraph.focused.svg`
  - `flamegraph.html`
- Differential profile outputs live under `target/mobench/profile/diff/` and
  include `profile-diff.json`, `summary.md`, and differential folded stacks.

## Config Conventions

Project resolution order:

1. Explicit CLI flags.
2. Explicit config path.
3. Discovered `mobench.toml`.
4. Cargo workspace metadata.
5. Git root.
6. Legacy fallback paths.

Device resolution should reuse the shared CLI surface:

- `--device`
- `--os-version`
- `--profile`
- `--device-matrix`
- `--device-tags`

Generated runner backend selection belongs in `[project].ffi_backend`.

## Documentation Conventions

- Public Rust items should document user-facing behavior with `//!` or `///`.
- Comments should explain why a branch or tooling workaround exists.
- README and guide examples should describe current shipped behavior.
- Avoid linking to removed migration/spec docs from current docs.
- Keep Mermaid sources in `docs/diagrams/` mirrored with README Mermaid blocks.

## Template Editing

- Edit `templates/` first.
- Mirror changes into `crates/mobench-sdk/templates/`.
- Keep Android and iOS runner JSON/log markers aligned so CLI parsers remain
  cross-platform.
- Update docs when template inputs, output paths, or generated backend behavior
  changes.

## Error Handling Style

- SDK errors should be typed where possible.
- CLI errors should include actionable context.
- Unsupported provider/backend combinations should fail explicitly.
- Do not silently downgrade BrowserStack native profiling to timing-only
  behavior.
