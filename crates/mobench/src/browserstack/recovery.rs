use anyhow::{Result, anyhow};
use serde_json::Value;

use super::{PerformanceMetrics, merge_performance_metrics};

pub(crate) struct RecoveredPayload {
    pub(crate) benchmark_results: Vec<Value>,
    pub(crate) performance_metrics: PerformanceMetrics,
}

pub(crate) fn recover_from_session_artifacts<F, B, P>(
    session_json: &Value,
    mut fetch_text: F,
    mut extract_benchmark_results: B,
    mut extract_performance_metrics: P,
) -> Result<RecoveredPayload>
where
    F: FnMut(&str) -> Result<String>,
    B: FnMut(&str) -> Result<Vec<Value>>,
    P: FnMut(&str) -> Result<PerformanceMetrics>,
{
    let artifact_urls = collect_text_artifact_urls(session_json);
    if artifact_urls.is_empty() {
        return Err(anyhow!("No text artifact URLs found in session response"));
    }

    let mut benchmark_results = Vec::new();
    let mut performance_metrics = PerformanceMetrics::default();

    for (_, url) in artifact_urls {
        let contents = match fetch_text(&url) {
            Ok(contents) => contents,
            Err(_) => continue,
        };

        if benchmark_results.is_empty()
            && let Ok(results) = extract_benchmark_results(&contents)
        {
            benchmark_results = results;
        }

        if let Ok(metrics) = extract_performance_metrics(&contents)
            && metrics.sample_count > 0
        {
            performance_metrics =
                merge_performance_metrics(Some(performance_metrics), Some(metrics))
                    .unwrap_or_default();
        }
    }

    if benchmark_results.is_empty() {
        Err(anyhow!("No benchmark results found in session artifacts"))
    } else {
        Ok(RecoveredPayload {
            benchmark_results,
            performance_metrics,
        })
    }
}

fn collect_text_artifact_urls(value: &Value) -> Vec<(String, String)> {
    let mut urls = Vec::new();
    collect_text_artifact_urls_recursive(value, "", &mut urls);
    urls.sort_by_key(|(key, url)| artifact_url_priority(key, url));
    urls
}

fn collect_text_artifact_urls_recursive(
    value: &Value,
    prefix: &str,
    out: &mut Vec<(String, String)>,
) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let next = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                if let Value::String(url) = value
                    && (url.starts_with("http") || url.starts_with("bs://"))
                    && artifact_url_priority(&next, url) < 4
                {
                    out.push((next.clone(), url.clone()));
                }
                collect_text_artifact_urls_recursive(value, &next, out);
            }
        }
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                let next = format!("{}[{}]", prefix, index);
                collect_text_artifact_urls_recursive(value, &next, out);
            }
        }
        _ => {}
    }
}

fn artifact_url_priority(key: &str, url: &str) -> u8 {
    let lower = format!("{} {}", key.to_ascii_lowercase(), url.to_ascii_lowercase());
    if lower.contains("bench-report") || lower.contains("bench_report") {
        0
    } else if lower.contains("device_log")
        || lower.contains("devicelog")
        || lower.contains("instrumentation_log")
        || lower.contains("app_log")
    {
        1
    } else if lower.ends_with(".json") || lower.ends_with(".log") || lower.ends_with(".txt") {
        2
    } else {
        4
    }
}
