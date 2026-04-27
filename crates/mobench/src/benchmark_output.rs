use anyhow::{Context, Result};
use serde_json::Value;

use crate::reports::{extract_benchmark_resource_usage, extract_samples};
use crate::{BenchmarkStats, DeviceSummary, MobileTarget, SummaryReport, compute_sample_stats};

pub(crate) const ANDROID_BENCH_LOG_MARKER: &str = "BENCH_JSON";

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkOutput {
    sections: Vec<BenchmarkOutputSection>,
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkOutputSection {
    pub(crate) heading: Option<String>,
    pub(crate) summary: SummaryReport,
}

impl BenchmarkOutput {
    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        if let Some(summary) = value.get("summary") {
            return Ok(Self {
                sections: vec![BenchmarkOutputSection {
                    heading: None,
                    summary: parse_summary_report(summary)?,
                }],
            });
        }

        if let Some(targets) = value.get("targets").and_then(|targets| targets.as_object()) {
            let mut target_names = targets.keys().cloned().collect::<Vec<_>>();
            target_names.sort();

            let mut sections = Vec::new();
            for name in target_names {
                let Some(entry) = targets.get(&name) else {
                    continue;
                };
                let output = Self::from_value(entry).with_context(|| {
                    format!("parsing benchmark output for target `{name}` in merged output")
                })?;
                sections.extend(output.sections.into_iter().map(|mut section| {
                    section.heading = Some(match section.heading {
                        Some(heading) => format!("{name} / {heading}"),
                        None => name.clone(),
                    });
                    section
                }));
            }
            if !sections.is_empty() {
                return Ok(Self { sections });
            }
        }

        if let Some(summary) = raw_benchmark_report_summary(value) {
            return Ok(Self {
                sections: vec![BenchmarkOutputSection {
                    heading: None,
                    summary,
                }],
            });
        }

        Ok(Self {
            sections: vec![BenchmarkOutputSection {
                heading: None,
                summary: parse_summary_report(value)?,
            }],
        })
    }

    pub(crate) fn sections(&self) -> &[BenchmarkOutputSection] {
        &self.sections
    }
}

fn parse_summary_report(value: &Value) -> Result<SummaryReport> {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object
            .entry("generated_at")
            .or_insert_with(|| Value::String("unknown".to_string()));
        object
            .entry("generated_at_unix")
            .or_insert_with(|| Value::from(0_u64));
    }
    serde_json::from_value(value).context("parsing summary report")
}

#[derive(Debug)]
struct RawBenchmarkEntry {
    target: MobileTarget,
    device: String,
    function: String,
    iterations: u32,
    warmup: u32,
    benchmark: BenchmarkStats,
}

fn raw_benchmark_report_summary(value: &Value) -> Option<SummaryReport> {
    let entries = match value {
        Value::Array(items) => items
            .iter()
            .filter_map(raw_benchmark_entry)
            .collect::<Vec<_>>(),
        _ => raw_benchmark_entry(value).into_iter().collect(),
    };

    let first = entries.first()?;
    let mut device_summaries = Vec::<DeviceSummary>::new();
    let mut devices = Vec::<String>::new();
    let mut functions = Vec::<String>::new();

    for entry in &entries {
        if !devices.contains(&entry.device) {
            devices.push(entry.device.clone());
            device_summaries.push(DeviceSummary {
                device: entry.device.clone(),
                benchmarks: Vec::new(),
            });
        }
        if !functions.contains(&entry.function) {
            functions.push(entry.function.clone());
        }
        if let Some(device_summary) = device_summaries
            .iter_mut()
            .find(|summary| summary.device == entry.device)
        {
            device_summary.benchmarks.push(entry.benchmark.clone());
        }
    }

    let function = if functions.len() == 1 {
        functions.remove(0)
    } else {
        "multiple".to_string()
    };

    Some(SummaryReport {
        generated_at: "raw benchmark report".to_string(),
        generated_at_unix: 0,
        target: first.target,
        function,
        iterations: first.iterations,
        warmup: first.warmup,
        devices,
        device_summaries,
    })
}

