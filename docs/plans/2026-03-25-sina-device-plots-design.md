# Sina-Style Device Comparison Plots

## Summary

Add per-function device comparison plots to the final Mobench summary flow.
The Rust CLI remains the user-facing entry point, but delegates plot rendering
to a separate Python tool. Each plot compares devices for a single benchmark
function by showing one dot per iteration, with runtime on the y-axis and
device on the x-axis.

The result should increase information density without weakening the existing
CI output contract. `summary.json` remains stable. Plots are additive report
artifacts embedded into `summary.md`.

## Goals

- Keep the primary workflow inside the existing CLI/report path.
- Render one plot per benchmark function in the final summary section.
- Compare devices on a single chart with one point per raw iteration.
- Use deterministic point placement instead of random jitter or KDE sampling.
- Produce sleek, publication-friendly SVG output.
- Preserve the current `summary.json` contract.

## Non-Goals

- Replacing the current table and markdown summaries.
- Introducing a hard dependency on marimo notebooks.
- Changing the CI v1 required output contract.
- Requiring plots to exist when only aggregate statistics are available.

## User Experience

The common flow stays the same:

```bash
cargo mobench ci run ...
cargo mobench report summarize --summary target/mobench/ci/summary.json
```

During the final summarize/report step, Mobench attempts to generate plots for
each benchmark function that has raw per-iteration data across devices. The
report embeds those plots in the same section as the summary table.

Plot generation mode should be configurable:

- `auto`: generate plots when the Python renderer and raw samples are available
- `off`: skip plotting entirely
- `require`: fail the summarize/report step if plots cannot be generated

`auto` should be the default.

## Architecture

### Rust CLI

Rust remains responsible for:

- locating and loading result artifacts
- understanding Mobench summary/result layouts
- normalizing plot input per function and device
- invoking the Python renderer
- embedding generated plot paths into the markdown report

Rust should not implement the visual rendering logic directly.

### Python Renderer

The Python renderer is a standalone script checked into the repository and
called by the CLI. It should:

- read a normalized plot payload from JSON
- render one SVG figure for one benchmark function
- implement deterministic horizontal point packing
- apply a custom Matplotlib style tuned for clean static output

This keeps the Rust side focused on artifact orchestration and keeps the
plotting side easy to iterate on visually.

## Artifacts

Required contract outputs remain unchanged:

- `summary.json`
- `summary.md`
- `results.csv`

Additive plot artifacts should live under:

```text
target/mobench/ci/plots/
```

Example outputs:

```text
target/mobench/ci/plots/nullifier-proof-generation.svg
target/mobench/ci/plots/query-proof-generation.svg
target/mobench/ci/plots/manifest.json
```

`manifest.json` is optional but recommended as an implementation detail. Rust
can emit a slim plotting payload to avoid duplicating Mobench artifact parsing
logic in Python.

## Data Flow

1. `ci run` produces the usual result artifacts.
2. The final summarize/report step loads the summary and richer raw sample
   sources already produced by earlier steps.
3. Rust groups raw samples by function and device.
4. Rust writes one normalized plot payload per function, or one shared manifest
   with one entry per function.
5. Rust invokes the Python renderer once per function.
6. The renderer writes SVGs into `plots/`.
7. The markdown summary embeds the SVGs in the corresponding function section.

The plotting feature depends on richer `benchmark_results` data when available.
If only aggregate stats exist, the function remains table-only.

## Plot Layout

Each figure represents a single benchmark function.

- x-axis: device
- y-axis: runtime in milliseconds
- one dot: one iteration sample

Each device gets a vertical strip of samples centered on the device position.
To avoid overlap, dots are packed horizontally around the device centerline.

The report should render separate plots for multiple functions, all inside the
same summary section.

## Point Placement Algorithm

Use a deterministic local packing algorithm rather than KDE-based Sina jitter.

For each device:

1. Sort samples by y value.
2. Convert each sample to plot coordinates.
3. For the next dot, search horizontal offsets in symmetric order around zero.
4. Choose the smallest `delta_x` that keeps the new dot center at least
   `epsilon` away from all previously placed dots in the same device strip.
5. Clamp the maximum allowed width so one dense device does not dominate the
   figure layout.

Where:

- `epsilon` = dot diameter + visual gap
- search order is symmetric, for example `0, +d, -d, +2d, -2d, ...`

This approach is:

- deterministic
- simpler than KDE-based placement
- faithful to the raw samples
- visually centered and compact

The implementation is best described as a Sina/beeswarm-style plot with
deterministic collision packing.

## Visual Style

The styling should aim for a clean, polished static look without requiring
marimo itself.

Important note: marimo does not appear to use a special Matplotlib beautifier.
Its Matplotlib integration mainly provides a rendering backend and applies
Matplotlib's built-in `dark_background` style in dark mode. We should not add a
marimo dependency for this feature.

Instead, ship a custom Matplotlib style layer for the renderer:

- SVG output by default
- light theme by default
- restrained typography and spacing
- thin horizontal gridlines
- compact margins
- muted per-device colors or a single neutral dot color
- subtle median marker per device
- humanized benchmark titles
- consistent figure sizing across plots

The style should live in a small dedicated module or `.mplstyle` file so it is
usable outside a notebook environment.

## Markdown Embedding

The markdown summary should keep the existing table, then add the plot directly
below it when present.

Preferred embedding uses relative links so artifacts survive CI upload and
download together:

```md
![nullifier-proof-generation](plots/nullifier-proof-generation.svg)
```

If a function has no plot, no placeholder is needed.

## Failure Handling

- If raw samples are missing for a function, skip that plot.
- If some devices for a function have raw samples and others do not, render the
  plot with the complete devices only and emit a warning.
- In `auto` mode, missing Python or renderer failures should not fail the
  overall report.
- In `require` mode, plot generation failures should fail the step.
- Output should be deterministic for stable CI diffs.

## Testing Strategy

### Rust

- unit tests for extracting raw per-device per-function samples from current
  result layouts
- tests for manifest generation
- tests for markdown embedding and relative path handling
- fixture-based tests covering mixed availability of raw and aggregate data

### Python

- unit tests for the collision-packing algorithm
- invariant tests that verify minimum distance constraints
- determinism tests using fixed input data
- smoke tests that generate SVG output for representative inputs

### End-to-End

- one fixture-driven summarize/report test that confirms plots are emitted and
  linked into markdown when raw sample data is available

## Rollout Notes

- Make plots additive and optional first.
- Keep the current text/table report fully useful without plots.
- Prefer a minimal renderer dependency surface, ideally `matplotlib` and
  standard library only unless a narrow extra package materially improves the
  result.

## Accepted Decisions

- Rust CLI entry point, Python renderer underneath
- one plot per benchmark function
- separate plots embedded in the final summary section
- compare devices on one graph
- point-per-iteration visualization
- deterministic collision-based horizontal packing
- custom Matplotlib styling rather than marimo notebook dependency
