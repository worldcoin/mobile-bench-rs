//! Types and logic for the `ci summarize` command.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
        std_dev_ms: 0.0,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
