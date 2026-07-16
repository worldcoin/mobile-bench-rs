# Mobench domain language

- **Run Specification** — the fully resolved benchmark request: function,
  counts, target, device selection, provider policy, and output policy.
- **Resolved Run Plan** — the command-level Run Specification after identity,
  expected Provider Sessions, counts, and report publication policy have been
  validated. It is the only state from which provider evidence may be
  collected.
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
- **Prepared Run** — validated provider evidence plus fully encoded canonical
  and compatibility reports that are eligible for atomic publication.
- **Committed Run** — the immutable publication and stable-alias receipt for a
  Prepared Run. A run is not externally complete until this receipt exists.
- **Run Outcome** — the command-level complete, partial, or failed result after
  successful bound reports are reconciled with every session in the Resolved
  Run Plan. It is distinct from one provider's Matrix Outcome because it also
  governs report preparation and publication.
- **Local Provider** — the provider that executes a benchmark through a local
  host harness and returns real samples without a remote device farm.
- **BrowserStack Provider** — the provider that uploads mobile artifacts,
  schedules remote device sessions, and collects their reports and telemetry.
- **Requested Device Identity** — the exact selector committed in the Resolved
  Run Plan. It remains stable even when a provider chooses a compatible minor
  OS release.
- **Observed Device Identity** — the device name and OS version authenticated by
  the provider's terminal build response. It is preserved alongside, never
  substituted for, the Requested Device Identity.
- **Report Binding** — the single transition that validates a producer envelope
  against the resolved run identity and then attaches authenticated provider
  run, transport-session, requested-device, and observed-device evidence.
