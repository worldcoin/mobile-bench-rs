use anyhow::{Context, Result};
use mobench::summarize::SummarizeReport;

pub fn parse_summary_json(bytes: &[u8]) -> Result<SummarizeReport> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).context("parsing summary.json value")?;

    mobench::summarize::parse_summary_value(&value).context("parsing mobench summary report")
}
