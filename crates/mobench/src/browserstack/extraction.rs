//! Provider payload normalization and artifact discovery.

use mobench_runtime::Distribution;
use serde_json::Value;

pub(super) fn normalize_benchmark_values(value: Value) -> Vec<Value> {
    match value {
        Value::Array(entries) => entries
            .into_iter()
            .filter_map(normalize_benchmark_value)
            .collect(),
        value => normalize_benchmark_value(value).into_iter().collect(),
    }
}

pub(super) fn normalize_benchmark_value(mut value: Value) -> Option<Value> {
    let samples = extract_sample_durations(&value);
    let stats = Distribution::from_slice(&samples).cli_v1_summary();
    let object = value.as_object_mut()?;

    if !object.contains_key("function")
        && let Some(function) = object
            .get("spec")
            .and_then(|spec| spec.get("name"))
            .and_then(|name| name.as_str())
    {
        object.insert("function".to_string(), Value::String(function.to_string()));
    }

    if !object.contains_key("samples")
        && let Some(samples_ns) = object
            .get("samples_ns")
            .and_then(|samples| samples.as_array())
    {
        object.insert("samples".to_string(), Value::Array(samples_ns.clone()));
    }

    let has_function = object
        .get("function")
        .and_then(|value| value.as_str())
        .is_some();
    let has_samples = object
        .get("samples")
        .and_then(|value| value.as_array())
        .is_some();
    let has_stats = ["mean_ns", "median_ns", "p95_ns", "min_ns", "max_ns"]
        .iter()
        .any(|key| object.get(*key).is_some());

    if !has_function || (!has_samples && !has_stats) {
        return None;
    }

    if let Some(stats) = stats {
        if !object.contains_key("mean_ns") {
            object.insert("mean_ns".to_string(), Value::from(stats.mean_ns));
        }
        if !object.contains_key("median_ns") {
            object.insert("median_ns".to_string(), Value::from(stats.median_ns));
        }
        if !object.contains_key("p95_ns") {
            object.insert("p95_ns".to_string(), Value::from(stats.p95_ns));
        }
        if !object.contains_key("min_ns") {
            object.insert("min_ns".to_string(), Value::from(stats.min_ns));
        }
        if !object.contains_key("max_ns") {
            object.insert("max_ns".to_string(), Value::from(stats.max_ns));
        }
    }

    Some(value)
}

pub(super) fn extend_unique_results(results: &mut Vec<Value>, new_results: Vec<Value>) {
    for result in new_results {
        if !results.iter().any(|existing| existing == &result) {
            results.push(result);
        }
    }
}

pub(super) fn collect_text_artifact_urls(value: &Value) -> Vec<(String, String)> {
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
                    format!("{prefix}.{key}")
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
                let next = format!("{prefix}[{index}]");
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

fn extract_sample_durations(value: &Value) -> Vec<u64> {
    let mut durations = Vec::new();

    if let Some(samples) = value.get("samples").and_then(|samples| samples.as_array()) {
        for sample in samples {
            if let Some(duration_ns) = sample
                .get("duration_ns")
                .and_then(|duration| duration.as_u64())
            {
                durations.push(duration_ns);
            } else if let Some(duration_ns) = sample.as_u64() {
                durations.push(duration_ns);
            }
        }
    }

    if durations.is_empty()
        && let Some(samples_ns) = value
            .get("samples_ns")
            .and_then(|samples| samples.as_array())
    {
        durations.extend(samples_ns.iter().filter_map(|sample| sample.as_u64()));
    }

    durations
}
