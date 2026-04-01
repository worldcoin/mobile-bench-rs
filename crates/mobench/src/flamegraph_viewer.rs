use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ViewerMetadataItem {
    pub label: String,
    pub value: String,
}

#[cfg(test)]
impl ViewerMetadataItem {
    pub(crate) fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ViewerHarnessTimelineSpan {
    pub phase: String,
    pub start_offset_ns: u64,
    pub end_offset_ns: u64,
    pub iteration: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ViewerTraceEvent {
    pub event_kind: String,
    pub start_offset_ns: u64,
    pub end_offset_ns: Option<u64>,
    pub frames: Vec<String>,
    pub phase: Option<String>,
    pub iteration: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ViewerTraceLane {
    pub id: String,
    pub label: String,
    pub events: Vec<ViewerTraceEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FrameSourceLink {
    pub frame: String,
    pub location: String,
    pub href: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlamegraphMode {
    Focused,
    Full,
    Timeline,
}

impl FlamegraphMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Focused => "focused",
            Self::Full => "full",
            Self::Timeline => "timeline",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FlamegraphViewerDoc {
    pub title: String,
    pub browser_title: String,
    pub full_svg_document: String,
    pub focused_svg_document: String,
    pub full_summary: ModeSummary,
    pub focused_summary: ModeSummary,
    pub sampled_duration_secs: Option<f64>,
    pub run_metadata: Vec<ViewerMetadataItem>,
    pub harness_timeline: Vec<ViewerHarnessTimelineSpan>,
    pub timeline_lanes: Vec<ViewerTraceLane>,
    pub timeline_total_duration_ns: Option<u64>,
    pub timeline_note: Option<String>,
    pub default_mode: FlamegraphMode,
    pub artifact_links: Vec<ArtifactLink>,
    pub source_links: Vec<FrameSourceLink>,
    pub source_link_note: Option<String>,
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
    folded
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
}

pub(crate) fn render_standalone_flamegraph_svg(folded_stacks: &str, title: &str) -> Result<String> {
    if folded_stacks.trim().is_empty() {
        return Ok(
            "<!DOCTYPE html><html><body><p>No native frames were symbolized.</p></body></html>"
                .into(),
        );
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
    let template = include_str!("flamegraph_viewer_template.html");
    let default_mode = escape_json_for_inline_script(
        &serde_json::to_string(doc.default_mode.as_str())
            .expect("serialize default flamegraph mode"),
    );
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
    let sampled_duration_secs = escape_json_for_inline_script(
        &serde_json::to_string(&doc.sampled_duration_secs)
            .expect("serialize sampled duration seconds"),
    );
    let harness_timeline = escape_json_for_inline_script(
        &serde_json::to_string(&doc.harness_timeline).expect("serialize harness timeline"),
    );
    let timeline_lanes = escape_json_for_inline_script(
        &serde_json::to_string(&doc.timeline_lanes).expect("serialize timeline lanes"),
    );
    let timeline_total_duration_ns = escape_json_for_inline_script(
        &serde_json::to_string(&doc.timeline_total_duration_ns)
            .expect("serialize timeline total duration"),
    );
    let timeline_note = escape_json_for_inline_script(
        &serde_json::to_string(&doc.timeline_note).expect("serialize timeline note"),
    );
    let source_links = escape_json_for_inline_script(
        &serde_json::to_string(&doc.source_links).expect("serialize source links"),
    );
    let source_link_note = escape_json_for_inline_script(
        &serde_json::to_string(&doc.source_link_note).expect("serialize source link note"),
    );

    template
        .replace("__BROWSER_TITLE__", &escape_html(&doc.browser_title))
        .replace("__VIEWER_TITLE__", &escape_html(&doc.title))
        .replace(
            "__RUN_METADATA_MARKUP__",
            &render_run_metadata_markup(&doc.run_metadata),
        )
        .replace(
            "__HARNESS_TIMELINE_MARKUP__",
            &render_harness_timeline_markup(&doc.harness_timeline),
        )
        .replace("__DEFAULT_MODE__", &default_mode)
        .replace("__FOCUSED_SVG__", &focused_svg)
        .replace("__FULL_SVG__", &full_svg)
        .replace("__FOCUSED_SUMMARY__", &focused_summary)
        .replace("__FULL_SUMMARY__", &full_summary)
        .replace("__ARTIFACT_LINKS__", &artifact_links)
        .replace("__SAMPLED_DURATION_SECS__", &sampled_duration_secs)
        .replace("__HARNESS_TIMELINE_JSON__", &harness_timeline)
        .replace("__TIMELINE_LANES_JSON__", &timeline_lanes)
        .replace(
            "__TIMELINE_TOTAL_DURATION_NS__",
            &timeline_total_duration_ns,
        )
        .replace("__TIMELINE_NOTE__", &timeline_note)
        .replace("__SOURCE_LINKS__", &source_links)
        .replace("__SOURCE_LINK_NOTE__", &source_link_note)
}

fn render_run_metadata_markup(items: &[ViewerMetadataItem]) -> String {
    items.iter()
        .map(|item| {
            format!(
                "<div class=\"run-metadata-item\"><span class=\"run-metadata-label\">{}</span><span class=\"run-metadata-value\">{}</span></div>",
                escape_html(&item.label),
                escape_html(&item.value)
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn render_harness_timeline_markup(spans: &[ViewerHarnessTimelineSpan]) -> String {
    let total_duration_ns = spans
        .iter()
        .map(|span| span.end_offset_ns)
        .max()
        .unwrap_or(0);
    let mut segments = String::new();
    for span in spans {
        let left = if total_duration_ns == 0 {
            0.0
        } else {
            (span.start_offset_ns as f64 / total_duration_ns as f64) * 100.0
        };
        let width = if total_duration_ns == 0 {
            100.0
        } else {
            (((span.end_offset_ns.saturating_sub(span.start_offset_ns)) as f64)
                / total_duration_ns as f64)
                * 100.0
        };
        let label = harness_phase_label(&span.phase, span.iteration);
        let duration = format_duration_ns(span.end_offset_ns.saturating_sub(span.start_offset_ns));
        let title = format!(
            "{} · {} to {}",
            label,
            format_duration_ns(span.start_offset_ns),
            format_duration_ns(span.end_offset_ns)
        );
        segments.push_str(&format!(
            "<span class=\"harness-segment\" data-phase=\"{}\" style=\"left:{:.4}%;width:{:.4}%\" title=\"{}\"><span class=\"harness-segment-name\">{}</span><span class=\"harness-segment-duration\">{}</span></span>",
            escape_html(&span.phase),
            left,
            width.max(0.5),
            escape_html(&title),
            escape_html(&label),
            escape_html(&duration),
        ));
    }

    format!(
        "<div class=\"harness-timeline-meta\"><span class=\"harness-timeline-title\">Harness Timeline</span><span class=\"harness-timeline-total\" id=\"harness-total\">Exact harness time · {}</span></div><div class=\"harness-track\" id=\"harness-track\">{}</div><div class=\"harness-track-scale\" id=\"harness-scale\"><span>0</span><span>{}</span></div><div class=\"harness-readout\" id=\"harness-readout\">Hover a harness segment to inspect its full label.</div>",
        escape_html(&format_duration_ns(total_duration_ns)),
        segments,
        escape_html(&format_duration_ns(total_duration_ns)),
    )
}

fn harness_phase_label(phase: &str, iteration: Option<u32>) -> String {
    let base = match phase {
        "setup" => "Setup",
        "fixture-setup" => "Fixture Setup",
        "warmup-benchmark" => "Warmup",
        "measured-benchmark" => "Bench Body",
        "teardown" => "Teardown",
        "fixture-teardown" => "Fixture Teardown",
        "harness" => "Harness",
        _ => phase,
    };
    match iteration {
        Some(iteration) => format!("{base} #{}", iteration + 1),
        None => base.to_string(),
    }
}

fn format_duration_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.2} s", ns as f64 / 1_000_000_000.0)
    } else if ns >= 1_000_000 {
        format!("{:.2} ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.2} µs", ns as f64 / 1_000.0)
    } else {
        format!("{ns} ns")
    }
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
        let Some((stack, counts)) = split_folded_stack_counts(trimmed) else {
            lines.push(trimmed.to_string());
            continue;
        };
        let pretty_stack = stack
            .split(';')
            .map(prettify_frame_label)
            .collect::<Vec<_>>()
            .join(";");
        lines.push(format!("{pretty_stack} {counts}"));
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

fn trim_stack_to_first_anchor<'a>(
    frames: &'a [&'a str],
    anchors: &[&str],
) -> Option<&'a [&'a str]> {
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

fn split_folded_stack_counts(line: &str) -> Option<(&str, &str)> {
    let (rest, last) = line.rsplit_once(' ')?;
    if last.parse::<i64>().is_err() {
        return None;
    }
    if let Some((stack, previous)) = rest.rsplit_once(' ')
        && previous.parse::<i64>().is_ok()
    {
        return Some((stack, &line[stack.len() + 1..]));
    }
    Some((rest, last))
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
    let rendered = retint_flamegraph_background(rendered);
    let rendered = retint_flamegraph_palette(rendered);
    inject_svg_script(rendered, MOBENCH_SVG_HELPER_SCRIPT)
}

fn retint_flamegraph_background(document: String) -> String {
    document
        .replace(r##"stop-color="#eeeeee""##, r##"stop-color="#ffffff""##)
        .replace(r##"stop-color="#eeeeb0""##, r##"stop-color="#ffffff""##)
}

fn retint_flamegraph_palette(document: String) -> String {
    const NEEDLE: &str = r#"fill="rgb("#;
    if !document.contains(NEEDLE) {
        return document;
    }

    let mut output = String::with_capacity(document.len());
    let mut cursor = 0usize;

    while let Some(relative) = document[cursor..].find(NEEDLE) {
        let start = cursor + relative;
        output.push_str(&document[cursor..start]);
        let value_start = start + NEEDLE.len();
        let Some(value_end) = document[value_start..].find(")\"") else {
            output.push_str(&document[start..]);
            return output;
        };
        let value_end = value_start + value_end;
        let raw = &document[value_start..value_end];
        let replacement = parse_fill_rgb(raw)
            .map(|rgb| flamegraph_fill_for_rgb(rgb, frame_title_for_fill(&document, start)))
            .unwrap_or_else(|| format!(r#"fill="rgb({raw})""#));
        output.push_str(&replacement);
        cursor = value_end + 2;
    }

    output.push_str(&document[cursor..]);
    output
}

fn frame_title_for_fill<'a>(document: &'a str, fill_start: usize) -> &'a str {
    document[..fill_start]
        .rfind("<title>")
        .and_then(|title_start| {
            let title_body_start = title_start + "<title>".len();
            document[title_body_start..fill_start]
                .find("</title>")
                .map(|title_end| &document[title_body_start..title_body_start + title_end])
        })
        .unwrap_or("neutral-frame")
}

fn parse_fill_rgb(raw: &str) -> Option<(u8, u8, u8)> {
    let mut parts = raw.split(',');
    let r = parts.next()?.trim().parse().ok()?;
    let g = parts.next()?.trim().parse().ok()?;
    let b = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((r, g, b))
}

fn flamegraph_fill_for_rgb(rgb: (u8, u8, u8), title: &str) -> String {
    let (r, g, b) = rgb;
    if rgb == (0, 0, 0) {
        return format!(r#"fill="rgb({r},{g},{b})""#);
    }

    if is_neutral_fill(rgb) {
        return neutral_frame_fill_for_title(title);
    }

    if r >= g && r >= b && r.saturating_sub(b) > 12 {
        let palette = [(255, 107, 116), (255, 123, 122), (255, 139, 127)];
        let shade = ((r as usize + g as usize + b as usize) / 48).min(palette.len() - 1);
        let (nr, ng, nb) = palette[shade];
        return format!(r#"fill="rgb({nr},{ng},{nb})""#);
    }

    if b >= r && b >= g && b.saturating_sub(r) > 8 {
        let palette = [(255, 214, 194), (255, 202, 179), (255, 191, 164)];
        let shade = ((r as usize + g as usize + b as usize) / 96).min(palette.len() - 1);
        let (nr, ng, nb) = palette[shade];
        return format!(r#"fill="rgb({nr},{ng},{nb})""#);
    }

    format!(r#"fill="rgb({r},{g},{b})""#)
}

fn is_neutral_fill((r, g, b): (u8, u8, u8)) -> bool {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    max >= 242 && max.saturating_sub(min) <= 10
}

fn neutral_frame_fill_for_title(title: &str) -> String {
    const PALETTE: [(u8, u8, u8); 8] = [
        (255, 107, 116),
        (255, 166, 95),
        (255, 178, 92),
        (255, 194, 99),
        (255, 220, 107),
        (255, 208, 123),
        (255, 156, 120),
        (255, 189, 118),
    ];
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut hasher);
    let (r, g, b) = PALETTE[(hasher.finish() as usize) % PALETTE.len()];
    format!(r#"fill="rgb({r},{g},{b})""#)
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

  window.mobenchScrollToBase = function () {
    function applyScroll() {
      var doc = document.documentElement || document.body;
      var body = document.body || document.documentElement;
      var maxScroll = Math.max(
        0,
        (doc ? doc.scrollHeight : 0),
        (body ? body.scrollHeight : 0)
      ) - window.innerHeight;
      window.scrollTo(0, Math.max(0, maxScroll));
    }
    applyScroll();
    setTimeout(applyScroll, 0);
  };

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
      window.mobenchScrollToBase();
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
            browser_title: "Mobench Flamegraph - mobile-bench-rs".into(),
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
            sampled_duration_secs: Some(10.0),
            run_metadata: vec![
                ViewerMetadataItem::new("Target", "ios"),
                ViewerMetadataItem::new("Benchmark", "sample_fns::fibonacci"),
                ViewerMetadataItem::new("Run ID", "ios-demo"),
            ],
            harness_timeline: vec![
                ViewerHarnessTimelineSpan {
                    phase: "setup".into(),
                    start_offset_ns: 0,
                    end_offset_ns: 100,
                    iteration: None,
                },
                ViewerHarnessTimelineSpan {
                    phase: "measured-benchmark".into(),
                    start_offset_ns: 100,
                    end_offset_ns: 300,
                    iteration: Some(0),
                },
                ViewerHarnessTimelineSpan {
                    phase: "teardown".into(),
                    start_offset_ns: 300,
                    end_offset_ns: 420,
                    iteration: None,
                },
            ],
            timeline_lanes: vec![ViewerTraceLane {
                id: "main".into(),
                label: "Main Thread".into(),
                events: vec![
                    ViewerTraceEvent {
                        event_kind: "span".into(),
                        start_offset_ns: 0,
                        end_offset_ns: Some(100),
                        frames: vec!["fixture::setup".into()],
                        phase: Some("setup".into()),
                        iteration: None,
                    },
                    ViewerTraceEvent {
                        event_kind: "sample".into(),
                        start_offset_ns: 140,
                        end_offset_ns: None,
                        frames: vec![
                            "sample_fns::run_benchmark".into(),
                            "sample_fns::fibonacci".into(),
                        ],
                        phase: Some("measured-benchmark".into()),
                        iteration: Some(0),
                    },
                ],
            }],
            timeline_total_duration_ns: Some(420),
            timeline_note: Some(
                "Timeline shows exact harness chronology and available trace samples.".into(),
            ),
            default_mode: FlamegraphMode::Focused,
            artifact_links: vec![ArtifactLink::new("native-report.txt", "native-report.txt")],
            source_links: Vec::new(),
            source_link_note: Some("Source links are unavailable in this fixture.".into()),
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
            &[
                "sample_fns::run_benchmark",
                "mobench_sdk::timing::run_closure",
            ],
        );

        assert_eq!(
            focused.folded,
            "sample_fns::run_benchmark;mobench_sdk::timing::run_closure;sample_fns::fibonacci 3"
        );
    }

    #[test]
    fn standalone_viewer_html_embeds_full_and_focused_modes() {
        let html = render_flamegraph_viewer_html(sample_doc());

        assert!(html.contains("rel=\"icon\""));
        assert!(html.contains("data:image/svg+xml"));
        assert!(html.contains("<title>Mobench Flamegraph - mobile-bench-rs</title>"));
        assert!(html.contains("iOS Native Profile"));
        assert!(html.contains("Benchmark Only"));
        assert!(html.contains("Full Process"));
        assert!(html.contains("Timeline"));
        assert!(html.contains("data-mode=\"focused\""));
        assert!(html.contains("data-mode=\"full\""));
        assert!(html.contains("data-mode=\"timeline\""));
        assert!(html.contains("<svg id=\\\"full\\\"><\\/svg>"));
        assert!(html.contains("<svg id=\\\"focused\\\"><\\/svg>"));
    }

    #[test]
    fn viewer_html_embeds_reference_palette_shell_tokens() {
        let html = render_flamegraph_viewer_html(sample_doc());

        assert!(html.contains("--accent: #ff6b74;"));
        assert!(html.contains("--accent-strong: #f25c68;"));
        assert!(html.contains("--bg: #fffdfb;"));
        assert!(html.contains("font: 14px/1.45 \"Avenir Next\", \"SF Pro Display\""));
    }

    #[test]
    fn viewer_html_includes_history_and_brush_zoom_controls() {
        let html = render_flamegraph_viewer_html(sample_doc());
        assert!(html.contains("id=\"viewer-back\""));
        assert!(html.contains("id=\"viewer-forward\""));
        assert!(html.contains("id=\"viewer-fullscreen\""));
        assert!(html.contains("id=\"viewer-toggle-timeline\""));
        assert!(html.contains("id=\"viewer-legend\""));
        assert!(html.contains("id=\"viewer-reset\""));
        assert!(html.contains("id=\"viewer-search\""));
        assert!(html.contains("id=\"viewer-select-range\" hidden"));
        assert!(html.contains("id=\"selection-overlay\""));
        assert!(html.contains("Drag across the graph to zoom. Double-click to step back one range. Press T to toggle the timeline. Press R to reset. Press F for fullscreen."));
        assert!(html.contains("function stepBackCurrentRange()"));
        assert!(html.contains("function requestStepBackCurrentRange()"));
        assert!(html.contains("function stepForwardCurrentRange()"));
        assert!(html.contains("function toggleFullscreenGraph(force)"));
        assert!(html.contains("function toggleTimelineStrip(force)"));
        assert!(html.contains("function toggleShortcutLegend(force)"));
        assert!(html.contains("function showSelectionHintFor(durationMs)"));
        assert!(html.contains("function scrollAggregateFrameToBase(mode)"));
        assert!(html.contains("function resetZoomFully()"));
        assert!(html.contains("function scrollActiveGraphSurface(deltaX, deltaY)"));
        assert!(html.contains("function isRepeatedOverlayTap(event)"));
        assert!(html.contains("window.addEventListener(\"keydown\""));
        assert!(html.contains("id=\"shortcut-overlay\""));
        assert!(
            html.contains("Keyboard and graph gestures for moving around the profiler quickly.")
        );
        assert!(html.contains("shortcut-badge"));
        assert!(html.contains("showSelectionHintFor(3000);"));
        assert!(html.contains("event.key === \"1\""));
        assert!(html.contains("event.key === \"2\""));
        assert!(html.contains("event.key === \"3\""));
        assert!(html.contains("event.key === \"t\" || event.key === \"T\""));
        assert!(html.contains("event.key === \"j\" || event.key === \"J\""));
        assert!(html.contains("event.key === \"k\" || event.key === \"K\""));
        assert!(html.contains("event.key === \"f\" || event.key === \"F\""));
        assert!(html.contains("event.key === \"Escape\" && fullscreenVisible()"));
        assert!(html.contains("event.key === \"?\" || (event.key === \"/\" && event.shiftKey)"));
        assert!(html.contains("selectionOverlay.addEventListener(\"wheel\""));
        assert!(html.contains("selectionOverlay.addEventListener(\"dblclick\""));
        assert!(html.contains("id=\"timeline-stage\""));
        assert!(html.contains("body.graph-fullscreen .graph-axis"));
        assert!(html.contains("body.graph-fullscreen .harness-timeline"));
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
    fn viewer_html_restores_metadata_harness_and_sampled_time_shell() {
        let html = render_flamegraph_viewer_html(sample_doc());

        assert!(html.contains("Visible Duration"));
        assert!(html.contains("id=\"run-metadata\""));
        assert!(html.contains(
            "grid-template-columns: repeat(var(--run-metadata-columns, 1), minmax(0, 1fr));"
        ));
        assert!(html.contains("function syncRunMetadataColumns()"));
        assert!(html.contains("id=\"harness-timeline\""));
        assert!(html.contains("id=\"harness-readout\""));
        assert!(html.contains("id=\"graph-axis\""));
        assert!(html.contains(".harness-timeline {\n      grid-row: 1;"));
        assert!(html.contains(".graph-stage {\n      grid-row: 2;"));
        assert!(html.contains(".graph-axis {\n      grid-row: 3;"));
    }

    #[test]
    fn viewer_html_tracks_aggregate_dataset_separately_from_timeline_mode() {
        let html = render_flamegraph_viewer_html(sample_doc());

        assert!(html.contains("lastAggregateMode"));
        assert!(html.contains("getAggregateDatasetMode"));
        assert!(html.contains("savedAggregateViews"));
        assert!(html.contains("syncAggregateWindowToTimelineView"));
    }

    #[test]
    fn summarize_folded_stacks_caps_inclusive_percent_for_repeated_frames() {
        let summary =
            summarize_folded_stacks("root;repeat;repeat 4\nroot;repeat;leaf 1\n", 2, 0, None);

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

        assert!(
            summary
                .self_frames
                .iter()
                .any(|frame| frame.frame == "sample_fns::fibonacci")
        );
        assert!(
            summary
                .self_frames
                .iter()
                .any(|frame| frame.frame == "<u32 as core::iter::range::Step>::forward_unchecked")
        );
    }

    #[test]
    fn prettify_folded_stacks_preserves_differential_counts() {
        let pretty =
            prettify_folded_stacks_for_display("root;sample_fns::fibonacci::ha1ebbae54edac99d 3 7");
        assert_eq!(pretty, "root;sample_fns::fibonacci 3 7");
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
        assert!(svg.contains("mobenchScrollToBase"));
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

    #[test]
    fn standalone_svg_retints_neutral_differential_frames() {
        let svg = render_standalone_flamegraph_svg(
            "root;sample_fns::fibonacci 1 1\nroot;sample_fns::checksum 1 1",
            "Diff Flamegraph",
        )
        .expect("render differential svg");
        assert!(!svg.contains("fill=\"rgb(250,250,250)\""));
        assert!(!svg.contains("rgb(217, 215, 255)"));
        assert!(!svg.contains("rgb(232, 228, 255)"));
        assert!(
            svg.contains("rgb(255,107,116)")
                || svg.contains("rgb(255,178,92)")
                || svg.contains("rgb(255,220,107)")
                || svg.contains("rgb(255,207,177)")
        );
    }

    #[test]
    fn standalone_svg_replaces_default_gray_background_gradient() {
        let svg =
            render_standalone_flamegraph_svg("root;sample_fns::fibonacci 1", "Test Flamegraph")
                .expect("render svg");
        assert!(!svg.contains("stop-color=\"#eeeeee\""));
        assert!(!svg.contains("stop-color=\"#eeeeb0\""));
        assert!(svg.contains("stop-color=\"#ffffff\""));
    }
}
