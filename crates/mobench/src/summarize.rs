//! Types and logic for the `ci summarize` command.

use anyhow::{Context, Result};
use comfy_table::{presets::UTF8_FULL, Attribute, Cell, ContentArrangement, Table};
use serde::{Deserialize, Serialize};
use std::path::Path;

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// A fully-assembled summary ready for rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizeReport {
    pub platforms: Vec<PlatformReport>,
}

/// Results for a single platform (iOS or Android).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformReport {
    pub platform: String,
    pub device: DeviceInfo,
    pub benchmarks: Vec<BenchmarkResult>,
    pub iterations: u32,
    pub warmup: u32,
}

/// Device information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub os: String,
    pub os_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chipset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_gb: Option<f64>,
}

/// Aggregated result for a single benchmark function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub name: String,
    pub label: String,
    pub timing: TimingStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_usage: Option<ResourceUsage>,
}

/// Timing statistics across all iterations (in milliseconds).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingStats {
    pub avg_ms: f64,
    pub median_ms: f64,
    pub best_ms: f64,
    pub worst_ms: f64,
    pub p95_ms: f64,
    #[serde(skip_serializing_if = "is_zero")]
    pub std_dev_ms: f64,
}

/// Resource usage metrics from BrowserStack session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub cpu_avg_percent: Option<f64>,
    pub cpu_peak_percent: Option<f64>,
    pub ram_avg_mb: Option<f64>,
    pub ram_peak_mb: Option<f64>,
}

/// Parse a summary.json value into a [`SummarizeReport`].
pub fn parse_summary_value(value: &serde_json::Value) -> Result<SummarizeReport> {
    let summary = value
        .get("summary")
        .context("Missing 'summary' key in JSON")?;

    let target = summary
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let iterations = summary
        .get("iterations")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let warmup = summary
        .get("warmup")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let device_summaries = summary
        .get("device_summaries")
        .and_then(|v| v.as_array())
        .context("Missing 'device_summaries'")?;

    let mut platforms = Vec::new();

    for ds in device_summaries {
        let device_str = ds
            .get("device")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let device = parse_device_string(device_str);

        let benchmarks = ds
            .get("benchmarks")
            .and_then(|v| v.as_array())
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|b| parse_benchmark_entry(b).ok())
            .collect();

        platforms.push(PlatformReport {
            platform: target.clone(),
            device,
            benchmarks,
            iterations,
            warmup,
        });
    }

    Ok(SummarizeReport { platforms })
}

fn parse_device_string(s: &str) -> DeviceInfo {
    let (name, os_version) = match s.rsplit_once('-') {
        Some((n, v)) => (n.to_string(), v.to_string()),
        None => (s.to_string(), "unknown".to_string()),
    };

    let os = if name.contains("iPhone") || name.contains("iPad") {
        "iOS".to_string()
    } else {
        "Android".to_string()
    };

    DeviceInfo {
        name,
        os,
        os_version,
        chipset: None,
        ram_gb: None,
    }
}

fn parse_benchmark_entry(value: &serde_json::Value) -> Result<BenchmarkResult> {
    let name = value
        .get("function")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let label = humanize_benchmark_name(&name);

    let ns_to_ms = |key: &str| -> f64 {
        value
            .get(key)
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            / 1_000_000.0
    };

    let timing = TimingStats {
        avg_ms: ns_to_ms("mean_ns"),
        median_ms: ns_to_ms("median_ns"),
        best_ms: ns_to_ms("min_ns"),
        worst_ms: ns_to_ms("max_ns"),
        p95_ms: ns_to_ms("p95_ns"),
        std_dev_ms: ns_to_ms("std_dev_ns"),
    };

    Ok(BenchmarkResult {
        name,
        label,
        timing,
        resource_usage: None,
    })
}

fn humanize_benchmark_name(name: &str) -> String {
    let s = name
        .replace("bench_", "")
        .replace("_generation", "")
        .replace("_only", "");

    if s.contains("nullifier") {
        format!("\u{03C0}2 {}", s.replace('_', "-"))
    } else if s.contains("query") {
        format!("\u{03C0}1 {}", s.replace('_', "-"))
    } else {
        s.replace('_', "-")
    }
}

