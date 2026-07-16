//! Versioned Mobench contracts and boundary validation.

use serde_json::Value;
use thiserror::Error;

mod report;

pub use report::{
    BoundRunReportV2, ExpectedProviderBinding, ExpectedReportIdentity, LegacyV1AdapterError,
    MAX_REPORT_COUNT, MAX_REPORT_FAILURE_MESSAGE_BYTES, MAX_REPORT_IDENTIFIER_BYTES,
    MAX_REPORT_SAMPLES, ProviderReportBinding, REPORT_SCHEMA_V2, ReducedProvenanceReport,
    ReportBindingError, ReportConstructionError, ReportCount, ReportCounts, ReportEnvelopeV2,
    ReportFailure, ReportIdentifier, ReportIdentifierError, ReportIdentity, ReportOutcome,
    ReportValidationError, adapt_legacy_v1_json,
};

/// Maximum accepted JSON payload for one Android benchmark report.
pub const MAX_ANDROID_BENCH_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Errors produced while decoding Android benchmark log frames.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AndroidBenchFrameError {
    #[error("Android benchmark frame contains invalid JSON: {0}")]
    InvalidJson(String),
    #[error("Android benchmark chunk at line {line} appeared before a start marker")]
    UnexpectedChunk { line: usize },
    #[error("Android benchmark end marker at line {line} appeared before a start marker")]
    UnexpectedEnd { line: usize },
    #[error("Android benchmark frame starting at line {start_line} is incomplete")]
    IncompleteFrame { start_line: usize },
    #[error(
        "Android benchmark frame at line {line} interleaves with frame starting at line {start_line}"
    )]
    InterleavedFrame { start_line: usize, line: usize },
    #[error("Android benchmark payload at line {line} exceeds the {limit}-byte size limit")]
    PayloadTooLarge { line: usize, limit: usize },
    #[error("Android benchmark marker at line {line} is malformed")]
    MalformedMarker { line: usize },
}

struct OpenAndroidFrame {
    payload: String,
    start_line: usize,
}

/// Decode Android benchmark reports from generated logcat framing.
///
/// This validates framing, ordering, JSON syntax, and payload size. It does not
/// authenticate the emitting process or bind a report to a requested run;
/// callers must apply that policy once versioned run identity is available.
pub fn decode_android_bench_frames(logs: &str) -> Result<Vec<Value>, AndroidBenchFrameError> {
    let mut values = Vec::new();
    let mut open_frame = None::<OpenAndroidFrame>;

    for (index, line) in logs.lines().enumerate() {
        let line_number = index + 1;
        let Some(message) = android_protocol_message(line) else {
            continue;
        };
        if message == "BENCH_JSON_START" {
            if let Some(frame) = &open_frame {
                return Err(AndroidBenchFrameError::InterleavedFrame {
                    start_line: frame.start_line,
                    line: line_number,
                });
            }
            open_frame = Some(OpenAndroidFrame {
                payload: String::new(),
                start_line: line_number,
            });
        } else if let Some(chunk) = message.strip_prefix("BENCH_JSON_CHUNK ") {
            if let Some(frame) = open_frame.as_mut() {
                if frame.payload.len().saturating_add(chunk.len()) > MAX_ANDROID_BENCH_PAYLOAD_BYTES
                {
                    return Err(AndroidBenchFrameError::PayloadTooLarge {
                        line: line_number,
                        limit: MAX_ANDROID_BENCH_PAYLOAD_BYTES,
                    });
                }
                frame.payload.push_str(chunk);
            } else {
                return Err(AndroidBenchFrameError::UnexpectedChunk { line: line_number });
            }
        } else if message == "BENCH_JSON_END" {
            let Some(frame) = open_frame.take() else {
                return Err(AndroidBenchFrameError::UnexpectedEnd { line: line_number });
            };
            values.push(
                serde_json::from_str(&frame.payload)
                    .map_err(|error| AndroidBenchFrameError::InvalidJson(error.to_string()))?,
            );
        } else if let Some(payload) = message.strip_prefix("BENCH_JSON ") {
            if let Some(frame) = &open_frame {
                return Err(AndroidBenchFrameError::InterleavedFrame {
                    start_line: frame.start_line,
                    line: line_number,
                });
            }
            if payload.len() > MAX_ANDROID_BENCH_PAYLOAD_BYTES {
                return Err(AndroidBenchFrameError::PayloadTooLarge {
                    line: line_number,
                    limit: MAX_ANDROID_BENCH_PAYLOAD_BYTES,
                });
            }
            values.push(
                serde_json::from_str(payload)
                    .map_err(|error| AndroidBenchFrameError::InvalidJson(error.to_string()))?,
            );
        } else {
            return Err(AndroidBenchFrameError::MalformedMarker { line: line_number });
        }
    }

    if let Some(frame) = open_frame {
        return Err(AndroidBenchFrameError::IncompleteFrame {
            start_line: frame.start_line,
        });
    }

    Ok(values)
}

