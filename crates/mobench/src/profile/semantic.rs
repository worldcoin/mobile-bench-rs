use anyhow::Result;
use serde_json::Value;

use super::{
    HarnessTimelineSpanRecord, ProfileManifest, SemanticCaptureStatus, SemanticPhaseRecord,
};

pub(super) fn merge_from_bench_report(
    manifest: &mut ProfileManifest,
    bench_report: &Value,
) -> Result<()> {
    populate_from_benchmark_value(manifest, bench_report);
    Ok(())
}

pub(super) fn populate_from_benchmark_value(
    manifest: &mut ProfileManifest,
    benchmark_value: &Value,
) {
    if let Some(spec) = benchmark_value.get("spec") {
        manifest.capture_metadata.benchmark_iterations = spec
            .get("iterations")
            .and_then(Value::as_u64)
            .map(|value| value as u32);
        manifest.capture_metadata.benchmark_warmup = spec
            .get("warmup")
            .and_then(Value::as_u64)
            .map(|value| value as u32);
    }

    if let Some(timeline) = benchmark_value.get("timeline").and_then(Value::as_array) {
        manifest.semantic_profile.harness_timeline = timeline
            .iter()
            .filter_map(|span| {
                Some(HarnessTimelineSpanRecord {
                    phase: span.get("phase")?.as_str()?.to_string(),
                    start_offset_ns: span.get("start_offset_ns")?.as_u64()?,
                    end_offset_ns: span.get("end_offset_ns")?.as_u64()?,
                    iteration: span
                        .get("iteration")
                        .and_then(Value::as_u64)
                        .map(|value| value as u32),
                })
            })
            .collect();
    }

    let Some(phases) = benchmark_value.get("phases").and_then(Value::as_array) else {
        return;
    };

    let phase_duration_total_ns: u64 = phases
        .iter()
        .filter_map(|phase| phase.get("duration_ns").and_then(Value::as_u64))
        .sum();
    let sample_duration_total_ns = benchmark_value_sample_duration_total_ns(benchmark_value);
    let total_duration_ns = if sample_duration_total_ns > 0 {
        sample_duration_total_ns
    } else {
        phase_duration_total_ns
    };

    let mut semantic_phases = Vec::new();
    let mut partial = false;
    for phase in phases {
        let Some(name) = phase.get("name").and_then(Value::as_str) else {
            partial = true;
            continue;
        };
        let duration_ns = phase.get("duration_ns").and_then(Value::as_u64);
        let percent_total = duration_ns.and_then(|duration_ns| {
            (total_duration_ns > 0).then_some(
                (duration_ns.saturating_mul(100) + (total_duration_ns / 2)) / total_duration_ns,
            )
        });
        if duration_ns.is_none() {
            partial = true;
        }
        semantic_phases.push(SemanticPhaseRecord {
            name: name.to_string(),
            duration_ns,
            percent_total,
        });
    }

    if semantic_phases.is_empty() {
        return;
    }

    manifest.semantic_profile.status = if partial {
        SemanticCaptureStatus::Partial
    } else {
        SemanticCaptureStatus::Captured
    };
    manifest.semantic_profile.phases = semantic_phases;
}

fn benchmark_value_sample_duration_total_ns(benchmark_value: &Value) -> u64 {
    let sample_objects_total_ns: u64 = benchmark_value
        .get("samples")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|sample| sample.get("duration_ns").and_then(Value::as_u64))
        .sum();
    if sample_objects_total_ns > 0 {
        return sample_objects_total_ns;
    }

    benchmark_value
        .get("samples_ns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .sum()
}