/// Load all summary JSON files from a results directory.
pub fn load_results_dir(dir: &Path) -> Result<SummarizeReport> {
    let mut all_platforms = Vec::new();

    for entry in std::fs::read_dir(dir).context("Failed to read results directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            let value: serde_json::Value = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse {}", path.display()))?;

            if let Ok(report) = parse_summary_value(&value) {
                all_platforms.extend(report.platforms);
            }
        }
    }

    if all_platforms.is_empty() {
        anyhow::bail!("No valid summary JSON files found in {}", dir.display());
    }

    Ok(SummarizeReport {
        platforms: all_platforms,
    })
}

/// Render the full report as terminal tables.
pub fn render_table(report: &SummarizeReport) -> String {
    let mut output = String::new();

    for platform in &report.platforms {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&render_platform_table(platform));
    }

    output
}

fn render_platform_table(platform: &PlatformReport) -> String {
    let mut output = String::new();

    // Header line
    let mut header = format!(
        "{} — {} ({} {})",
        platform.platform.to_uppercase(),
        platform.device.name,
        platform.device.os,
        platform.device.os_version,
    );
    if let Some(chipset) = &platform.device.chipset {
        header.push_str(&format!(" · {chipset}"));
    }
    if let Some(ram) = platform.device.ram_gb {
        header.push_str(&format!(" · {ram} GB RAM"));
    }
    output.push_str(&header);
    output.push('\n');

    let has_resource_usage = platform
        .benchmarks
        .iter()
        .any(|b| b.resource_usage.is_some());

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);

    let mut headers = vec!["Benchmark", "Avg ms", "Best", "Worst", "Median", "P95"];
    if has_resource_usage {
        headers.extend(["CPU %", "RAM MB"]);
    }
    table.set_header(
        headers
            .iter()
            .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
    );

    for bench in &platform.benchmarks {
        let mut row = vec![
            Cell::new(&bench.label),
            Cell::new(format!("{:.1}", bench.timing.avg_ms)).add_attribute(Attribute::Bold),
            Cell::new(format!("{:.1}", bench.timing.best_ms)),
            Cell::new(format!("{:.1}", bench.timing.worst_ms)),
            Cell::new(format!("{:.1}", bench.timing.median_ms)),
            Cell::new(format!("{:.1}", bench.timing.p95_ms)),
        ];

        if has_resource_usage {
            if let Some(ru) = &bench.resource_usage {
                row.push(Cell::new(
                    ru.cpu_avg_percent
                        .map(|v| format!("{v:.0}%"))
                        .unwrap_or_else(|| "—".to_string()),
                ));
                row.push(Cell::new(
                    ru.ram_avg_mb
                        .map(|v| format!("{v:.0}"))
                        .unwrap_or_else(|| "—".to_string()),
                ));
            } else {
                row.push(Cell::new("—"));
                row.push(Cell::new("—"));
            }
        }

        table.add_row(row);
    }

    output.push_str(&table.to_string());
    output.push_str(&format!(
        "\n  {} iterations · {} warmup · avg is primary metric\n",
        platform.iterations, platform.warmup
    ));

    output
}

