//! Provider-independent session reconciliation.
//!
//! Provider adapters submit the terminal sessions they expected and the
//! artifacts they collected. This module owns the invariant that a complete
//! matrix has exactly one validated report for every distinct expected device
//! and session. It deliberately knows nothing about HTTP, BrowserStack, local
//! processes, report JSON, or rendering.

use std::collections::{HashMap, HashSet};
use std::fmt;

use thiserror::Error;

/// Terminal result of reconciling a requested provider matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatrixOutcome {
    /// Every expected session passed and produced exactly one report.
    Complete,
    /// At least one session completed, but at least one required session did not.
    Partial,
    /// No expected session produced one passing report.
    Failed,
}

/// One terminal session advertised by a provider adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedSession {
    /// Provider session identifier.
    pub session_id: String,
    /// Stable device label within this matrix.
    pub device_id: String,
    /// Provider terminal status, preserved for diagnostics.
    pub status: String,
}

/// Artifacts collected for one provider session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectedSession {
    /// Provider session identifier used for correlation.
    pub session_id: String,
    /// Number of validated benchmark reports attributed to the session.
    pub report_count: usize,
    /// Optional already-sanitized failure summary from the adapter.
    pub failure: Option<String>,
}

/// Why one expected session did not satisfy the complete-matrix contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionDisposition {
    /// The provider passed the session and exactly one report was collected.
    Complete,
    /// No collection record was produced for the expected session.
    Missing,
    /// The provider reported a terminal status other than `passed`.
    NonPassed {
        /// Raw provider status retained for actionable diagnostics.
        status: String,
        /// Optional provider-specific failure summary.
        failure: Option<String>,
    },
    /// The provider passed the session, but no validated report was collected.
    Resultless {
        /// Optional provider-specific failure summary.
        failure: Option<String>,
    },
}

/// Reconciliation receipt for one expected session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionAssessment {
    /// Provider session identifier.
    pub session_id: String,
    /// Stable device label.
    pub device_id: String,
    /// Terminal disposition.
    pub disposition: SessionDisposition,
}

/// Deterministic reconciliation of an entire provider matrix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatrixAssessment {
    outcome: MatrixOutcome,
    sessions: Vec<SessionAssessment>,
}

impl MatrixAssessment {
    /// Matrix-level complete/partial/failed outcome.
    pub const fn outcome(&self) -> MatrixOutcome {
        self.outcome
    }

    /// Session receipts in the same order as the expected matrix.
    pub fn sessions(&self) -> &[SessionAssessment] {
        &self.sessions
    }

    /// True only when every expected session satisfied the contract.
    pub const fn is_complete(&self) -> bool {
        matches!(self.outcome, MatrixOutcome::Complete)
    }
}

