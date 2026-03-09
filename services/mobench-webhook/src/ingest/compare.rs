use serde::Serialize;

use crate::db::models::BenchmarkResultRecord;

pub fn preferred_base_ref(base_ref: Option<&str>) -> &str {
    base_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("main")
}

#[derive(Debug, Clone, Serialize)]
pub struct ComparisonRow {
    pub function_name: String,
    pub function_label: String,
    pub baseline_avg_ms: Option<f64>,
    pub candidate_avg_ms: f64,
    pub delta_pct: Option<f64>,
    pub label: String,
}

pub fn compare_result_sets(
    baseline_results: &[BenchmarkResultRecord],
    candidate_results: &[BenchmarkResultRecord],
    threshold_pct: f64,
) -> Vec<ComparisonRow> {
    let mut rows = candidate_results
        .iter()
        .map(|candidate| {
            let baseline = baseline_results
                .iter()
                .find(|result| result.function_name == candidate.function_name);
            let baseline_avg_ms = baseline.map(|result| result.avg_ms);
            let delta_pct = percent_delta(baseline_avg_ms, Some(candidate.avg_ms));

            ComparisonRow {
                function_name: candidate.function_name.clone(),
                function_label: candidate.function_label.clone(),
                baseline_avg_ms,
                candidate_avg_ms: candidate.avg_ms,
                delta_pct,
                label: delta_label(delta_pct, threshold_pct).to_string(),
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|lhs, rhs| lhs.function_name.cmp(&rhs.function_name));
    rows
}

pub fn percent_delta(baseline: Option<f64>, candidate: Option<f64>) -> Option<f64> {
    let baseline = baseline?;
    let candidate = candidate?;
    if baseline == 0.0 {
        return None;
    }

    Some(((candidate - baseline) / baseline) * 100.0)
}

pub fn delta_label(delta_pct: Option<f64>, threshold_pct: f64) -> &'static str {
    match delta_pct {
        Some(value) if value >= threshold_pct => "regressed",
        Some(value) if value <= -threshold_pct => "improved",
        Some(_) => "neutral",
        None => "neutral",
    }
}