/// Render the report as a markdown table.
pub fn render_markdown(report: &SummarizeReport) -> String {
    let mut output = String::new();

    for platform in &report.platforms {
        if !output.is_empty() {
            output.push('\n');
        }

        let mut header = format!(
            "### {} — {} ({} {})",
            platform.platform.to_uppercase(),
            platform.device.name,
            platform.device.os,
            platform.device.os_version,
        );
        if let Some(chipset) = &platform.device.chipset {
            header.push_str(&format!(" · {chipset}"));
        }
        if let Some(ram) = platform.device.ram_gb {
            header.push_str(&format!(" · {ram} GB RAM"));
        }
        output.push_str(&header);
        output.push_str("\n\n");

        let has_ru = platform
            .benchmarks
            .iter()
            .any(|b| b.resource_usage.is_some());

        if has_ru {
            output.push_str(
                "| Benchmark | Avg ms | Best | Worst | Median | P95 | CPU % | RAM MB |\n",
            );
            output.push_str(
                "|-----------|--------|------|-------|--------|-----|-------|--------|\n",
            );
        } else {
            output.push_str("| Benchmark | Avg ms | Best | Worst | Median | P95 |\n");
            output.push_str("|-----------|--------|------|-------|--------|-----|\n");
        }

        for bench in &platform.benchmarks {
            let mut row = format!(
                "| {} | **{:.1}** | {:.1} | {:.1} | {:.1} | {:.1} |",
                bench.label,
                bench.timing.avg_ms,
                bench.timing.best_ms,
                bench.timing.worst_ms,
                bench.timing.median_ms,
                bench.timing.p95_ms,
            );

            if has_ru {
                if let Some(ru) = &bench.resource_usage {
                    row.push_str(&format!(
                        " {} | {} |",
                        ru.cpu_avg_percent
                            .map(|v| format!("{v:.0}%"))
                            .unwrap_or_else(|| "—".into()),
                        ru.ram_avg_mb
                            .map(|v| format!("{v:.0}"))
                            .unwrap_or_else(|| "—".into()),
                    ));
                } else {
                    row.push_str(" — | — |");
                }
            }

            output.push_str(&row);
            output.push('\n');
        }

        output.push_str(&format!(
            "\n*{} iterations · {} warmup · avg is primary metric*\n",
            platform.iterations, platform.warmup
        ));
    }

    output
}

/// Enrich an offline report with BrowserStack session metrics.
pub fn enrich_with_browserstack(
    report: &mut SummarizeReport,
    build_summary: &crate::browserstack::BuildSummary,
) {
    for platform in &mut report.platforms {
        let platform_sessions: Vec<_> = build_summary
            .sessions
            .iter()
            .filter(|session| session_matches_platform(platform, session))
            .collect();
        let Some(session) = select_session_for_platform(platform, &platform_sessions) else {
            continue;
        };

        if !session.os.is_empty() {
            platform.device.os = session.os.clone();
            platform.device.os_version = session.os_version.clone();
            if platform.device.name == "unknown" && !session.device.is_empty() {
                platform.device.name = session.device.clone();
            }
        }

        if let Some(perf) = &session.performance {
            for bench in &mut platform.benchmarks {
                if bench.resource_usage.is_none() {
                    bench.resource_usage = Some(ResourceUsage {
                        cpu_avg_percent: perf.cpu.as_ref().map(|c| c.average_percent),
                        cpu_peak_percent: perf.cpu.as_ref().map(|c| c.peak_percent),
                        ram_avg_mb: perf.memory.as_ref().map(|m| m.average_mb),
                        ram_peak_mb: perf.memory.as_ref().map(|m| m.peak_mb),
                    });
                }
            }
        }
    }
}

fn session_matches_platform(
    platform: &PlatformReport,
    session: &crate::browserstack::SessionSummary,
) -> bool {
    let session_is_ios = session.os.eq_ignore_ascii_case("ios")
        || session.os.eq_ignore_ascii_case("iphone")
        || session.os.eq_ignore_ascii_case("ipad");
    let platform_is_ios = platform.platform.eq_ignore_ascii_case("ios");
    session_is_ios == platform_is_ios
}

fn select_session_for_platform<'a>(
    platform: &PlatformReport,
    sessions: &[&'a crate::browserstack::SessionSummary],
) -> Option<&'a crate::browserstack::SessionSummary> {
    let platform_device = normalize_device_match_key(&platform.device.name);
    let platform_os_version = normalize_version_match_key(&platform.device.os_version);

    let matched = sessions
        .iter()
        .filter_map(|session| {
            let score = device_match_score(&platform_device, &platform_os_version, session);
            (score > 0).then_some((score, *session))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, session)| session);

    matched.or_else(|| (sessions.len() == 1).then_some(sessions[0]))
}

