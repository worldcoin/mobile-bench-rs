# Codebase Reference

Updated: 2026-07-17. Current release candidate: `0.1.45`.

These notes describe the current repository structure, implementation
boundaries, integration points, and release-sensitive public surface. For user
workflows, start with [../guides/README.md](../guides/README.md). For the full
behavior contract, see [../specs/mobench-current-spec.md](../specs/mobench-current-spec.md).

- [ARCHITECTURE.md](ARCHITECTURE.md): how the CLI, SDK, generated runners,
  BrowserStack flow, reporting, and profiling fit together.
- [PUBLIC_API.md](PUBLIC_API.md): public Rust APIs, CLI/API contracts,
  feature flags, MSRV, semver boundaries, and release checks.
- [STRUCTURE.md](STRUCTURE.md): important crates, templates, docs, workflows,
  generated outputs, and where new work belongs.
- [STACK.md](STACK.md): languages, Rust crates, native toolchains, services,
  and artifact types.
- [INTEGRATIONS.md](INTEGRATIONS.md): BrowserStack, GitHub Actions, Android,
  iOS, generated runner backends, and local native profiling.
- [../guides/reusable-workflow-security.md](../guides/reusable-workflow-security.md):
  reusable workflow threat model, trust boundary, permissions, and migration.
- [CONVENTIONS.md](CONVENTIONS.md): naming, output, config, docs, template,
  and error-handling conventions.
- [TESTING.md](TESTING.md): host tests, tool-gated checks, service-gated checks,
  fixture validation, CI contract checks, and profiling smoke tests.
