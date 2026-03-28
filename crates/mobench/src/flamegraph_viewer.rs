use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FocusedFoldedStacks {
    pub folded: String,
    pub matched_stack_count: usize,
    pub excluded_stack_count: usize,
    pub included_samples: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FrameBreakdown {
    pub frame: String,
    pub samples: u64,
    pub percent_total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ModeSummary {
    pub total_samples: u64,
    pub visible_stack_count: usize,
    pub matched_stack_count: usize,
    pub excluded_stack_count: usize,
    pub warning: Option<String>,
    pub self_frames: Vec<FrameBreakdown>,
    pub inclusive_frames: Vec<FrameBreakdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ArtifactLink {
    pub label: String,
    pub path: String,
}

impl ArtifactLink {
    pub(crate) fn new(label: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlamegraphMode {
    Focused,
    Full,
}

impl FlamegraphMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Focused => "focused",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FlamegraphViewerDoc {
    pub title: String,
    pub full_svg_document: String,
    pub focused_svg_document: String,
    pub full_summary: ModeSummary,
    pub focused_summary: ModeSummary,
    pub default_mode: FlamegraphMode,
    pub artifact_links: Vec<ArtifactLink>,
}

pub(crate) fn derive_benchmark_focused_folded_stacks(
    folded: &str,
    anchors: &[&str],
) -> FocusedFoldedStacks {
    let mut lines = Vec::new();
    let mut matched_stack_count = 0_usize;
    let mut excluded_stack_count = 0_usize;
    let mut included_samples = 0_u64;

    for line in folded.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((stack, count)) = split_folded_stack_line(trimmed) else {
            excluded_stack_count += 1;
            continue;
        };
        let frames: Vec<&str> = stack.split(';').collect();
        let Some(trimmed_frames) = trim_stack_to_first_anchor(&frames, anchors) else {
            excluded_stack_count += 1;
            continue;
        };
        matched_stack_count += 1;
        included_samples += count;
        lines.push(format!("{} {}", trimmed_frames.join(";"), count));
    }

    FocusedFoldedStacks {
        folded: lines.join("\n"),
        matched_stack_count,
        excluded_stack_count,
        included_samples,
    }
}

pub(crate) fn summarize_folded_stacks(
    folded: &str,
    matched_stack_count: usize,
    excluded_stack_count: usize,
    warning: Option<String>,
) -> ModeSummary {
    let mut total_samples = 0_u64;
    let mut visible_stack_count = 0_usize;
    let mut inclusive_samples: BTreeMap<String, u64> = BTreeMap::new();
    let mut self_samples: BTreeMap<String, u64> = BTreeMap::new();

    for line in folded.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((stack, count)) = split_folded_stack_line(trimmed) else {
            continue;
        };
        let frames: Vec<String> = stack.split(';').map(prettify_frame_label).collect();
        visible_stack_count += 1;
        total_samples += count;
        let mut seen_frames = BTreeSet::new();
        for frame in &frames {
            if seen_frames.insert(frame.clone()) {
                *inclusive_samples.entry(frame.clone()).or_default() += count;
            }
        }
        if let Some(leaf_frame) = frames.last() {
            *self_samples.entry(leaf_frame.clone()).or_default() += count;
        }
    }

    ModeSummary {
        total_samples,
        visible_stack_count,
        matched_stack_count,
        excluded_stack_count,
        warning,
        self_frames: build_frame_breakdown_list(self_samples, total_samples),
        inclusive_frames: build_frame_breakdown_list(inclusive_samples, total_samples),
    }
}

pub(crate) fn count_folded_stack_lines(folded: &str) -> usize {
    folded.lines().filter(|line| !line.trim().is_empty()).count()
}

pub(crate) fn render_standalone_flamegraph_svg(
    folded_stacks: &str,
    title: &str,
) -> Result<String> {
    if folded_stacks.trim().is_empty() {
        return Ok("<!DOCTYPE html><html><body><p>No native frames were symbolized.</p></body></html>"
            .into());
    }

    let mut options = inferno::flamegraph::Options::default();
    options.title = title.into();
    let mut rendered = Vec::new();
    let display_stacks = prettify_folded_stacks_for_display(folded_stacks);
    inferno::flamegraph::from_reader(
        &mut options,
        Cursor::new(display_stacks.as_bytes()),
        &mut rendered,
    )?;
    let rendered = String::from_utf8(rendered).context("inferno produced non-UTF-8 flamegraph")?;
    Ok(finalize_standalone_flamegraph_document(rendered))
}

pub(crate) fn render_flamegraph_viewer_html(doc: FlamegraphViewerDoc) -> String {
    let default_mode = doc.default_mode.as_str();
    let full_svg = escape_json_for_inline_script(
        &serde_json::to_string(&doc.full_svg_document).expect("serialize full svg"),
    );
    let focused_svg = escape_json_for_inline_script(
        &serde_json::to_string(&doc.focused_svg_document).expect("serialize focused svg"),
    );
    let full_summary = escape_json_for_inline_script(
        &serde_json::to_string(&doc.full_summary).expect("serialize full mode summary"),
    );
    let focused_summary = escape_json_for_inline_script(
        &serde_json::to_string(&doc.focused_summary).expect("serialize focused mode summary"),
    );
    let artifact_links = escape_json_for_inline_script(
        &serde_json::to_string(&doc.artifact_links).expect("serialize flamegraph artifact links"),
    );
    let default_mode_json = escape_json_for_inline_script(
        &serde_json::to_string(default_mode).expect("serialize default flamegraph mode"),
    );

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>
    :root {{
      color-scheme: light;
      --bg: #f3f1d6;
      --panel: rgba(255,255,255,0.78);
      --panel-border: rgba(80,66,17,0.18);
      --text: #2d260f;
      --muted: #6f6339;
      --accent: #9a3f20;
      --accent-soft: rgba(154,63,32,0.12);
      --shadow: 0 18px 40px rgba(77, 58, 11, 0.12);
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      min-height: 100vh;
      background:
        radial-gradient(circle at top left, rgba(255,255,255,0.55), transparent 22rem),
        linear-gradient(180deg, #f8f6df 0%, #efe8b4 100%);
      color: var(--text);
      font: 14px/1.45 ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }}
    .viewer {{
      min-height: 100vh;
      display: grid;
      grid-template-rows: auto auto 1fr;
      gap: 12px;
      padding: 16px;
    }}
    .toolbar, .summary, .sidebar {{
      background: var(--panel);
      border: 1px solid var(--panel-border);
      box-shadow: var(--shadow);
      backdrop-filter: blur(14px);
      border-radius: 16px;
    }}
    .toolbar {{
      position: sticky;
      top: 0;
      z-index: 5;
      display: flex;
      gap: 12px;
      align-items: center;
      justify-content: space-between;
      padding: 12px 14px;
    }}
    .toolbar-group {{
      display: flex;
      gap: 8px;
      align-items: center;
      flex-wrap: wrap;
    }}
    .toolbar-title {{
      font-size: 13px;
      color: var(--muted);
      text-transform: uppercase;
      letter-spacing: 0.08em;
      margin-right: 6px;
    }}
    button {{
      border: 1px solid rgba(80,66,17,0.16);
      background: rgba(255,255,255,0.86);
      color: var(--text);
      border-radius: 999px;
      padding: 8px 12px;
      font: inherit;
      cursor: pointer;
      transition: background 120ms ease, border-color 120ms ease, transform 120ms ease;
    }}
    button:hover {{
      background: white;
      border-color: rgba(80,66,17,0.28);
    }}
    button:disabled {{
      opacity: 0.45;
      cursor: not-allowed;
    }}
    button.active {{
      background: var(--accent);
      border-color: var(--accent);
      color: white;
    }}
    button.selection-active {{
      background: #c3621f;
      border-color: #c3621f;
      color: white;
    }}
    .summary {{
      display: grid;
      grid-template-columns: repeat(4, minmax(0, 1fr));
      gap: 8px;
      padding: 12px 14px;
    }}
    .summary-item {{
      min-width: 0;
    }}
    .summary-label {{
      display: block;
      font-size: 11px;
      color: var(--muted);
      text-transform: uppercase;
      letter-spacing: 0.08em;
      margin-bottom: 4px;
    }}
    .summary-value {{
      font-size: 13px;
      font-weight: 600;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }}
    .workspace {{
      min-height: 0;
      display: grid;
      grid-template-columns: minmax(0, 1fr) 320px;
      gap: 12px;
    }}
    .graph-panel {{
      position: relative;
      min-height: 0;
      background: var(--panel);
      border: 1px solid var(--panel-border);
      border-radius: 16px;
      box-shadow: var(--shadow);
      overflow: hidden;
    }}
    .graph-stage {{
      position: relative;
      height: calc(100vh - 190px);
      min-height: 540px;
    }}
    iframe {{
      position: absolute;
      inset: 0;
      width: 100%;
      height: 100%;
      border: 0;
      background: transparent;
    }}
    iframe[hidden] {{
      display: none;
    }}
    .selection-overlay {{
      position: absolute;
      inset: 0;
      display: none;
      cursor: crosshair;
      background: transparent;
      z-index: 2;
    }}
    .selection-overlay.active {{
      display: block;
    }}
    .selection-box {{
      position: absolute;
      top: 0;
      bottom: 0;
      border: 1px solid rgba(154,63,32,0.65);
      background: rgba(154,63,32,0.12);
      box-shadow: inset 0 0 0 1px rgba(255,255,255,0.35);
      pointer-events: none;
      display: none;
    }}
    .selection-box.visible {{
      display: block;
    }}
    .selection-hint {{
      position: absolute;
      right: 14px;
      bottom: 14px;
      padding: 6px 10px;
      border-radius: 999px;
      background: rgba(45,38,15,0.72);
      color: white;
      font-size: 12px;
      z-index: 3;
      display: none;
    }}
    .selection-hint.active {{
      display: block;
    }}
    .sidebar {{
      padding: 14px;
      overflow: auto;
    }}
    .sidebar h2 {{
      margin: 0 0 10px 0;
      font-size: 14px;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: var(--muted);
    }}
    .warning {{
      margin-bottom: 12px;
      padding: 10px 12px;
      border-radius: 12px;
      background: rgba(195,98,31,0.12);
      color: #6f3415;
      border: 1px solid rgba(195,98,31,0.18);
    }}
    .frame-list, .artifact-list {{
      list-style: none;
      padding: 0;
      margin: 0;
      display: grid;
      gap: 8px;
    }}
    .frame-item, .artifact-item {{
      border: 1px solid rgba(80,66,17,0.10);
      border-radius: 12px;
      padding: 10px 12px;
      background: rgba(255,255,255,0.6);
    }}
    .frame-name {{
      display: block;
      font-weight: 600;
      word-break: break-word;
      margin-bottom: 4px;
    }}
    .frame-meta {{
      font-size: 12px;
      color: var(--muted);
    }}
    .artifact-link {{
      color: #7a341a;
      text-decoration: none;
      font-weight: 600;
      word-break: break-word;
    }}
    .artifact-link:hover {{
      text-decoration: underline;
    }}
    @media (max-width: 1100px) {{
      .workspace {{
        grid-template-columns: 1fr;
      }}
      .sidebar {{
        max-height: none;
      }}
    }}
    @media (max-width: 800px) {{
      .summary {{
        grid-template-columns: repeat(2, minmax(0, 1fr));
      }}
      .graph-stage {{
        min-height: 420px;
        height: calc(100vh - 250px);
      }}
    }}
  </style>
</head>
<body>
  <div class="viewer">
    <header class="toolbar">
      <div class="toolbar-group">
        <span class="toolbar-title">{title}</span>
        <button id="mode-focused" data-mode="focused" class="active">Benchmark Only</button>
        <button id="mode-full" data-mode="full">Full Process</button>
      </div>
      <div class="toolbar-group">
        <button id="viewer-select-range">Select Range</button>
        <button id="viewer-back" data-history-scope="focused">Back</button>
        <button id="viewer-forward" data-history-scope="focused">Forward</button>
        <button id="viewer-reset">Reset</button>
        <button id="viewer-search">Search</button>
      </div>
    </header>
    <section class="summary">
      <div class="summary-item">
        <span class="summary-label">Mode</span>
        <span class="summary-value" id="summary-mode">Benchmark Only</span>
      </div>
      <div class="summary-item">
        <span class="summary-label">Current Root</span>
        <span class="summary-value" id="summary-root">all</span>
      </div>
      <div class="summary-item">
        <span class="summary-label">Visible Samples</span>
        <span class="summary-value" id="summary-samples">-</span>
      </div>
      <div class="summary-item">
        <span class="summary-label">Selection Width</span>
        <span class="summary-value" id="summary-range">100%</span>
      </div>
    </section>
    <div class="workspace">
      <section class="graph-panel">
        <div class="graph-stage" id="graph-stage">
          <iframe id="frame-focused" title="Benchmark only flamegraph"></iframe>
          <iframe id="frame-full" title="Full process flamegraph" hidden></iframe>
          <div id="selection-overlay" class="selection-overlay">
            <div id="selection-box" class="selection-box"></div>
          </div>
          <div id="selection-hint" class="selection-hint">Drag across the graph to zoom the current x-axis range</div>
        </div>
      </section>
      <aside class="sidebar">
        <div id="sidebar-warning"></div>
        <h2>Self Time</h2>
        <ul id="self-frame-list" class="frame-list"></ul>
        <h2 style="margin-top:18px;">Inclusive Time</h2>
        <ul id="inclusive-frame-list" class="frame-list"></ul>
        <h2 style="margin-top:18px;">Artifacts</h2>
        <ul id="artifact-list" class="artifact-list"></ul>
      </aside>
    </div>
  </div>
  <script>
    const MOBENCH_DOCS = {{
      focused: {focused_svg},
      full: {full_svg},
    }};
    const MOBENCH_SUMMARIES = {{
      focused: {focused_summary},
      full: {full_summary},
    }};
    const MOBENCH_ARTIFACTS = {artifact_links};
    const MOBENCH_LABELS = {{
      focused: "Benchmark Only",
      full: "Full Process",
    }};
    const MOBENCH_STATE = {{
      activeMode: {default_mode_json},
      selectionMode: false,
      loaded: new Set(),
      currentView: {{
        focused: null,
        full: null,
      }},
    }};

    const frames = {{
      focused: document.getElementById("frame-focused"),
      full: document.getElementById("frame-full"),
    }};
    const modeButtons = {{
      focused: document.getElementById("mode-focused"),
      full: document.getElementById("mode-full"),
    }};
    const summaryMode = document.getElementById("summary-mode");
    const summaryRoot = document.getElementById("summary-root");
    const summarySamples = document.getElementById("summary-samples");
    const summaryRange = document.getElementById("summary-range");
    const backButton = document.getElementById("viewer-back");
    const forwardButton = document.getElementById("viewer-forward");
    const resetButton = document.getElementById("viewer-reset");
    const searchButton = document.getElementById("viewer-search");
    const selectionButton = document.getElementById("viewer-select-range");
    const selectionOverlay = document.getElementById("selection-overlay");
    const selectionBox = document.getElementById("selection-box");
    const selectionHint = document.getElementById("selection-hint");
    const sidebarWarning = document.getElementById("sidebar-warning");
    const selfFrameList = document.getElementById("self-frame-list");
    const inclusiveFrameList = document.getElementById("inclusive-frame-list");
    const artifactList = document.getElementById("artifact-list");

    let dragStartX = null;
    let dragCurrentX = null;

    function getActiveFrame() {{
      return frames[MOBENCH_STATE.activeMode];
    }}

    function getActiveWindow() {{
      const frame = getActiveFrame();
      return frame && frame.contentWindow ? frame.contentWindow : null;
    }}

    function getFrameWindow(mode) {{
      const frame = frames[mode];
      return frame && frame.contentWindow ? frame.contentWindow : null;
    }}

    function getFrameDocument(mode) {{
      const frame = frames[mode];
      return frame && frame.contentDocument ? frame.contentDocument : null;
    }}

    function ensureFrameLoaded(mode) {{
      if (MOBENCH_STATE.loaded.has(mode)) {{
        return;
      }}
      frames[mode].srcdoc = MOBENCH_DOCS[mode];
      MOBENCH_STATE.loaded.add(mode);
    }}

    function switchMode(mode) {{
      MOBENCH_STATE.activeMode = mode;
      frames.focused.hidden = mode !== "focused";
      frames.full.hidden = mode !== "full";
      modeButtons.focused.classList.toggle("active", mode === "focused");
      modeButtons.full.classList.toggle("active", mode === "full");
      backButton.dataset.historyScope = mode;
      forwardButton.dataset.historyScope = mode;
      ensureFrameLoaded(mode);
      stopSelectionMode();
      renderSidebar();
      refreshSummary();
    }}

    function renderSidebar() {{
      const summary = MOBENCH_SUMMARIES[MOBENCH_STATE.activeMode];
      sidebarWarning.innerHTML = "";
      if (summary.warning) {{
        const warning = document.createElement("div");
        warning.className = "warning";
        warning.textContent = summary.warning;
        sidebarWarning.appendChild(warning);
      }}

      selfFrameList.innerHTML = "";
      for (const frame of summary.self_frames) {{
        const item = document.createElement("li");
        item.className = "frame-item";
        item.innerHTML = `
          <span class="frame-name">${{escapeHtml(frame.frame)}}</span>
          <span class="frame-meta">${{frame.samples.toLocaleString()}} samples · ${{frame.percent_total}}% self</span>
        `;
        selfFrameList.appendChild(item);
      }}

      inclusiveFrameList.innerHTML = "";
      for (const frame of summary.inclusive_frames) {{
        const item = document.createElement("li");
        item.className = "frame-item";
        item.innerHTML = `
          <span class="frame-name">${{escapeHtml(frame.frame)}}</span>
          <span class="frame-meta">${{frame.samples.toLocaleString()}} samples · ${{frame.percent_total}}% inclusive</span>
        `;
        inclusiveFrameList.appendChild(item);
      }}

      artifactList.innerHTML = "";
      for (const artifact of MOBENCH_ARTIFACTS) {{
        const item = document.createElement("li");
        item.className = "artifact-item";
        item.innerHTML = `<a class="artifact-link" href="${{escapeAttribute(artifact.path)}}">${{escapeHtml(artifact.label)}}</a>`;
        artifactList.appendChild(item);
      }}
    }}

    function refreshSummary() {{
      const summary = MOBENCH_SUMMARIES[MOBENCH_STATE.activeMode];
      const current = MOBENCH_STATE.currentView[MOBENCH_STATE.activeMode];
      summaryMode.textContent = MOBENCH_LABELS[MOBENCH_STATE.activeMode];
      summaryRoot.textContent = current && current.label ? current.label : "all";
      const visibleSamples = current && typeof current.width === "number"
        ? current.width
        : summary.total_samples;
      summarySamples.textContent = visibleSamples.toLocaleString();
      const percent = summary.total_samples > 0
        ? Math.max(1, Math.round((visibleSamples / summary.total_samples) * 100))
        : 100;
      summaryRange.textContent = `${{percent}}%`;

      const activeWindow = getActiveWindow();
      backButton.disabled = !(activeWindow && activeWindow.mobenchCanGoBack && activeWindow.mobenchCanGoBack());
      forwardButton.disabled = !(activeWindow && activeWindow.mobenchCanGoForward && activeWindow.mobenchCanGoForward());
    }}

    function escapeHtml(value) {{
      return String(value)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/\"/g, "&quot;");
    }}

    function escapeAttribute(value) {{
      return escapeHtml(value).replace(/'/g, "&#39;");
    }}

    function parsePercent(value) {{
      return parseFloat(String(value || "0").replace("%", "")) || 0;
    }}

    function getVisibleSamplesForMode(mode) {{
      const current = MOBENCH_STATE.currentView[mode];
      if (current && typeof current.width === "number" && current.width > 0) {{
        return current.width;
      }}
      return Math.max(1, MOBENCH_SUMMARIES[mode].total_samples || 1);
    }}

    function startSelectionMode() {{
      MOBENCH_STATE.selectionMode = true;
      selectionButton.classList.add("selection-active");
      selectionOverlay.classList.add("active");
      selectionHint.classList.add("active");
    }}

    function stopSelectionMode() {{
      MOBENCH_STATE.selectionMode = false;
      selectionButton.classList.remove("selection-active");
      selectionOverlay.classList.remove("active");
      selectionHint.classList.remove("active");
      selectionBox.classList.remove("visible");
      dragStartX = null;
      dragCurrentX = null;
    }}

    function updateSelectionBox() {{
      if (dragStartX == null || dragCurrentX == null) {{
        selectionBox.classList.remove("visible");
        return;
      }}
      const left = Math.min(dragStartX, dragCurrentX);
      const right = Math.max(dragStartX, dragCurrentX);
      selectionBox.classList.add("visible");
      selectionBox.style.left = `${{left}}px`;
      selectionBox.style.width = `${{Math.max(1, right - left)}}px`;
    }}

    selectionOverlay.addEventListener("pointerdown", (event) => {{
      if (!MOBENCH_STATE.selectionMode) {{
        return;
      }}
      event.preventDefault();
      const rect = selectionOverlay.getBoundingClientRect();
      dragStartX = event.clientX - rect.left;
      dragCurrentX = dragStartX;
      updateSelectionBox();
      selectionOverlay.setPointerCapture(event.pointerId);
    }});

    selectionOverlay.addEventListener("pointermove", (event) => {{
      if (!MOBENCH_STATE.selectionMode || dragStartX == null) {{
        return;
      }}
      const rect = selectionOverlay.getBoundingClientRect();
      dragCurrentX = Math.min(rect.width, Math.max(0, event.clientX - rect.left));
      updateSelectionBox();
    }});

    selectionOverlay.addEventListener("pointerup", (event) => {{
      if (!MOBENCH_STATE.selectionMode || dragStartX == null || dragCurrentX == null) {{
        stopSelectionMode();
        return;
      }}
      const activeWindow = getActiveWindow();
      if (activeWindow && activeWindow.mobenchZoomVisibleFraction) {{
        const rect = selectionOverlay.getBoundingClientRect();
        const from = Math.min(dragStartX, dragCurrentX) / rect.width;
        const to = Math.max(dragStartX, dragCurrentX) / rect.width;
        activeWindow.mobenchZoomVisibleFraction(from, to);
      }}
      stopSelectionMode();
    }});

    selectionOverlay.addEventListener("pointercancel", () => {{
      stopSelectionMode();
    }});

    selectionButton.addEventListener("click", () => {{
      if (MOBENCH_STATE.selectionMode) {{
        stopSelectionMode();
      }} else {{
        startSelectionMode();
      }}
    }});

    backButton.addEventListener("click", () => {{
      const activeWindow = getActiveWindow();
      if (activeWindow && activeWindow.mobenchHistoryBack) {{
        activeWindow.mobenchHistoryBack();
      }}
    }});

    forwardButton.addEventListener("click", () => {{
      const activeWindow = getActiveWindow();
      if (activeWindow && activeWindow.mobenchHistoryForward) {{
        activeWindow.mobenchHistoryForward();
      }}
    }});

    resetButton.addEventListener("click", () => {{
      const activeWindow = getActiveWindow();
      if (activeWindow && activeWindow.mobenchResetView) {{
        activeWindow.mobenchResetView();
      }}
    }});

    searchButton.addEventListener("click", () => {{
      const activeWindow = getActiveWindow();
      if (!activeWindow || !activeWindow.mobenchSearch) {{
        return;
      }}
      const term = window.prompt("Search current flamegraph mode (regexp allowed). Leave empty to clear search.", "");
      if (term === null) {{
        return;
      }}
      activeWindow.mobenchSearch(term);
    }});

    modeButtons.focused.addEventListener("click", () => switchMode("focused"));
    modeButtons.full.addEventListener("click", () => switchMode("full"));

    window.addEventListener("message", (event) => {{
      if (!event.data || event.data.type !== "mobench:view-change") {{
        return;
      }}
      const mode = event.source === frames.focused.contentWindow
        ? "focused"
        : event.source === frames.full.contentWindow
        ? "full"
        : null;
      if (!mode) {{
        return;
      }}
      MOBENCH_STATE.currentView[mode] = event.data;
      if (mode === MOBENCH_STATE.activeMode) {{
        refreshSummary();
      }}
    }});

    frames.focused.addEventListener("load", () => {{
      const win = frames.focused.contentWindow;
      if (win && win.mobenchGetViewState) {{
        MOBENCH_STATE.currentView.focused = win.mobenchGetViewState();
        if (MOBENCH_STATE.activeMode === "focused") {{
          refreshSummary();
        }}
      }}
    }});

    frames.full.addEventListener("load", () => {{
      const win = frames.full.contentWindow;
      if (win && win.mobenchGetViewState) {{
        MOBENCH_STATE.currentView.full = win.mobenchGetViewState();
        if (MOBENCH_STATE.activeMode === "full") {{
          refreshSummary();
        }}
      }}
    }});

    ensureFrameLoaded("focused");
    ensureFrameLoaded("full");
    switchMode(MOBENCH_STATE.activeMode);
  </script>
</body>
</html>"#,
        title = escape_html(&doc.title),
        focused_svg = focused_svg,
        full_svg = full_svg,
        focused_summary = focused_summary,
        full_summary = full_summary,
        artifact_links = artifact_links,
        default_mode_json = default_mode_json
    )
}

fn escape_json_for_inline_script(json: &str) -> String {
    json.replace("</", "<\\/")
}

fn build_frame_breakdown_list(
    frame_samples: BTreeMap<String, u64>,
    total_samples: u64,
) -> Vec<FrameBreakdown> {
    let mut frames: Vec<FrameBreakdown> = frame_samples
        .into_iter()
        .map(|(frame, samples)| FrameBreakdown {
            frame,
            samples,
            percent_total: if total_samples == 0 {
                0
            } else {
                samples.saturating_mul(100) / total_samples
            },
        })
        .collect();
    frames.sort_by(|left, right| {
        right
            .samples
            .cmp(&left.samples)
            .then_with(|| left.frame.cmp(&right.frame))
    });
    frames.truncate(12);
    frames
}

fn prettify_folded_stacks_for_display(folded_stacks: &str) -> String {
    let mut lines = Vec::new();
    for line in folded_stacks.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((stack, count)) = split_folded_stack_line(trimmed) else {
            lines.push(trimmed.to_string());
            continue;
        };
        let pretty_stack = stack
            .split(';')
            .map(prettify_frame_label)
            .collect::<Vec<_>>()
            .join(";");
        lines.push(format!("{pretty_stack} {count}"));
    }
    lines.join("\n")
}

fn prettify_frame_label(frame: &str) -> String {
    let mut pretty = frame.to_string();
    for (needle, replacement) in [
        ("_$LT$", "<"),
        ("$LT$", "<"),
        ("$GT$", ">"),
        ("$u20$", " "),
        ("$u7b$", "{"),
        ("$u7d$", "}"),
        ("$u5b$", "["),
        ("$u5d$", "]"),
        ("$LP$", "("),
        ("$RP$", ")"),
        ("$C$", ","),
        ("$RF$", "&"),
    ] {
        pretty = pretty.replace(needle, replacement);
    }
    pretty = pretty.replace("..", "::");

    if let Some(hash_idx) = pretty.rfind("::h") {
        let hash = &pretty[hash_idx + 3..];
        if !hash.is_empty() && hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
            pretty.truncate(hash_idx);
        }
    }

    pretty
}

fn trim_stack_to_first_anchor<'a>(frames: &'a [&'a str], anchors: &[&str]) -> Option<&'a [&'a str]> {
    frames
        .iter()
        .position(|frame| anchors.iter().any(|anchor| frame.contains(anchor)))
        .map(|idx| &frames[idx..])
}

fn split_folded_stack_line(line: &str) -> Option<(&str, u64)> {
    let split = line.rfind(' ')?;
    let count = line[split + 1..].parse().ok()?;
    Some((&line[..split], count))
}

fn finalize_standalone_flamegraph_document(rendered: String) -> String {
    let rendered = rendered.replacen(
        "<svg version=\"1.1\"",
        "<svg version=\"1.1\" style=\"display:block;width:100vw;min-width:100vw;max-width:100vw;height:auto\"",
        1,
    );
    let rendered = rendered.replacen("var fluiddrawing = true;", "var fluiddrawing = false;", 1);
    let rendered = rendered.replacen(
        "#unzoom { cursor:pointer; }",
        "#unzoom { cursor:pointer; display:none; }\n#search, #matched, #details, #title { display:none; }",
        1,
    );
    inject_svg_script(rendered, MOBENCH_SVG_HELPER_SCRIPT)
}

fn inject_svg_script(document: String, script: &str) -> String {
    let Some(index) = document.rfind("</svg>") else {
        return document;
    };
    let mut output = String::with_capacity(document.len() + script.len() + 48);
    output.push_str(&document[..index]);
    output.push_str("<script><![CDATA[");
    output.push_str(script);
    output.push_str("]]></script>");
    output.push_str(&document[index..]);
    output
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const MOBENCH_SVG_HELPER_SCRIPT: &str = r#"
(function () {
  function mobenchTotalSamples() {
    return typeof total_samples === "number"
      ? total_samples
      : parseInt(frames.attributes.total_samples.value || "0", 10);
  }

  function mobenchTitleForNode(node) {
    try {
      var title = find_child(node, "title");
      if (!title || !title.textContent) return "selected range";
      return title.textContent.replace(/\s+\([^)]*\)$/, "");
    } catch (_error) {
      return "selected range";
    }
  }

  function mobenchNormalizeView(view) {
    var total = mobenchTotalSamples();
    var xmin = Math.max(0, Math.floor(view.xmin || 0));
    var width = Math.max(1, Math.floor(view.width || total || 1));
    if (xmin + width > total && total > 0) {
      width = total - xmin;
    }
    if (width <= 0) {
      xmin = 0;
      width = Math.max(1, total);
    }
    return {
      xmin: xmin,
      width: width,
      label: view.label || "selected range"
    };
  }

  var mobenchState = {
    history: [],
    index: -1,
    current: null
  };

  var mobenchCollapsedTowerState = {
    originalViewBox: null,
    originalHeight: null,
    originalBodyHeight: "",
    originalDocumentHeight: ""
  };

  function mobenchSvgRoot() {
    return document.querySelector("svg");
  }

  function mobenchRememberViewport() {
    var svg = mobenchSvgRoot();
    if (!svg) return null;
    if (mobenchCollapsedTowerState.originalViewBox === null) {
      mobenchCollapsedTowerState.originalViewBox =
        svg.getAttribute("viewBox")
        || ("0 0 " + (svg.viewBox && svg.viewBox.baseVal ? svg.viewBox.baseVal.width : 1200)
          + " "
          + (svg.viewBox && svg.viewBox.baseVal ? svg.viewBox.baseVal.height : parseFloat(svg.getAttribute("height") || "900")));
      mobenchCollapsedTowerState.originalHeight = svg.getAttribute("height");
      mobenchCollapsedTowerState.originalBodyHeight = document.body.style.height || "";
      mobenchCollapsedTowerState.originalDocumentHeight = document.documentElement.style.height || "";
    }
    return svg;
  }

  window.mobenchClearCollapsedTowerPresentation = function () {
    var svg = mobenchRememberViewport();
    var elements = frames.children;
    for (var i = 0; i < elements.length; i++) {
      if (elements[i].dataset.mobenchTowerHidden === "1") {
        elements[i].style.display = "";
        delete elements[i].dataset.mobenchTowerHidden;
      }
    }
    if (svg) {
      if (mobenchCollapsedTowerState.originalViewBox !== null) {
        svg.setAttribute("viewBox", mobenchCollapsedTowerState.originalViewBox);
      }
      if (mobenchCollapsedTowerState.originalHeight !== null) {
        svg.setAttribute("height", mobenchCollapsedTowerState.originalHeight);
      } else {
        svg.removeAttribute("height");
      }
      document.body.style.height = mobenchCollapsedTowerState.originalBodyHeight;
      document.documentElement.style.height = mobenchCollapsedTowerState.originalDocumentHeight;
    }
  };

  window.mobenchGetVisibleFrames = function () {
    var visible = [];
    var elements = frames.children;
    for (var i = 0; i < elements.length; i++) {
      var element = elements[i];
      if (element.classList.contains("hide") || element.dataset.mobenchTowerHidden === "1") {
        continue;
      }
      var rect = find_child(element, "rect");
      if (!rect || !rect.attributes["fg:x"] || !rect.attributes["fg:w"]) {
        continue;
      }
      var titleNode = find_child(element, "title");
      var titleText = titleNode && titleNode.textContent ? titleNode.textContent : "";
      var samplesMatch = titleText.match(/\(([0-9,]+)\s+samples?/);
      visible.push({
        index: i,
        label: mobenchTitleForNode(element),
        title: titleText,
        xPct: parseFloat(rect.attributes.x.value || "0"),
        widthPct: parseFloat(rect.attributes.width.value || "0"),
        x: parseInt(rect.attributes["fg:x"].value || "0", 10),
        width: parseInt(rect.attributes["fg:w"].value || "0", 10),
        y: parseFloat(rect.attributes.y.value || "0"),
        samples: samplesMatch ? parseInt(samplesMatch[1].replace(/,/g, ""), 10) : 0
      });
    }
    return visible;
  };

  window.mobenchSetCollapsedTowerPresentation = function (presentation) {
    window.mobenchClearCollapsedTowerPresentation();
    if (!presentation || !presentation.hiddenIndexes || !presentation.hiddenIndexes.length) {
      return false;
    }
    var svg = mobenchRememberViewport();
    var elements = frames.children;
    for (var i = 0; i < presentation.hiddenIndexes.length; i++) {
      var index = presentation.hiddenIndexes[i];
      if (!elements[index]) continue;
      elements[index].style.display = "none";
      elements[index].dataset.mobenchTowerHidden = "1";
    }
    if (svg && presentation.viewport) {
      var originalViewBox = (mobenchCollapsedTowerState.originalViewBox || "0 0 1200 900")
        .split(/\s+/)
        .map(function (value) { return parseFloat(value) || 0; });
      var minY = Math.max(0, presentation.viewport.minY || 0);
      var height = Math.max(220, presentation.viewport.height || originalViewBox[3] || 900);
      svg.setAttribute("viewBox", [originalViewBox[0], minY, originalViewBox[2], height].join(" "));
      svg.setAttribute("height", String(Math.round(height)));
      document.body.style.height = Math.round(height) + "px";
      document.documentElement.style.height = Math.round(height) + "px";
    }
    return true;
  };

  function mobenchResetDom() {
    window.mobenchClearCollapsedTowerPresentation();
    var elements = frames.children;
    for (var i = 0; i < elements.length; i++) {
      elements[i].classList.remove("parent");
      elements[i].classList.remove("hide");
      zoom_reset(elements[i]);
    }
    update_text_for_elements(elements);
  }

  function mobenchNotifyParent() {
    try {
      parent.postMessage({
        type: "mobench:view-change",
        label: mobenchState.current ? mobenchState.current.label : "all",
        start: mobenchState.current ? mobenchState.current.xmin : 0,
        width: mobenchState.current ? mobenchState.current.width : mobenchTotalSamples(),
        total: mobenchTotalSamples()
      }, "*");
    } catch (_error) {}
  }

  function mobenchPushHistory(view) {
    if (
      mobenchState.index >= 0 &&
      mobenchState.history[mobenchState.index] &&
      mobenchState.history[mobenchState.index].xmin === view.xmin &&
      mobenchState.history[mobenchState.index].width === view.width &&
      mobenchState.history[mobenchState.index].label === view.label
    ) {
      return;
    }
    mobenchState.history = mobenchState.history.slice(0, mobenchState.index + 1);
    mobenchState.history.push(view);
    mobenchState.index = mobenchState.history.length - 1;
  }

  function mobenchApplyAbsoluteRange(xmin, width, label, pushHistory) {
    if (!frames) return false;
    var total = mobenchTotalSamples();
    var view = mobenchNormalizeView({
      xmin: xmin,
      width: width,
      label: label
    });
    mobenchResetDom();
    var elements = frames.children;
    var toUpdate = [];
    var xmax = view.xmin + view.width;
    for (var i = 0; i < elements.length; i++) {
      var element = elements[i];
      var rect = find_child(element, "rect");
      if (!rect || !rect.attributes["fg:x"] || !rect.attributes["fg:w"]) {
        continue;
      }
      var ex = parseInt(rect.attributes["fg:x"].value, 10);
      var ew = parseInt(rect.attributes["fg:w"].value, 10);
      var ix0 = Math.max(ex, view.xmin);
      var ix1 = Math.min(ex + ew, xmax);
      if (!(ix1 > ix0)) {
        element.classList.add("hide");
        continue;
      }
      rect.attributes.x.value = format_percent(100 * (ix0 - view.xmin) / view.width);
      rect.attributes.width.value = format_percent(100 * (ix1 - ix0) / view.width);
      toUpdate.push(element);
    }
    update_text_for_elements(toUpdate);
    mobenchState.current = view;
    if (pushHistory !== false) {
      mobenchPushHistory(view);
    }
    mobenchNotifyParent();
    return view.width < total;
  }

  window.mobenchResetView = function () {
    var view = {
      xmin: 0,
      width: Math.max(1, mobenchTotalSamples()),
      label: "all"
    };
    mobenchResetDom();
    mobenchState.history = [view];
    mobenchState.index = 0;
    mobenchState.current = view;
    mobenchNotifyParent();
  };

  window.mobenchCanGoBack = function () {
    return mobenchState.index > 0;
  };

  window.mobenchCanGoForward = function () {
    return mobenchState.index >= 0 && mobenchState.index < mobenchState.history.length - 1;
  };

  window.mobenchHistoryBack = function () {
    if (!window.mobenchCanGoBack()) return false;
    mobenchState.index -= 1;
    var view = mobenchState.history[mobenchState.index];
    return mobenchApplyAbsoluteRange(view.xmin, view.width, view.label, false);
  };

  window.mobenchHistoryForward = function () {
    if (!window.mobenchCanGoForward()) return false;
    mobenchState.index += 1;
    var view = mobenchState.history[mobenchState.index];
    return mobenchApplyAbsoluteRange(view.xmin, view.width, view.label, false);
  };

  window.mobenchZoomToFrame = function (node, pushHistory) {
    var rect = find_child(node, "rect");
    if (!rect || !rect.attributes["fg:x"] || !rect.attributes["fg:w"]) {
      return false;
    }
    return mobenchApplyAbsoluteRange(
      parseInt(rect.attributes["fg:x"].value, 10),
      parseInt(rect.attributes["fg:w"].value, 10),
      mobenchTitleForNode(node),
      pushHistory !== false
    );
  };

  window.mobenchZoomVisibleFraction = function (from, to) {
    var total = mobenchTotalSamples();
    if (!mobenchState.current) {
      mobenchState.current = { xmin: 0, width: total, label: "all" };
    }
    var start = Math.min(from, to);
    var end = Math.max(from, to);
    if (!isFinite(start) || !isFinite(end) || (end - start) < 0.015) {
      return false;
    }
    var xmin = Math.floor(mobenchState.current.xmin + mobenchState.current.width * start);
    var width = Math.max(1, Math.floor(mobenchState.current.width * (end - start)));
    return mobenchApplyAbsoluteRange(xmin, width, "selected range", true);
  };

  window.mobenchZoomAbsoluteRange = function (xmin, width, label) {
    if (!isFinite(xmin) || !isFinite(width) || width <= 0) {
      return false;
    }
    return mobenchApplyAbsoluteRange(xmin, width, label || "selected range", true);
  };

  window.mobenchSearch = function (term) {
    if (!term) {
      if (typeof reset_search === "function") {
        reset_search();
      }
      searching = 0;
      mobenchNotifyParent();
      return;
    }
    if (typeof search === "function") {
      search(term);
      mobenchNotifyParent();
    }
  };

  window.mobenchGetViewState = function () {
    if (!mobenchState.current) {
      return {
        label: "all",
        start: 0,
        width: mobenchTotalSamples(),
        total: mobenchTotalSamples()
      };
    }
    return {
      label: mobenchState.current.label,
      start: mobenchState.current.xmin,
      width: mobenchState.current.width,
      total: mobenchTotalSamples()
    };
  };

  zoom = function (node) {
    return window.mobenchZoomToFrame(node, true);
  };

  unzoom = function () {
    window.mobenchResetView();
  };

  window.addEventListener("load", function () {
    setTimeout(function () {
      window.mobenchResetView();
    }, 0);
  });
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc() -> FlamegraphViewerDoc {
        FlamegraphViewerDoc {
            title: "iOS Native Profile".into(),
            full_svg_document: "<svg id=\"full\"></svg>".into(),
            focused_svg_document: "<svg id=\"focused\"></svg>".into(),
            full_summary: ModeSummary {
                total_samples: 10,
                visible_stack_count: 2,
                matched_stack_count: 2,
                excluded_stack_count: 0,
                warning: None,
                self_frames: vec![FrameBreakdown {
                    frame: "start".into(),
                    samples: 10,
                    percent_total: 100,
                }],
                inclusive_frames: vec![FrameBreakdown {
                    frame: "start".into(),
                    samples: 10,
                    percent_total: 100,
                }],
            },
            focused_summary: ModeSummary {
                total_samples: 5,
                visible_stack_count: 1,
                matched_stack_count: 1,
                excluded_stack_count: 1,
                warning: Some("focused warning".into()),
                self_frames: vec![FrameBreakdown {
                    frame: "sample_fns::fibonacci".into(),
                    samples: 5,
                    percent_total: 100,
                }],
                inclusive_frames: vec![FrameBreakdown {
                    frame: "sample_fns::run_benchmark".into(),
                    samples: 5,
                    percent_total: 100,
                }],
            },
            default_mode: FlamegraphMode::Focused,
            artifact_links: vec![ArtifactLink::new("native-report.txt", "native-report.txt")],
        }
    }

    #[test]
    fn focused_stack_derivation_returns_empty_when_no_anchor_matches() {
        let folded = "start;UIKitMain 1\n";
        let focused =
            derive_benchmark_focused_folded_stacks(folded, &["sample_fns::run_benchmark"]);
        assert!(focused.folded.is_empty());
        assert_eq!(focused.matched_stack_count, 0);
    }

    #[test]
    fn derive_benchmark_focused_folded_stacks_trims_ios_stack_to_benchmark_anchor() {
        let folded = concat!(
            "start;UIApplicationMain;runBenchmark(spec:);uniffi_sample_fns_fn_func_run_benchmark;",
            "sample_fns::run_benchmark;mobench_sdk::timing::profile_phase;",
            "sample_fns::fibonacci 5\n"
        );

        let focused = derive_benchmark_focused_folded_stacks(
            folded,
            &["runBenchmark(spec:)", "sample_fns::run_benchmark"],
        );

        assert_eq!(
            focused.folded,
            "runBenchmark(spec:);uniffi_sample_fns_fn_func_run_benchmark;sample_fns::run_benchmark;mobench_sdk::timing::profile_phase;sample_fns::fibonacci 5"
        );
    }

    #[test]
    fn derive_benchmark_focused_folded_stacks_trims_android_stack_to_rust_anchor() {
        let folded = concat!(
            "all;uniffi.sample_fns.Sample_fnsKt.runBenchmark;",
            "libsample_fns.so;sample_fns::run_benchmark;mobench_sdk::timing::run_closure;",
            "sample_fns::fibonacci 3\n"
        );

        let focused = derive_benchmark_focused_folded_stacks(
            folded,
            &["sample_fns::run_benchmark", "mobench_sdk::timing::run_closure"],
        );

        assert_eq!(
            focused.folded,
            "sample_fns::run_benchmark;mobench_sdk::timing::run_closure;sample_fns::fibonacci 3"
        );
    }

    #[test]
    fn standalone_viewer_html_embeds_full_and_focused_modes() {
        let html = render_flamegraph_viewer_html(sample_doc());

        assert!(html.contains("Benchmark Only"));
        assert!(html.contains("Full Process"));
        assert!(html.contains("data-mode=\"focused\""));
        assert!(html.contains("data-mode=\"full\""));
        assert!(html.contains("<svg id=\\\"full\\\"><\\/svg>"));
        assert!(html.contains("<svg id=\\\"focused\\\"><\\/svg>"));
    }

    #[test]
    fn viewer_html_includes_history_and_brush_zoom_controls() {
        let html = render_flamegraph_viewer_html(sample_doc());
        assert!(html.contains("id=\"viewer-back\""));
        assert!(html.contains("id=\"viewer-forward\""));
        assert!(html.contains("id=\"viewer-reset\""));
        assert!(html.contains("id=\"viewer-search\""));
        assert!(html.contains("id=\"viewer-select-range\""));
        assert!(html.contains("id=\"selection-overlay\""));
        assert!(html.contains("data-history-scope=\"focused\""));
        assert!(!html.contains("target=\"_blank\""));
    }

    #[test]
    fn viewer_html_omits_experimental_tower_controls() {
        let html = render_flamegraph_viewer_html(sample_doc());
        assert!(!html.contains("id=\"viewer-hide-towers\""));
        assert!(!html.contains("Hide Thin Towers"));
        assert!(!html.contains("id=\"tower-overlay\""));
        assert!(!html.contains("id=\"tower-meta\""));
        assert!(!html.contains("applyTowerCollapse"));
    }

    #[test]
    fn viewer_html_renders_hot_frame_summary_for_each_mode() {
        let html = render_flamegraph_viewer_html(sample_doc());
        assert!(html.contains("Self Time"));
        assert!(html.contains("Inclusive Time"));
        assert!(html.contains("sample_fns::fibonacci"));
        assert!(html.contains("sample_fns::run_benchmark"));
        assert!(html.contains("focused warning"));
    }

    #[test]
    fn summarize_folded_stacks_caps_inclusive_percent_for_repeated_frames() {
        let summary = summarize_folded_stacks(
            "root;repeat;repeat 4\nroot;repeat;leaf 1\n",
            2,
            0,
            None,
        );

        let repeat = summary
            .inclusive_frames
            .iter()
            .find(|frame| frame.frame == "repeat")
            .expect("repeat frame");
        let leaf = summary
            .self_frames
            .iter()
            .find(|frame| frame.frame == "leaf")
            .expect("leaf frame");

        assert_eq!(repeat.samples, 5);
        assert_eq!(repeat.percent_total, 100);
        assert_eq!(leaf.samples, 1);
        assert_eq!(leaf.percent_total, 20);
    }

    #[test]
    fn summarize_folded_stacks_prettifies_rust_symbol_noise() {
        let summary = summarize_folded_stacks(
            "root;sample_fns::fibonacci::ha1ebbae54edac99d 3\nroot;_$LT$u32$u20$as$u20$core..iter..range..Step$GT$::forward_unchecked::h2f57f430431a1dbe 2\n",
            2,
            0,
            None,
        );

        assert!(summary
            .self_frames
            .iter()
            .any(|frame| frame.frame == "sample_fns::fibonacci"));
        assert!(summary
            .self_frames
            .iter()
            .any(|frame| frame.frame == "<u32 as core::iter::range::Step>::forward_unchecked"));
    }

    #[test]
    fn viewer_html_escapes_embedded_svg_script_terminators() {
        let mut doc = sample_doc();
        doc.focused_svg_document = "<svg><script>alert('focused')</script></svg>".into();
        doc.full_svg_document = "<svg><script>alert('full')</script></svg>".into();

        let html = render_flamegraph_viewer_html(doc);

        assert!(html.contains("<\\/script>"));
        assert!(!html.contains("alert('focused')</script></svg>,\n      full:"));
    }

    #[test]
    fn standalone_svg_defaults_to_viewport_width_and_custom_helpers() {
        let svg =
            render_standalone_flamegraph_svg("root;sample_fns::fibonacci 1", "Test Flamegraph")
                .expect("render svg");
        assert!(svg.contains("var fluiddrawing = false;"));
        assert!(svg.contains("width:100vw"));
        assert!(svg.contains("mobenchZoomVisibleFraction"));
    }

    #[test]
    fn standalone_svg_includes_tower_expand_hooks() {
        let svg =
            render_standalone_flamegraph_svg("root;sample_fns::fibonacci 1", "Test Flamegraph")
                .expect("render svg");
        assert!(svg.contains("mobenchGetVisibleFrames"));
        assert!(svg.contains("mobenchSetCollapsedTowerPresentation"));
        assert!(svg.contains("mobenchClearCollapsedTowerPresentation"));
        assert!(svg.contains("mobenchZoomAbsoluteRange"));
    }
}