fn device_match_score(
    platform_device: &str,
    platform_os_version: &str,
    session: &crate::browserstack::SessionSummary,
) -> usize {
    let session_device = normalize_device_match_key(&session.device);
    let session_os_version = normalize_version_match_key(&session.os_version);

    let device_score = if platform_device.is_empty() || session_device.is_empty() {
        0
    } else if platform_device == session_device {
        100
    } else if platform_device.contains(&session_device) || session_device.contains(platform_device) {
        75
    } else if token_subset_match(platform_device, &session_device)
        || token_subset_match(&session_device, platform_device)
    {
        50
    } else {
        0
    };

    if device_score == 0 {
        return 0;
    }

    let version_score =
        usize::from(!platform_os_version.is_empty() && platform_os_version == session_os_version)
            * 10;

    device_score + version_score
}

fn normalize_device_match_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_version_match_key(value: &str) -> String {
    let mut parts: Vec<&str> = value
        .trim()
        .split('.')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    while parts.last() == Some(&"0") {
        parts.pop();
    }
    parts.join(".")
}

fn token_subset_match(left: &str, right: &str) -> bool {
    let left_tokens: Vec<&str> = left.split_whitespace().collect();
    let right_tokens: Vec<&str> = right.split_whitespace().collect();

    !left_tokens.is_empty()
        && left_tokens
            .iter()
            .all(|left_token| right_tokens.iter().any(|right_token| right_token == left_token))
}

