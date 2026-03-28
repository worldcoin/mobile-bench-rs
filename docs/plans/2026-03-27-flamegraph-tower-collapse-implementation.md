# Flamegraph Tower Collapse Implementation Plan

Status: not shipped. The tower-collapse implementation was rolled back; keep this file only as historical implementation context, not as the current viewer behavior.

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the flamegraph viewer hide tall thin low-self-time towers by default, show `+` expand affordances for those ranges, and let users zoom into the hidden range to inspect it.

**Architecture:** Keep the collapse logic in the viewer layer. Analyze rendered flamegraph frames from SVG coordinates at the current zoom level, compute low-self-time tower candidates, hide them from the default graph height, and inject expand chips that zoom into the hidden range. Preserve raw folded-stack and SVG artifacts exactly as they are today.

**Tech Stack:** Rust, inferno-generated SVG flamegraphs, embedded viewer HTML/JS in `crates/mobench/src/flamegraph_viewer.rs`, Rust unit tests, local CLI smoke runs

---

### Task 1: Freeze The New Viewer Behavior With Tests

**Files:**
- Modify: `/Users/dcbuilder/Code/world/mobile-bench-rs/.worktrees/profile-browserstack-honesty/crates/mobench/src/flamegraph_viewer.rs`

**Step 1: Add failing viewer-shell tests**

Add tests that assert the generated viewer HTML includes:

- a `Hide Thin Towers` control
- expand-chip support
- tower-collapse metadata/hooks for the embedded graph

**Step 2: Add a failing tower-detector test**

Add a small synthetic flamegraph tree test that proves:

- a deep thin low-self-time chain is detected as collapsible
- a durable wide branch is not collapsed

**Step 3: Run the focused tests**

Run:

```bash
cargo test -p mobench flamegraph_viewer -- --nocapture
```

Expected:

- new tests fail before implementation

**Step 4: Commit**

```bash
git add crates/mobench/src/flamegraph_viewer.rs
git commit -m "test: freeze flamegraph tower collapse behavior"
```

### Task 2: Add Tower Analysis And Collapse Metadata

**Files:**
- Modify: `/Users/dcbuilder/Code/world/mobile-bench-rs/.worktrees/profile-browserstack-honesty/crates/mobench/src/flamegraph_viewer.rs`

**Step 1: Add SVG-frame analysis structures**

Implement internal structs/functions to represent:

- frame bounds
- depth
- parent/child coverage
- computed `self_samples`
- collapsible tower groups

**Step 2: Implement the collapse heuristic**

Use these default thresholds:

- width `< 18px`
- self time `< 0.5%`
- tower depth `>= 6`

The detector should return grouped tower ranges, not just individual frames.

**Step 3: Add unit tests for heuristic edge cases**

Cover:

- tower just below threshold collapses
- wide branch does not collapse
- shallow tower does not collapse
- zoomed wider range no longer collapses

**Step 4: Run tests**

Run:

```bash
cargo test -p mobench flamegraph_viewer -- --nocapture
```

Expected:

- tower-analysis tests pass

**Step 5: Commit**

```bash
git add crates/mobench/src/flamegraph_viewer.rs
git commit -m "feat: detect low-self-time flamegraph towers"
```

### Task 3: Hide Towers And Add Expand Chips In The Embedded SVG

**Files:**
- Modify: `/Users/dcbuilder/Code/world/mobile-bench-rs/.worktrees/profile-browserstack-honesty/crates/mobench/src/flamegraph_viewer.rs`

**Step 1: Extend the standalone SVG helper script**

Add helper functions that:

- inspect current visible frames
- mark collapsible towers
- hide their `<g>` groups
- inject `+` chip overlays at the tower x-ranges
- recompute the visible SVG height after collapse

**Step 2: Wire chip clicks to range zoom**

Clicking a chip should:

- call the existing zoom helper with that tower’s x-range
- rerun collapse analysis after zoom

**Step 3: Preserve existing interactions**

Ensure these still work:

- click zoom
- brush zoom
- back
- forward
- reset
- search

**Step 4: Run tests**

Run:

```bash
cargo test -p mobench flamegraph_viewer -- --nocapture
```

Expected:

- SVG/viewer tests pass

**Step 5: Commit**

```bash
git add crates/mobench/src/flamegraph_viewer.rs
git commit -m "feat: collapse thin flamegraph towers by default"
```

### Task 4: Add Viewer Controls And State For Tower Hiding

**Files:**
- Modify: `/Users/dcbuilder/Code/world/mobile-bench-rs/.worktrees/profile-browserstack-honesty/crates/mobench/src/flamegraph_viewer.rs`

**Step 1: Add the toolbar toggle**

Add `Hide Thin Towers` to the viewer toolbar, default `on`.

**Step 2: Add threshold/status display**

Surface:

- active threshold values
- number of collapsed towers or hidden frames when available

**Step 3: Maintain per-mode state**

Ensure `Benchmark Only` and `Full Process` retain independent zoom/collapse history.

**Step 4: Add HTML regression tests**

Assert the viewer HTML contains:

- the toggle
- threshold display
- collapse-status placeholders

**Step 5: Run tests**

Run:

```bash
cargo test -p mobench flamegraph_viewer -- --nocapture
```

Expected:

- viewer-shell tests pass

**Step 6: Commit**

```bash
git add crates/mobench/src/flamegraph_viewer.rs
git commit -m "feat: add flamegraph tower collapse controls"
```

### Task 5: Verify With A Real iOS Profile Artifact

**Files:**
- Modify if needed: `/Users/dcbuilder/Code/world/mobile-bench-rs/.worktrees/profile-browserstack-honesty/crates/mobench/src/flamegraph_viewer.rs`

**Step 1: Regenerate the iOS smoke artifact**

Run:

```bash
cargo run -p mobench --bin mobench -- profile run --target ios --provider local --backend ios-instruments --crate-path crates/sample-fns --function sample_fns::fibonacci --output-dir target/mobench/profile-smoke-ios
```

Expected:

- fresh artifacts under `target/mobench/profile-smoke-ios/ios-sample_fns--fibonacci/artifacts/processed`

**Step 2: Verify collapse behavior in the generated viewer**

Check:

- the initial graph height is materially shorter
- `+` expand chips appear
- clicking a chip zooms to a readable tower range
- `Back` and `Reset` restore the prior/default view

**Step 3: Re-run relevant tests**

Run:

```bash
cargo test -p mobench flamegraph_viewer -- --nocapture
cargo test -p mobench profile_ -- --nocapture
```

Expected:

- all tests pass

**Step 4: Commit**

```bash
git add crates/mobench/src/flamegraph_viewer.rs
git commit -m "feat: verify flamegraph tower collapse on iOS artifact"
```

### Task 6: Update Docs If Viewer Semantics Changed

**Files:**
- Modify if needed: `/Users/dcbuilder/Code/world/mobile-bench-rs/.worktrees/profile-browserstack-honesty/README.md`

**Step 1: Document the new viewer affordance**

Add a short note that:

- the interactive viewer hides tall thin low-self-time towers by default
- `+` chips expand hidden tower ranges
- `Reset` returns to the durable-only view

**Step 2: Verify docs**

Run:

```bash
cargo run -p mobench --bin mobench -- profile run --help
```

Expected:

- help remains accurate

**Step 3: Commit**

```bash
git add README.md
git commit -m "docs: describe flamegraph tower collapse viewer"
```
