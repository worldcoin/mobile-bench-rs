use anyhow::{Context, Result};
use serde_json::Value;

use crate::reports::{extract_benchmark_resource_usage, extract_samples};
use crate::{BenchmarkStats, DeviceSummary, MobileTarget, SummaryReport, compute_sample_stats};

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
                    summary: serde_json::from_value(summary.clone())
                        .context("parsing summary report")?,
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
                summary: serde_json::from_value(value.clone()).context("parsing summary report")?,
            }],
        })
    }

    pub(crate) fn sections(&self) -> &[BenchmarkOutputSection] {
        &self.sections
    }
}

fn raw_benchmark_report_summary(value: &Value) -> Option<SummaryReport> {
    let spec = value.get("spec")?;
    let function = spec
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| value.get("function").and_then(Value::as_str))?
        .to_string();
    let iterations = spec.get("iterations").and_then(Value::as_u64).unwrap_or(0) as u32;
    let warmup = spec.get("warmup").and_then(Value::as_u64).unwrap_or(0) as u32;

    let samples = extract_samples(value);
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

    Some(SummaryReport {
        generated_at: "raw benchmark report".to_string(),
        generated_at_unix: 0,
        target: MobileTarget::Android,
        function,
        iterations,
        warmup,
        devices: vec!["local".to_string()],
        device_summaries: vec![DeviceSummary {
            device: "local".to_string(),
            benchmarks: vec![benchmark],
        }],
    })
}