impl fmt::Display for MatrixAssessment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let missing = self
            .sessions
            .iter()
            .filter(|session| matches!(session.disposition, SessionDisposition::Missing))
            .map(session_label)
            .collect::<Vec<_>>();
        let non_passed = self
            .sessions
            .iter()
            .filter_map(|session| match &session.disposition {
                SessionDisposition::NonPassed { status, failure } => Some(format!(
                    "{} ({}, status={}{}{})",
                    session.device_id,
                    session.session_id,
                    status,
                    failure.as_ref().map(|_| "; ").unwrap_or_default(),
                    failure.as_deref().unwrap_or_default()
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        let resultless = self
            .sessions
            .iter()
            .filter_map(|session| match &session.disposition {
                SessionDisposition::Resultless { failure } => Some(format!(
                    "{} ({}{}{})",
                    session.device_id,
                    session.session_id,
                    failure.as_ref().map(|_| "; ").unwrap_or_default(),
                    failure.as_deref().unwrap_or_default()
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        let mut groups = Vec::new();
        if !missing.is_empty() {
            groups.push(format!(
                "missing collected sessions: {}",
                missing.join(", ")
            ));
        }
        if !non_passed.is_empty() {
            groups.push(format!("non-passed sessions: {}", non_passed.join(", ")));
        }
        if !resultless.is_empty() {
            groups.push(format!("result-less sessions: {}", resultless.join(", ")));
        }
        write!(formatter, "{}", groups.join("; "))
    }
}

fn session_label(session: &SessionAssessment) -> String {
    format!("{} ({})", session.device_id, session.session_id)
}

/// Structural ambiguity that prevents trustworthy reconciliation.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReconcileError {
    /// A terminal provider build did not advertise any sessions.
    #[error("terminal provider build reported no device sessions")]
    NoExpectedSessions,
    /// Two expected records used the same provider session id.
    #[error("duplicate expected session: {session_id}")]
    DuplicateExpectedSession { session_id: String },
    /// Two expected records used the same public device label.
    #[error("duplicate expected device: {device_id}")]
    DuplicateExpectedDevice { device_id: String },
    /// Two collection records claimed the same provider session.
    #[error("duplicate collected session: {session_id}")]
    DuplicateCollectedSession { session_id: String },
    /// A collection record did not correspond to the expected matrix.
    #[error("unexpected collected session: {session_id}")]
    UnexpectedCollectedSession { session_id: String },
    /// One session produced more than one report and cannot be attributed safely.
    #[error("duplicate benchmark results: {device_id} ({session_id}, got {report_count})")]
    DuplicateReports {
        device_id: String,
        session_id: String,
        report_count: usize,
    },
}

/// Reconcile terminal provider sessions with collected artifacts.
///
/// Structural ambiguity is returned as an error. Missing, failed, and
/// result-less sessions are retained in a typed assessment so callers can
/// distinguish partial from total failure without relabeling either as
/// complete.
pub fn reconcile_sessions(
    expected: &[ExpectedSession],
    collected: &[CollectedSession],
) -> Result<MatrixAssessment, ReconcileError> {
    if expected.is_empty() {
        return Err(ReconcileError::NoExpectedSessions);
    }

    let mut expected_session_ids = HashSet::with_capacity(expected.len());
    let mut expected_device_ids = HashSet::with_capacity(expected.len());
    for session in expected {
        if !expected_session_ids.insert(session.session_id.as_str()) {
            return Err(ReconcileError::DuplicateExpectedSession {
                session_id: session.session_id.clone(),
            });
        }
        if !expected_device_ids.insert(session.device_id.as_str()) {
            return Err(ReconcileError::DuplicateExpectedDevice {
                device_id: session.device_id.clone(),
            });
        }
    }

    let mut collected_by_session = HashMap::with_capacity(collected.len());
    for session in collected {
        if !expected_session_ids.contains(session.session_id.as_str()) {
            return Err(ReconcileError::UnexpectedCollectedSession {
                session_id: session.session_id.clone(),
            });
        }
        if collected_by_session
            .insert(session.session_id.as_str(), session)
            .is_some()
        {
            return Err(ReconcileError::DuplicateCollectedSession {
                session_id: session.session_id.clone(),
            });
        }
    }

    let mut completed = 0usize;
    let mut sessions = Vec::with_capacity(expected.len());
    for session in expected {
        let disposition = match collected_by_session.get(session.session_id.as_str()) {
            None => SessionDisposition::Missing,
            Some(collected) if collected.report_count > 1 => {
                return Err(ReconcileError::DuplicateReports {
                    device_id: session.device_id.clone(),
                    session_id: session.session_id.clone(),
                    report_count: collected.report_count,
                });
            }
            Some(collected) if !session.status.eq_ignore_ascii_case("passed") => {
                SessionDisposition::NonPassed {
                    status: session.status.clone(),
                    failure: collected.failure.clone(),
                }
            }
            Some(collected) if collected.report_count == 0 => SessionDisposition::Resultless {
                failure: collected.failure.clone(),
            },
            Some(_) => {
                completed += 1;
                SessionDisposition::Complete
            }
        };
        sessions.push(SessionAssessment {
            session_id: session.session_id.clone(),
            device_id: session.device_id.clone(),
            disposition,
        });
    }

    let outcome = if completed == expected.len() {
        MatrixOutcome::Complete
    } else if completed == 0 {
        MatrixOutcome::Failed
    } else {
        MatrixOutcome::Partial
    };
    Ok(MatrixAssessment { outcome, sessions })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected(count: usize) -> Vec<ExpectedSession> {
        (0..count)
            .map(|index| ExpectedSession {
                session_id: format!("session-{index}"),
                device_id: format!("device-{index}"),
                status: "passed".to_owned(),
            })
            .collect()
    }

    fn collected(count: usize) -> Vec<CollectedSession> {
        (0..count)
            .rev()
            .map(|index| CollectedSession {
                session_id: format!("session-{index}"),
                report_count: 1,
                failure: None,
            })
            .collect()
    }

    #[test]
    fn complete_matrices_are_deterministic_at_one_ten_and_fifty_devices() {
        for count in [1, 10, 50] {
            let expected = expected(count);
            let assessment = reconcile_sessions(&expected, &collected(count)).unwrap();
            assert_eq!(assessment.outcome(), MatrixOutcome::Complete);
            assert!(assessment.is_complete());
            assert_eq!(assessment.sessions().len(), count);
            for (index, session) in assessment.sessions().iter().enumerate() {
                assert_eq!(session.session_id, format!("session-{index}"));
                assert_eq!(session.disposition, SessionDisposition::Complete);
            }
        }
    }

    #[test]
    fn one_of_ten_is_partial_and_never_complete() {
        let assessment = reconcile_sessions(&expected(10), &collected(1)).unwrap();
        assert_eq!(assessment.outcome(), MatrixOutcome::Partial);
        assert!(!assessment.is_complete());
        assert!(
            assessment
                .to_string()
                .contains("missing collected sessions")
        );
        assert!(assessment.to_string().contains("session-9"));
    }

    #[test]
    fn zero_successful_sessions_is_failed() {
        let mut expected = expected(1);
        expected[0].status = "failed".to_owned();
        let collected = [CollectedSession {
            session_id: "session-0".to_owned(),
            report_count: 0,
            failure: Some("benchmark timed out".to_owned()),
        }];
        let assessment = reconcile_sessions(&expected, &collected).unwrap();
        assert_eq!(assessment.outcome(), MatrixOutcome::Failed);
        assert!(assessment.to_string().contains("non-passed sessions"));
        assert!(assessment.to_string().contains("benchmark timed out"));
    }

    #[test]
    fn passed_session_without_a_report_is_failed() {
        let collected = [CollectedSession {
            session_id: "session-0".to_owned(),
            report_count: 0,
            failure: Some("no report marker".to_owned()),
        }];
        let assessment = reconcile_sessions(&expected(1), &collected).unwrap();
        assert_eq!(assessment.outcome(), MatrixOutcome::Failed);
        assert!(assessment.to_string().contains("result-less sessions"));
    }

    #[test]
    fn ambiguity_and_unexpected_sessions_fail_closed() {
        let mut duplicate_expected = expected(2);
        duplicate_expected[1].device_id = duplicate_expected[0].device_id.clone();
        assert!(matches!(
            reconcile_sessions(&duplicate_expected, &collected(2)),
            Err(ReconcileError::DuplicateExpectedDevice { .. })
        ));

        let duplicate_collected = [collected(1)[0].clone(), collected(1)[0].clone()];
        assert!(matches!(
            reconcile_sessions(&expected(1), &duplicate_collected),
            Err(ReconcileError::DuplicateCollectedSession { .. })
        ));

        let unexpected = [CollectedSession {
            session_id: "not-requested".to_owned(),
            report_count: 1,
            failure: None,
        }];
        assert!(matches!(
            reconcile_sessions(&expected(1), &unexpected),
            Err(ReconcileError::UnexpectedCollectedSession { .. })
        ));
    }

    #[test]
    fn multiple_reports_for_one_session_are_ambiguous() {
        let collected = [CollectedSession {
            session_id: "session-0".to_owned(),
            report_count: 2,
            failure: None,
        }];
        assert_eq!(
            reconcile_sessions(&expected(1), &collected),
            Err(ReconcileError::DuplicateReports {
                device_id: "device-0".to_owned(),
                session_id: "session-0".to_owned(),
                report_count: 2,
            })
        );
    }
}
