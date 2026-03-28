# Flamegraph Tower Collapse Design

## Goal

Make the flamegraph viewer readable by default when a profile contains tall, thin towers of low-self-time frames. The viewer should hide those towers initially, show compact `+` expand affordances at their x-ranges, and let the user zoom into them on demand.

## Problem

The current flamegraph viewer renders every frame at full depth. For profiles with narrow, mostly vertical branches, that produces a graph that is too tall, forces long scrolling, and makes the durable branches hard to compare. Even when the benchmark-only capture is good, the default presentation still overemphasizes thin call towers that contribute little self time.

## Chosen Approach

Use a viewer-layer collapse pass driven by `self time`, not a preprocessing pass that mutates the stored folded-stack artifacts.

The viewer already has:

- dual modes: `Benchmark Only` and `Full Process`
- zoom history
- brush range selection
- SVG helper functions inside the embedded flamegraph document

The new behavior should extend that existing interaction model:

1. Analyze the rendered SVG frames in the current zoomed view.
2. Detect contiguous vertical towers that are both visually thin and low in `self time`.
3. Hide those towers from the default graph layout.
4. Render a `+` chip at each hidden tower’s x-range.
5. Clicking the chip zooms that tower range to full width and reveals the hidden frames for that region.

## Why This Approach

This preserves the raw outputs:

- `stacks.folded`
- `benchmark.focused.folded`
- `flamegraph.full.svg`
- `flamegraph.focused.svg`

It also makes the behavior dynamic per zoom level. A tower that is too thin in the default view may become readable after zooming; the collapse logic should therefore rerun after every zoom instead of baking the collapse into the artifact generation pipeline.

## Collapse Heuristic

Base the heuristic on `self time`.

Recommended defaults:

- collapse a tower when frame width in the current view is below about `18px`
- and frame self time is below about `0.5%` of visible samples
- and the tower depth is at least `6` frames

The detector should:

- reconstruct parent/child relationships from the flamegraph SVG coordinates (`fg:x`, `fg:w`, `y`)
- estimate each frame’s `self_samples` from inclusive width minus direct child coverage
- identify mostly vertical, low-self-time chains
- group each chain into one collapsed tower rooted at the first durable ancestor

## Interaction Model

Default behavior:

- `Hide Thin Towers` is enabled by default
- hidden towers do not contribute to graph height
- the graph height is cropped to the highest remaining visible frame

Expansion behavior:

- each hidden tower renders as a compact `+ N` chip
- clicking the chip zooms into that tower’s x-range
- after zoom, the collapse pass reruns for the new viewport
- `Back` returns to the prior durable-only view
- `Reset` returns to the default durable-only view for the current mode

Mode behavior:

- `Benchmark Only` and `Full Process` keep independent zoom/collapse history
- the collapse toggle state is shared unless later evidence shows mode-specific defaults are better

## UI Additions

Add to the toolbar:

- `Hide Thin Towers` toggle, default `on`

Add to the sidebar or toolbar metadata:

- the active thresholds:
  - `self < 0.5%`
  - `width < 18px`

The graph itself should show:

- `+` chips positioned at hidden tower x-ranges
- hidden tower depth or hidden frame count in the chip label if space allows

## Architecture

Keep this feature inside the existing flamegraph viewer layer in:

- `/Users/dcbuilder/Code/world/mobile-bench-rs/.worktrees/profile-browserstack-honesty/crates/mobench/src/flamegraph_viewer.rs`

Do not introduce a new persisted artifact format.

Implementation should likely touch:

- the generated viewer HTML shell
- the embedded helper script inside the standalone SVG document
- viewer tests for HTML generation and tower-detection behavior

## Testing

Add:

- unit tests for tower detection on a synthetic flamegraph tree
- HTML regression tests for:
  - `Hide Thin Towers` toggle
  - `+` expand chip support
  - per-view collapse metadata
- smoke verification on the iOS sample artifact to confirm:
  - initial graph height is shorter
  - collapsed towers render with `+` affordances
  - expanding a tower zooms to a legible range

## Non-Goals

- do not rewrite folded-stack artifacts on disk
- do not remove access to raw full-process detail
- do not rely on `inclusive time` as the primary collapse criterion

