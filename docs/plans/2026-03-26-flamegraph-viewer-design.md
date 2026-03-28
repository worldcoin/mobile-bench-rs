# Flamegraph Viewer Design

## Context

The current `mobench` flamegraph artifact is now technically correct enough to show the benchmark stack, but it is still a poor primary UX. The page opens as a raw `inferno` SVG with minimal controls, full-process noise dominates the default view, and common analysis tasks such as switching between benchmark-only and full-process stacks, brushing a range, and navigating zoom history are cumbersome.

This design changes the flamegraph artifact from "a standalone SVG file" into "a standalone flamegraph viewer page" while preserving the existing raw profiling outputs.

## Goals

- Make the default profiling artifact easy to navigate without external tooling.
- Default the user to a benchmark-focused view while preserving access to full-process data.
- Support toggling between benchmark-only and full-process flamegraphs in one artifact.
- Add explicit interaction affordances for click zoom, drag-to-select range zoom, reset, and zoom history.
- Keep the output standalone and local-file friendly.

## Non-Goals

- Rebuild flamegraph layout in the browser from raw stack data.
- Replace `inferno` with a custom canvas renderer.
- Remove or hide the existing raw artifacts such as `sample.txt`, `stacks.folded`, or `native-report.txt`.

## Recommended Approach

Generate a custom standalone HTML shell around two pre-rendered flamegraph SVGs:

- `Benchmark Only`
- `Full Process`

The HTML shell becomes the primary artifact (`flamegraph.html`) and provides the controls, summaries, and navigation state. The SVGs remain pre-rendered by `inferno`, which keeps generation deterministic and avoids moving layout logic into client-side JavaScript.

## Alternatives Considered

### Extend `inferno`'s emitted SVG in place

This is the lightest possible patch, but the embedded script is not structured for dual datasets, brush-zoom, or a real control surface. It would become brittle quickly.

### Replace the output with a custom canvas app

This has the highest interaction ceiling, but it is unnecessary for the current scope and would create a much larger maintenance burden.

## Product Shape

The generated viewer page should contain:

- A sticky toolbar with:
  - `Benchmark Only`
  - `Full Process`
  - `Back`
  - `Forward`
  - `Reset`
  - `Search`
- A summary strip showing:
  - current mode
  - current root frame
  - current selection width
  - visible sample count
- A details pane or summary section listing the hottest frames in the current mode and zoom root
- The current flamegraph visualization
- Links to supporting artifacts:
  - `native-report.txt`
  - `stacks.folded`
  - `benchmark.focused.folded`
  - `flamegraph.full.svg`
  - `flamegraph.focused.svg`

The default mode should be `Benchmark Only`.

## Architecture

The implementation has three layers.

### 1. Dataset generation

Keep generating the existing full folded stack dataset. Add a second derived folded dataset, `benchmark.focused.folded`, by trimming each stack down to the first benchmark anchor and everything below it.

### 2. Rendering

Render two SVG flamegraphs with `inferno`:

- full process
- benchmark focused

These become `flamegraph.full.svg` and `flamegraph.focused.svg`.

### 3. Viewer shell

Generate a standalone HTML page that embeds both SVGs, shows only one at a time, and layers our own navigation and summary UI on top.

## Benchmark-Focused Derivation

The benchmark-only dataset is produced by matching the first benchmark anchor in each stack and dropping all frames above that anchor.

Candidate anchors include:

- iOS:
  - `runBenchmark(spec:)`
  - `uniffi_*`
  - `sample_fns::run_benchmark`
  - `mobench_sdk::timing::run_closure`
- Android:
  - UniFFI/JNA entrypoints
  - Rust `run_benchmark`
  - `mobench_sdk::timing::run_closure`

The anchor list should be data-driven so new benchmark surfaces can be added without changing the renderer.

If a stack has no benchmark anchor, it is excluded from the focused dataset. If no stacks match at all, the viewer should surface that fact and degrade to the full-process view cleanly.

## Interaction Model

The viewer should support:

### Click zoom

Clicking a frame zooms into that frame.

### Brush zoom

Dragging horizontally across the graph selects an x-axis sample range. On mouse-up, that range expands to the full graph width.

### Navigation history

Every click zoom or brush zoom creates a history entry. The toolbar exposes:

- `Back`
- `Forward`
- `Reset`

Each mode should preserve its own history stack.

## Output Contract

Processed artifacts should expand to include:

- `stacks.folded`
- `benchmark.focused.folded`
- `native-report.txt`
- `flamegraph.full.svg`
- `flamegraph.focused.svg`
- `flamegraph.html`

The profile manifest should record both flamegraph datasets and the viewer artifact explicitly so summaries and downstream tooling can refer to them unambiguously.

## Failure Handling

- If the focused dataset is empty, the `Benchmark Only` tab should remain available but show a warning and allow a one-click switch to `Full Process`.
- If only one SVG renders successfully, the viewer should still load and surface the failure inline.
- If a brush selection is too small or resolves to zero samples, ignore it and keep the current view.

## Performance Constraints

- Keep flamegraph rendering static and generation-time only.
- Do not recompute flamegraph layout in browser JavaScript.
- The browser should manage mode switching, zoom state, history, and local summaries only.

## Testing Strategy

- Unit tests for benchmark-anchor trimming on representative Android and iOS folded stacks
- HTML generation tests for toolbar controls, dual-view shell, and artifact links
- Snapshot-style tests for `flamegraph.html` structure
- Smoke tests confirming:
  - `flamegraph.full.svg` exists
  - `flamegraph.focused.svg` exists
  - `flamegraph.html` references both
  - focused stacks contain benchmark frames such as `run_benchmark` and `fibonacci`

## Recommendation

Ship the viewer in two steps:

1. Benchmark-focused dataset derivation plus dual-view standalone HTML shell
2. Brush zoom, history controls, and hot-frame summaries

This keeps the first version grounded in the current pipeline while directly fixing the biggest UX problem: the default artifact should lead with the benchmark, not the entire process.
