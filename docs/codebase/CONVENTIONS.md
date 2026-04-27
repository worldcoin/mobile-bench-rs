# Conventions

Updated: 2026-04-01

## Naming

- Rust files, modules, functions, and variables: `snake_case`
- public types and enum variants: `PascalCase`
- constants and environment variable names: `SCREAMING_SNAKE_CASE`
- artifact labels in manifests: kebab-case or short descriptive strings

## Output conventions

- default generated output root: `target/mobench/`
- benchmark CI outputs: `summary.json`, `summary.md`, `results.csv`
- profile outputs are run-scoped and also mirrored to latest-run convenience copies
- processed profile artifacts keep stable names:
  - `stacks.folded`
  - `native-report.txt`
  - `frame-locations.json` when Android symbolization resolves file/line metadata
  - `flamegraph.full.svg`
  - `flamegraph.focused.svg`
  - `flamegraph.html`
- differential profile outputs live under `target/mobench/profile/diff/` and use:
  - `profile-diff.json`
  - `summary.md`
  - `diff.full.folded`
  - `diff.focused.folded`

## Config conventions

Resolution order stays consistent across build/run/profile commands:

1. explicit CLI flags
2. explicit config path
3. discovered `mobench.toml`
4. workspace root
5. git root
6. legacy fallback paths

Device resolution semantics should reuse the same surface area instead of inventing command-specific flags:
- `--device`
- `--os-version`
- `--profile`
- `--device-matrix`

## Documentation and comments

- public Rust items should use `//!` or `///` comments when they define user-facing behavior
- comments should explain why a branch/tooling workaround exists, not restate obvious code
- README and template docs should describe current shipped behavior, not superseded MVP constraints

## Template editing

- edit `templates/` and the mirrored `crates/mobench-sdk/templates/` copy together
- keep Android and iOS runner JSON/log markers aligned so CLI parsers stay cross-platform
- when SDK report structures change, regenerate or refresh template/runtime bindings in the same change

## Error handling style

- SDK surfaces typed errors via `thiserror`
- CLI orchestration adds context with `anyhow`
- unsupported provider/backend combinations must fail explicitly and describe the supported alternative
