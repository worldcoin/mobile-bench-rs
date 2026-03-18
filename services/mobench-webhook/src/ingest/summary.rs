use anyhow::{Context, Result};
use mobench::summarize::SummarizeReport;

pub fn parse_summary_json(bytes: &[u8]) -> Result<SummarizeReport> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).context("parsing summary.json value")?;

    match serde_json::from_value::<SummarizeReport>(value.clone()) {
        Ok(report) => Ok(report),
        Err(_) => mobench::summarize::parse_summary_value(&value)
            .context("parsing mobench summary report"),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_summary_json;

    #[test]
    fn parses_ci_summarize_report_json() {
        let bytes = br#"{
          "platforms": [
            {
              "platform": "ios",
              "device": {
                "name": "iPhone 14",
                "os": "iOS",
                "os_version": "16.0"
              },
              "benchmarks": [
                {
                  "name": "sample_fns::fibonacci",
                  "label": "fibonacci",
                  "timing": {
                    "avg_ms": 12.4,
                    "median_ms": 12.1,
                    "best_ms": 11.8,
                    "worst_ms": 13.2,
                    "p95_ms": 13.0,
                    "std_dev_ms": 0.4
                  }
                }
              ],
              "iterations": 30,
              "warmup": 5
            }
          ]
        }"#;

        let report = parse_summary_json(bytes).unwrap();

        assert_eq!(report.platforms.len(), 1);
        assert_eq!(report.platforms[0].platform, "ios");
        assert_eq!(report.platforms[0].benchmarks.len(), 1);
        assert_eq!(report.platforms[0].benchmarks[0].name, "sample_fns::fibonacci");
    }
}
