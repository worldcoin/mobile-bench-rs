# Mobench domain language

- **Run Specification** — the fully resolved benchmark request: function,
  counts, target, device selection, provider policy, and output policy.
- **Provider Run** — one execution of a Run Specification through a provider.
  It may contain one or many Provider Sessions.
- **Provider Session** — one provider attempt for one requested device.
- **Started Run** — the durable provider handle returned after scheduling and
  before terminal evidence has been collected.
- **Collected Session** — the reports and optional failure evidence attributed
  to one Provider Session.
- **Session Receipt** — the deterministic terminal assessment that says whether
  one expected Provider Session was complete, missing, non-passed, or
  result-less.
- **Matrix Outcome** — the complete, partial, or failed result obtained by
  reconciling every expected Provider Session with collected evidence.
- **Local Provider** — the provider that executes a benchmark through a local
  host harness and returns real samples without a remote device farm.
- **BrowserStack Provider** — the provider that uploads mobile artifacts,
  schedules remote device sessions, and collects their reports and telemetry.