fn raw_benchmark_entry(value: &Value) -> Option<RawBenchmarkEntry> {
    let spec = value.get("spec");
    let function = spec
        .and_then(|spec| spec.get("name"))
        .and_then(Value::as_str)
        .or_else(|| value.get("function").and_then(Value::as_str))?
        .to_string();
    let iterations = spec
        .and_then(|spec| spec.get("iterations"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let warmup = spec
        .and_then(|spec| spec.get("warmup"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let device = value
        .get("device")
        .and_then(Value::as_str)
        .unwrap_or("local")
        .to_string();

    let samples = extract_raw_samples(value);
    let has_explicit_stats = ["mean_ns", "median_ns", "p95_ns", "min_ns", "max_ns"]
        .iter()
        .any(|key| value.get(*key).and_then(Value::as_u64).is_some());
    if samples.is_empty() && !has_explicit_stats {
        return None;
    }
    let stats = compute_sample_stats(&samples);
    let benchmark = BenchmarkStats {
        function: function.clone(),
        samples: samples.len(),
        mean_ns: stats
            .as_ref()
            .map(|stats| stats.mean_ns)
            .or_else(|| value.get("mean_ns").and_then(Value::as_u64)),
        median_ns: stats
            .as_ref()
            .map(|stats| stats.median_ns)
            .or_else(|| value.get("median_ns").and_then(Value::as_u64)),
        p95_ns: stats
            .as_ref()
            .map(|stats| stats.p95_ns)
            .or_else(|| value.get("p95_ns").and_then(Value::as_u64)),
        min_ns: stats
            .as_ref()
            .map(|stats| stats.min_ns)
            .or_else(|| value.get("min_ns").and_then(Value::as_u64)),
        max_ns: stats
            .as_ref()
            .map(|stats| stats.max_ns)
            .or_else(|| value.get("max_ns").and_then(Value::as_u64)),
        resource_usage: extract_benchmark_resource_usage(value, None),
    };

    Some(RawBenchmarkEntry {
        target: raw_benchmark_target(value, &device),
        device,
        function,
        iterations,
        warmup,
        benchmark,
    })
}

fn extract_raw_samples(value: &Value) -> Vec<u64> {
    let mut samples = extract_samples(value);
    if samples.is_empty()
        && let Some(samples_ns) = value.get("samples_ns").and_then(Value::as_array)
    {
        samples.extend(samples_ns.iter().filter_map(Value::as_u64));
    }
    samples
}

fn raw_benchmark_target(value: &Value, device: &str) -> MobileTarget {
    if let Some(target) = value
        .get("target")
        .or_else(|| value.get("spec").and_then(|spec| spec.get("target")))
        .and_then(Value::as_str)
    {
        return match target.to_ascii_lowercase().as_str() {
            "ios" => MobileTarget::Ios,
            _ => MobileTarget::Android,
        };
    }

    let device = device.to_ascii_lowercase();
    if device.contains("iphone") || device.contains("ipad") || device.contains("ios") {
        MobileTarget::Ios
    } else {
        MobileTarget::Android
    }
}

pub(crate) fn extract_benchmark_reports_from_logs(logs: &str) -> Vec<Value> {
    let mut results = Vec::new();
    if let Some(json) = extract_ios_benchmark_json(logs) {
        results.push(json);
    }

    let marker = "BENCH_JSON ";
    for line in logs.lines() {
        if let Some(index) = line.find(marker) {
            let json_part = &line[index + marker.len()..];
            if let Ok(parsed) = serde_json::from_str::<Value>(json_part) {
                results.push(parsed);
            }
        }
    }

    results
}

pub(crate) fn extract_ios_benchmark_json(logs: &str) -> Option<Value> {
    let start_marker = "BENCH_REPORT_JSON_START";
    let end_marker = "BENCH_REPORT_JSON_END";
    let start_pos = logs.rfind(start_marker)?;
    let after_start = &logs[start_pos + start_marker.len()..];
    let end_pos = after_start.find(end_marker)?;
    extract_ios_json_from_log_section(&after_start[..end_pos])
}

fn extract_ios_json_from_log_section(section: &str) -> Option<Value> {
    let trimmed = section.trim();
    if trimmed.starts_with('{')
        && trimmed.ends_with('}')
        && let Ok(parsed) = serde_json::from_str::<Value>(trimmed)
    {
        return Some(parsed);
    }

    for line in section.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(json_start) = line.find('{')
            && let Some(json) = extract_balanced_json(&line[json_start..])
            && let Ok(parsed) = serde_json::from_str::<Value>(&json)
        {
            return Some(parsed);
        }
    }

    let collapsed: String = section
        .lines()
        .map(|line| {
            if let Some(prefix_end) = line.find("] ") {
                &line[prefix_end + 2..]
            } else {
                line.trim()
            }
        })
        .collect::<Vec<_>>()
        .join("");
    let json_start = collapsed.find('{')?;
    let json = extract_balanced_json(&collapsed[json_start..])?;
    serde_json::from_str(&json).ok()
}

fn extract_balanced_json(input: &str) -> Option<String> {
    if !input.starts_with('{') {
        return None;
    }

    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;
    for (index, ch) in input.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(input[..=index].to_string());
                }
            }
            _ => {}
        }
    }

    None
}

pub(crate) fn benchmark_value_function(value: &Value) -> Option<&str> {
    value.get("function").and_then(Value::as_str).or_else(|| {
        value
            .get("spec")
            .and_then(|spec| spec.get("name"))
            .and_then(Value::as_str)
    })
}

pub(crate) fn select_benchmark_value_for_function<'a>(
    values: &'a [Value],
    function: &str,
) -> Option<&'a Value> {
    let simple_name = function.split("::").last().unwrap_or(function);
    values
        .iter()
        .rev()
        .find(|value| {
            benchmark_value_function(value).is_some_and(|name| {
                name == function
                    || name == simple_name
                    || name.ends_with(&format!("::{simple_name}"))
                    || function.ends_with(&format!("::{name}"))
            })
        })
        .or_else(|| values.last())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_reports_select_requested_benchmark_function() {
        let reports = extract_benchmark_reports_from_logs(
            "03-26 19:00:00.000 BenchRunner I BENCH_JSON {\"function\":\"sample_fns::checksum\",\"samples_ns\":[1]}\n\
             03-26 19:00:01.000 BenchRunner I BENCH_JSON {\"function\":\"sample_fns::fibonacci\",\"samples_ns\":[2]}",
        );

        let selected = select_benchmark_value_for_function(&reports, "sample_fns::fibonacci")
            .expect("selected benchmark report");

        assert_eq!(
            benchmark_value_function(selected),
            Some("sample_fns::fibonacci")
        );
    }
}
