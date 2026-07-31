//! Canonical GitHub report adapters.
//!
//! This module owns payload shape and provider limits. HTTP authentication and
//! transport stay in the CLI crate.

use serde::Serialize;

pub const GITHUB_CHECK_ANNOTATION_LIMIT: usize = 50;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckRunAnnotation {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub annotation_level: String,
    pub message: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckRunRequest {
    pub name: String,
    pub head_sha: String,
    pub status: String,
    pub conclusion: String,
    pub output: CheckRunOutput,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckRunOutput {
    pub title: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<CheckRunAnnotation>,
}

impl CheckRunRequest {
    #[must_use]
    pub fn completed(
        name: impl Into<String>,
        head_sha: impl Into<String>,
        conclusion: impl Into<String>,
        title: impl Into<String>,
        summary: impl Into<String>,
        mut annotations: Vec<CheckRunAnnotation>,
    ) -> Self {
        annotations.truncate(GITHUB_CHECK_ANNOTATION_LIMIT);
        Self {
            name: name.into(),
            head_sha: head_sha.into(),
            status: "completed".to_owned(),
            conclusion: conclusion.into(),
            output: CheckRunOutput {
                title: title.into(),
                summary: summary.into(),
                annotations,
            },
        }
    }

    #[must_use]
    pub fn annotations_count(&self) -> usize {
        self.output.annotations.len()
    }
}

#[must_use]
pub fn render_sticky_comment(marker: &str, markdown: &str) -> String {
    format!("{marker}\n\n{markdown}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_check_payload_applies_the_provider_annotation_limit() {
        let annotations = (0..55)
            .map(|index| CheckRunAnnotation {
                path: "bench.rs".to_owned(),
                start_line: index + 1,
                end_line: index + 1,
                annotation_level: "failure".to_owned(),
                message: format!("regression {index}"),
                title: "Mobench regression".to_owned(),
            })
            .collect();
        let request = CheckRunRequest::completed(
            "mobench",
            "abc123",
            "failure",
            "Regressions",
            "summary",
            annotations,
        );

        assert_eq!(request.status, "completed");
        assert_eq!(request.annotations_count(), GITHUB_CHECK_ANNOTATION_LIMIT);
        assert_eq!(request.output.annotations[49].start_line, 50);
    }

    #[test]
    fn sticky_comment_preserves_the_released_marker_layout() {
        assert_eq!(
            render_sticky_comment("<!-- mobench -->", "### Results\n"),
            "<!-- mobench -->\n\n### Results\n"
        );
    }
}