/// Render the report as JSON.
pub fn render_json(report: &SummarizeReport) -> Result<String> {
    serde_json::to_string_pretty(report).context("Failed to serialize report as JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browserstack::{
        AggregateCpuMetrics, AggregateMemoryMetrics, BuildSummary, PerformanceMetrics,
        SessionSummary,
    };

    fn sample_summary_json() -> serde_json::Value {
        serde_json::json!({
            "summary": {
                "generated_at": "2026-02-26T12:00:00Z",
                "target": "ios",
                "function": "bench_nullifier_proving_only",
                "iterations": 30,
                "warmup": 5,
                "devices": ["iPhone 14-16.0"],
                "device_summaries": [{
                    "device": "iPhone 14-16.0",
                    "benchmarks": [{
                        "function": "bench_nullifier_proving_only",
                        "samples": 30,
                        "mean_ns": 1204500000_u64,
                        "median_ns": 1198000000_u64,
                        "p95_ns": 1290000000_u64,
                        "min_ns": 1180200000_u64,
                        "max_ns": 1298100000_u64
                    }]
                }]
            }
        })
    }

    #[test]
    fn test_parse_summary_json() {
        let json = sample_summary_json();
        let report = parse_summary_value(&json).unwrap();
        assert_eq!(report.platforms.len(), 1);
        let p = &report.platforms[0];
        assert_eq!(p.platform, "ios");
        assert_eq!(p.iterations, 30);
        assert_eq!(p.warmup, 5);
        assert_eq!(p.benchmarks.len(), 1);
        let b = &p.benchmarks[0];
        assert!((b.timing.avg_ms - 1204.5).abs() < 0.1);
        assert!((b.timing.best_ms - 1180.2).abs() < 0.1);
        assert!((b.timing.worst_ms - 1298.1).abs() < 0.1);
    }

    #[test]
    fn test_parse_device_string_ios() {
        let d = parse_device_string("iPhone 14-16.0");
        assert_eq!(d.name, "iPhone 14");
        assert_eq!(d.os, "iOS");
        assert_eq!(d.os_version, "16.0");
    }

    #[test]
    fn test_parse_device_string_android() {
        let d = parse_device_string("Google Pixel 6-12.0");
        assert_eq!(d.name, "Google Pixel 6");
        assert_eq!(d.os, "Android");
        assert_eq!(d.os_version, "12.0");
    }

    #[test]
    fn test_humanize_benchmark_name() {
        assert_eq!(
            humanize_benchmark_name("bench_nullifier_proving_only"),
            "\u{03C0}2 nullifier-proving"
        );
        assert_eq!(
            humanize_benchmark_name("bench_query_proof_generation"),
            "\u{03C0}1 query-proof"
        );
    }

    #[test]
    fn test_load_results_dir() {
        let dir = tempfile::tempdir().unwrap();
        let json = sample_summary_json();
        std::fs::write(
            dir.path().join("test.json"),
            serde_json::to_string(&json).unwrap(),
        )
        .unwrap();

        let report = load_results_dir(dir.path()).unwrap();
        assert_eq!(report.platforms.len(), 1);
        assert_eq!(report.platforms[0].platform, "ios");
    }

    #[test]
    fn test_load_results_dir_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_results_dir(dir.path()).is_err());
    }

    #[test]
    fn test_render_table_output() {
        let report = SummarizeReport {
            platforms: vec![PlatformReport {
                platform: "ios".to_string(),
                device: DeviceInfo {
                    name: "iPhone 14".to_string(),
                    os: "iOS".to_string(),
                    os_version: "16.0".to_string(),
                    chipset: Some("A15 Bionic".to_string()),
                    ram_gb: Some(6.0),
                },
                benchmarks: vec![BenchmarkResult {
                    name: "bench_nullifier_proving_only".to_string(),
                    label: "\u{03C0}2 nullifier-proving".to_string(),
                    timing: TimingStats {
                        avg_ms: 1204.5,
                        median_ms: 1198.0,
                        best_ms: 1180.2,
                        worst_ms: 1298.1,
                        p95_ms: 1290.0,
                        std_dev_ms: 35.2,
                    },
                    resource_usage: Some(ResourceUsage {
                        cpu_avg_percent: Some(94.0),
                        cpu_peak_percent: Some(98.0),
                        ram_avg_mb: Some(623.0),
                        ram_peak_mb: Some(650.0),
                    }),
                }],
                iterations: 30,
                warmup: 5,
            }],
        };
        let output = render_table(&report);
        assert!(output.contains("iPhone 14"));
        assert!(output.contains("1204.5"));
        assert!(output.contains("A15 Bionic"));
        assert!(output.contains("6 GB RAM"));
    }

    #[test]
    fn test_render_markdown_output() {
        let report = SummarizeReport {
            platforms: vec![PlatformReport {
                platform: "ios".to_string(),
                device: DeviceInfo {
                    name: "iPhone 14".to_string(),
                    os: "iOS".to_string(),
                    os_version: "16.0".to_string(),
                    chipset: None,
                    ram_gb: None,
                },
                benchmarks: vec![BenchmarkResult {
                    name: "bench_nullifier_proving_only".to_string(),
                    label: "\u{03C0}2 nullifier-proving".to_string(),
                    timing: TimingStats {
                        avg_ms: 1204.5,
                        median_ms: 1198.0,
                        best_ms: 1180.2,
                        worst_ms: 1298.1,
                        p95_ms: 1290.0,
                        std_dev_ms: 35.2,
                    },
                    resource_usage: None,
                }],
                iterations: 30,
                warmup: 5,
            }],
        };
        let output = render_markdown(&report);
        assert!(output.contains("### IOS"));
        assert!(output.contains("**1204.5**"));
        assert!(output.contains("| Benchmark |"));
    }

    #[test]
    fn test_render_json_output() {
        let report = SummarizeReport {
            platforms: vec![PlatformReport {
                platform: "ios".to_string(),
                device: DeviceInfo {
                    name: "iPhone 14".to_string(),
                    os: "iOS".to_string(),
                    os_version: "16.0".to_string(),
                    chipset: None,
                    ram_gb: None,
                },
                benchmarks: vec![],
                iterations: 30,
                warmup: 5,
            }],
        };
        let json_str = render_json(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["platforms"][0]["platform"], "ios");
    }

    #[test]
    fn test_enrich_with_browserstack_matches_device_names() {
        let mut report = SummarizeReport {
            platforms: vec![
                PlatformReport {
                    platform: "ios".to_string(),
                    device: DeviceInfo {
                        name: "iPhone 14".to_string(),
                        os: "iOS".to_string(),
                        os_version: "16.0".to_string(),
                        chipset: None,
                        ram_gb: None,
                    },
                    benchmarks: vec![BenchmarkResult {
                        name: "bench_nullifier_proving_only".to_string(),
                        label: "\u{03C0}2 nullifier-proving".to_string(),
                        timing: TimingStats {
                            avg_ms: 1204.5,
                            median_ms: 1198.0,
                            best_ms: 1180.2,
                            worst_ms: 1298.1,
                            p95_ms: 1290.0,
                            std_dev_ms: 35.2,
                        },
                        resource_usage: None,
                    }],
                    iterations: 30,
                    warmup: 5,
                },
                PlatformReport {
                    platform: "ios".to_string(),
                    device: DeviceInfo {
                        name: "iPhone 15".to_string(),
                        os: "iOS".to_string(),
                        os_version: "17.0".to_string(),
                        chipset: None,
                        ram_gb: None,
                    },
                    benchmarks: vec![BenchmarkResult {
                        name: "bench_query_proof_generation".to_string(),
                        label: "\u{03C0}1 query-proof".to_string(),
                        timing: TimingStats {
                            avg_ms: 802.3,
                            median_ms: 798.0,
                            best_ms: 780.2,
                            worst_ms: 840.1,
                            p95_ms: 835.0,
                            std_dev_ms: 18.4,
                        },
                        resource_usage: None,
                    }],
                    iterations: 30,
                    warmup: 5,
                },
            ],
        };
        let build_summary = BuildSummary {
            build_id: "build-123".to_string(),
            status: "done".to_string(),
            sessions: vec![
                SessionSummary {
                    session_id: "session-15".to_string(),
                    device: "iPhone 15".to_string(),
                    os: "iOS".to_string(),
                    os_version: "17.0".to_string(),
                    duration_secs: Some(120),
                    performance: Some(PerformanceMetrics {
                        sample_count: 3,
                        memory: Some(AggregateMemoryMetrics {
                            peak_mb: 750.0,
                            average_mb: 700.0,
                            min_mb: 680.0,
                        }),
                        cpu: Some(AggregateCpuMetrics {
                            peak_percent: 92.0,
                            average_percent: 81.0,
                            min_percent: 74.0,
                        }),
                        snapshots: Vec::new(),
                    }),
                },
                SessionSummary {
                    session_id: "session-14".to_string(),
                    device: "iPhone 14".to_string(),
                    os: "iOS".to_string(),
                    os_version: "16.0".to_string(),
                    duration_secs: Some(115),
                    performance: Some(PerformanceMetrics {
                        sample_count: 3,
                        memory: Some(AggregateMemoryMetrics {
                            peak_mb: 550.0,
                            average_mb: 500.0,
                            min_mb: 480.0,
                        }),
                        cpu: Some(AggregateCpuMetrics {
                            peak_percent: 55.0,
                            average_percent: 44.0,
                            min_percent: 32.0,
                        }),
                        snapshots: Vec::new(),
                    }),
                },
            ],
        };

        enrich_with_browserstack(&mut report, &build_summary);

        let iphone_14 = &report.platforms[0];
        let iphone_15 = &report.platforms[1];

        assert_eq!(iphone_14.device.os_version, "16.0");
        assert_eq!(
            iphone_14.benchmarks[0]
                .resource_usage
                .as_ref()
                .and_then(|usage| usage.cpu_avg_percent),
            Some(44.0)
        );
        assert_eq!(
            iphone_14.benchmarks[0]
                .resource_usage
                .as_ref()
                .and_then(|usage| usage.ram_avg_mb),
            Some(500.0)
        );

        assert_eq!(iphone_15.device.os_version, "17.0");
        assert_eq!(
            iphone_15.benchmarks[0]
                .resource_usage
                .as_ref()
                .and_then(|usage| usage.cpu_avg_percent),
            Some(81.0)
        );
        assert_eq!(
            iphone_15.benchmarks[0]
                .resource_usage
                .as_ref()
                .and_then(|usage| usage.ram_avg_mb),
            Some(700.0)
        );
    }
}