fn android_protocol_message(line: &str) -> Option<&str> {
    const MARKERS: [&str; 4] = [
        "BENCH_JSON_START",
        "BENCH_JSON_CHUNK",
        "BENCH_JSON_END",
        "BENCH_JSON ",
    ];
    let trimmed = line.trim();
    let (index, _) = MARKERS
        .iter()
        .filter_map(|marker| trimmed.find(marker).map(|index| (index, marker)))
        .min_by_key(|(index, _)| *index)?;
    if index == 0 {
        return Some(trimmed);
    }

    let prefix = trimmed[..index].trim_end();
    let marker_is_log_message = prefix.contains("BenchRunner")
        && (prefix.ends_with(':')
            || matches!(
                prefix.split_whitespace().last(),
                Some("V" | "D" | "I" | "W" | "E")
            ));
    marker_is_log_message.then_some(&trimmed[index..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_generated_android_chunk_frame() {
        let logs = r#"
2026-07-15 I/BenchRunner: BENCH_JSON_START
2026-07-15 I/BenchRunner: BENCH_JSON_CHUNK {"function":"sample_fns::checksum",
2026-07-15 I/BenchRunner: BENCH_JSON_CHUNK "samples_ns":[1000,2000]}
2026-07-15 I/BenchRunner: BENCH_JSON_END
"#;

        let values = decode_android_bench_frames(logs).expect("decode generated frame");

        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["function"], "sample_fns::checksum");
        assert_eq!(values[0]["samples_ns"], serde_json::json!([1000, 2000]));
    }

    #[test]
    fn decodes_released_android_legacy_frame() {
        let logs =
            r#"2026-01-20 I/BenchRunner: BENCH_JSON {"function":"legacy::bench","samples_ns":[7]}"#;

        let values = decode_android_bench_frames(logs).expect("decode legacy frame");

        assert_eq!(
            values,
            vec![serde_json::json!({
                "function": "legacy::bench",
                "samples_ns": [7]
            })]
        );
    }

    #[test]
    fn rejects_android_chunk_before_start() {
        let logs = r#"I/BenchRunner: BENCH_JSON_CHUNK {"function":"forged"}"#;

        assert_eq!(
            decode_android_bench_frames(logs),
            Err(AndroidBenchFrameError::UnexpectedChunk { line: 1 })
        );
    }

    #[test]
    fn rejects_truncated_android_chunk_frame() {
        let logs = r#"
I/BenchRunner: BENCH_JSON_START
I/BenchRunner: BENCH_JSON_CHUNK {"function":"truncated"
"#;

        assert_eq!(
            decode_android_bench_frames(logs),
            Err(AndroidBenchFrameError::IncompleteFrame { start_line: 2 })
        );
    }

    #[test]
    fn rejects_interleaved_android_chunk_frames() {
        let logs = r#"
I/BenchRunner: BENCH_JSON_START
I/BenchRunner: BENCH_JSON_CHUNK {"function":"first"}
I/BenchRunner: BENCH_JSON_START
"#;

        assert_eq!(
            decode_android_bench_frames(logs),
            Err(AndroidBenchFrameError::InterleavedFrame {
                start_line: 2,
                line: 4,
            })
        );
    }

    #[test]
    fn rejects_android_payload_over_size_bound() {
        let oversized = "x".repeat(MAX_ANDROID_BENCH_PAYLOAD_BYTES + 1);
        let logs = format!("I/BenchRunner: BENCH_JSON {{\"payload\":\"{oversized}\"}}");

        assert_eq!(
            decode_android_bench_frames(&logs),
            Err(AndroidBenchFrameError::PayloadTooLarge {
                line: 1,
                limit: MAX_ANDROID_BENCH_PAYLOAD_BYTES,
            })
        );
    }

    #[test]
    fn rejects_android_end_before_start() {
        assert_eq!(
            decode_android_bench_frames("I/BenchRunner: BENCH_JSON_END"),
            Err(AndroidBenchFrameError::UnexpectedEnd { line: 1 })
        );
    }

    #[test]
    fn ignores_protocol_words_inside_other_log_messages() {
        let logs = r#"E/BenchRunner: BENCH_FAILURE_JSON {"message":"worker exited before BENCH_JSON was emitted"}"#;

        assert_eq!(decode_android_bench_frames(logs), Ok(Vec::new()));
    }

    #[test]
    fn rejects_android_marker_with_trailing_data() {
        assert_eq!(
            decode_android_bench_frames("I/BenchRunner: BENCH_JSON_START forged"),
            Err(AndroidBenchFrameError::MalformedMarker { line: 1 })
        );
    }
}
