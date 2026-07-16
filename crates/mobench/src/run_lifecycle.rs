//! Command-level benchmark Run lifecycle and atomic report publication.
//!
//! The provider Module owns starting and collecting one Provider Run. This
//! Module owns the wider command lifecycle after resolution: it verifies that
//! bound reports belong to the resolved Run Specification, derives the
//! terminal outcome, prepares the canonical report, and commits all report
//! artifacts as one manifest-backed publication.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use mobench_artifacts::{ArtifactId, ArtifactPathError, LatestArtifact, RunWorkspace};
use mobench_domain::{BoundRunReportV2, ReportCounts, ReportOutcome};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::MobileTarget;

/// A fully resolved command-level Run Specification.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedRunPlan {
    run_id: String,
    target: MobileTarget,
    function_id: String,
    requested: ReportCounts,
    expected_sessions: BTreeSet<String>,
}

impl ResolvedRunPlan {
    /// Validate the identity and expected Provider Sessions for one run.
    pub(crate) fn new(
        run_id: impl Into<String>,
        target: MobileTarget,
        function_id: impl Into<String>,
        requested: ReportCounts,
        expected_sessions: impl IntoIterator<Item = String>,
    ) -> Result<Self, LifecycleError> {
        let run_id = run_id.into();
        let function_id = function_id.into();
        mobench_domain::ReportIdentifier::parse(run_id.clone())
            .map_err(|error| LifecycleError::InvalidIdentity(error.to_string()))?;
        mobench_domain::ReportIdentifier::parse(function_id.clone())
            .map_err(|error| LifecycleError::InvalidIdentity(error.to_string()))?;

        let mut unique_sessions = BTreeSet::new();
        for session in expected_sessions {
            mobench_domain::ReportIdentifier::parse(session.clone())
                .map_err(|error| LifecycleError::InvalidIdentity(error.to_string()))?;
            if !unique_sessions.insert(session.clone()) {
                return Err(LifecycleError::DuplicateExpectedSession { session });
            }
        }

        Ok(Self {
            run_id,
            target,
            function_id,
            requested,
            expected_sessions: unique_sessions,
        })
    }

    /// Enter the lifecycle after command resolution.
    pub(crate) fn begin(self) -> ResolvedRun {
        ResolvedRun { plan: self }
    }
}

/// A resolved lifecycle that has not accepted terminal provider evidence yet.
#[derive(Debug)]
pub(crate) struct ResolvedRun {
    plan: ResolvedRunPlan,
}

impl ResolvedRun {
    /// Validate provider-bound reports and derive one terminal outcome.
    pub(crate) fn collect(
        self,
        reports: Vec<BoundRunReportV2>,
    ) -> Result<CollectedRun, LifecycleError> {
        let mut observed_sessions = BTreeSet::new();
        let mut successful_sessions = 0_usize;

        for report in &reports {
            let envelope = report.envelope();
            let binding = report.binding();
            let identity = envelope.identity();
            if identity.run_id().as_str() != self.plan.run_id
                || identity.function_id().as_str() != self.plan.function_id
                || envelope.requested() != self.plan.requested
            {
                return Err(LifecycleError::ReportPlanMismatch {
                    session: binding.transport_session_id().as_str().to_owned(),
                });
            }

            let requested_device = binding.requested_device_id().as_str().to_owned();
            if !self.plan.expected_sessions.contains(&requested_device) {
                return Err(LifecycleError::UnexpectedReport {
                    session: requested_device,
                });
            }
            if !observed_sessions.insert(requested_device.clone()) {
                return Err(LifecycleError::DuplicateReport {
                    session: requested_device,
                });
            }
            if matches!(envelope.outcome(), ReportOutcome::Success) {
                successful_sessions += 1;
            }
        }

        let expected_sessions = self.plan.expected_sessions.len();
        let outcome = if successful_sessions == expected_sessions {
            RunOutcome::Complete {
                expected_sessions,
                successful_sessions,
            }
        } else if successful_sessions == 0 {
            RunOutcome::Failed {
                expected_sessions,
                successful_sessions,
            }
        } else {
            RunOutcome::Partial {
                expected_sessions,
                successful_sessions,
            }
        };

        Ok(CollectedRun {
            plan: self.plan,
            reports,
            outcome,
        })
    }
}

/// Terminal command-level result derived from validated provider evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum RunOutcome {
    /// Every expected Provider Session produced a successful bound report.
    Complete {
        expected_sessions: usize,
        successful_sessions: usize,
    },
    /// Some, but not all, expected Provider Sessions produced successful reports.
    Partial {
        expected_sessions: usize,
        successful_sessions: usize,
    },
    /// No expected Provider Session produced a successful report.
    Failed {
        expected_sessions: usize,
        successful_sessions: usize,
    },
}

impl RunOutcome {
    pub(crate) const fn is_complete(self) -> bool {
        matches!(self, Self::Complete { .. })
    }
}

/// A collected run that is eligible for canonical report preparation.
#[derive(Debug)]
pub(crate) struct CollectedRun {
    plan: ResolvedRunPlan,
    reports: Vec<BoundRunReportV2>,
    outcome: RunOutcome,
}

impl CollectedRun {
    /// Prepare compatibility outputs and the canonical v2 report as one bundle.
    pub(crate) fn prepare(self, artifacts: ReportArtifacts) -> Result<PreparedRun, LifecycleError> {
        let canonical = CanonicalSummaryV2 {
            schema_version: "mobench.summary/v2",
            run_id: &self.plan.run_id,
            target: self.plan.target,
            function_id: &self.plan.function_id,
            requested: self.plan.requested,
            lifecycle: self.outcome,
            reports: &self.reports,
        };
        let canonical_json = serde_json::to_vec_pretty(&canonical)?;
        let compatibility_json = serde_json::to_vec_pretty(&artifacts.compatibility_json)?;

        let mut files = vec![
            PreparedArtifact::new(artifacts.json_name, compatibility_json),
            PreparedArtifact::new(artifacts.markdown_name, artifacts.markdown.into_bytes()),
            PreparedArtifact::new(artifacts.canonical_name, canonical_json),
        ];
        if let Some((name, csv)) = artifacts.csv {
            files.push(PreparedArtifact::new(name, csv.into_bytes()));
        }

        Ok(PreparedRun {
            root: artifacts.root,
            logical_id: ArtifactId::new(self.plan.run_id)?,
            outcome: self.outcome,
            files,
        })
    }
}

#[derive(Serialize)]
struct CanonicalSummaryV2<'a> {
    schema_version: &'static str,
    run_id: &'a str,
    target: MobileTarget,
    function_id: &'a str,
    requested: ReportCounts,
    lifecycle: RunOutcome,
    reports: &'a [BoundRunReportV2],
}

/// Compatibility report contents and their stable output names.
#[derive(Debug)]
pub(crate) struct ReportArtifacts {
    root: PathBuf,
    json_name: ArtifactId,
    markdown_name: ArtifactId,
    canonical_name: ArtifactId,
    compatibility_json: Value,
    markdown: String,
    csv: Option<(ArtifactId, String)>,
}

impl ReportArtifacts {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        json_path: &Path,
        compatibility_json: Value,
        markdown_path: &Path,
        markdown: String,
        csv: Option<(&Path, String)>,
        canonical_path: &Path,
    ) -> Result<Self, LifecycleError> {
        let root = output_root(json_path);
        for path in [markdown_path, canonical_path]
            .into_iter()
            .chain(csv.as_ref().map(|(path, _)| *path))
        {
            if output_root(path) != root {
                return Err(LifecycleError::MixedOutputRoots {
                    expected: root.clone(),
                    observed: output_root(path),
                });
            }
        }

        let csv = match csv {
            Some((path, contents)) => Some((artifact_id(path)?, contents)),
            None => None,
        };
        let names = [
            artifact_id(json_path)?,
            artifact_id(markdown_path)?,
            artifact_id(canonical_path)?,
        ];
        let mut unique_names = BTreeSet::new();
        for name in names.iter().chain(csv.iter().map(|(name, _)| name)) {
            if !unique_names.insert(name.as_str().to_owned()) {
                return Err(LifecycleError::DuplicateArtifactName {
                    name: name.as_str().to_owned(),
                });
            }
        }

        Ok(Self {
            root,
            json_name: names[0].clone(),
            markdown_name: names[1].clone(),
            canonical_name: names[2].clone(),
            compatibility_json,
            markdown,
            csv,
        })
    }
}

fn output_root(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn artifact_id(path: &Path) -> Result<ArtifactId, LifecycleError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| LifecycleError::InvalidOutputPath {
            path: path.to_path_buf(),
        })?;
    ArtifactId::new(name.to_owned()).map_err(LifecycleError::from)
}

#[derive(Debug)]
struct PreparedArtifact {
    id: ArtifactId,
    bytes: Vec<u8>,
}

impl PreparedArtifact {
    fn new(id: ArtifactId, bytes: Vec<u8>) -> Self {
        Self { id, bytes }
    }
}

/// A fully encoded run whose reports can now be committed atomically.
#[derive(Debug)]
pub(crate) struct PreparedRun {
    root: PathBuf,
    logical_id: ArtifactId,
    outcome: RunOutcome,
    files: Vec<PreparedArtifact>,
}

impl PreparedRun {
    /// Commit an immutable publication, then atomically refresh stable aliases.
    pub(crate) fn commit(self) -> Result<CommittedRun, LifecycleError> {
        let workspace = RunWorkspace::allocate(&self.root, &self.logical_id)?;
        for artifact in &self.files {
            let path = workspace.staging_path().join(artifact.id.as_str());
            fs::write(&path, &artifact.bytes)
                .map_err(|source| LifecycleError::WriteArtifact { path, source })?;
        }

        let required = self
            .files
            .iter()
            .map(|artifact| artifact.id.clone())
            .collect::<Vec<_>>();
        let published = workspace.publish(&required)?;
        let latest = required
            .iter()
            .cloned()
            .map(LatestArtifact::same)
            .collect::<Vec<_>>();
        published.refresh_latest(&latest)?;

        Ok(CommittedRun {
            outcome: self.outcome,
            published_path: published.path().to_path_buf(),
            stable_paths: required
                .iter()
                .map(|id| self.root.join(id.as_str()))
                .collect(),
        })
    }
}

/// Receipt proving that the report bundle is immutable and its aliases committed.
#[derive(Debug)]
pub(crate) struct CommittedRun {
    outcome: RunOutcome,
    published_path: PathBuf,
    stable_paths: Vec<PathBuf>,
}

impl CommittedRun {
    pub(crate) const fn outcome(&self) -> RunOutcome {
        self.outcome
    }

    pub(crate) fn published_path(&self) -> &Path {
        &self.published_path
    }

    pub(crate) fn stable_paths(&self) -> &[PathBuf] {
        &self.stable_paths
    }
}

/// Invalid state transition or publication failure at the Run lifecycle Seam.
#[derive(Debug, Error)]
pub(crate) enum LifecycleError {
    #[error("invalid run lifecycle identity: {0}")]
    InvalidIdentity(String),
    #[error("duplicate expected Provider Session `{session}`")]
    DuplicateExpectedSession { session: String },
    #[error("bound report for `{session}` does not belong to the resolved run")]
    ReportPlanMismatch { session: String },
    #[error("bound report targets unexpected Provider Session `{session}`")]
    UnexpectedReport { session: String },
    #[error("multiple bound reports target Provider Session `{session}`")]
    DuplicateReport { session: String },
    #[error("report outputs span multiple roots: expected {expected}, observed {observed}")]
    MixedOutputRoots {
        expected: PathBuf,
        observed: PathBuf,
    },
    #[error("report output path has no valid UTF-8 file name: {path}")]
    InvalidOutputPath { path: PathBuf },
    #[error("duplicate report artifact name `{name}`")]
    DuplicateArtifactName { name: String },
    #[error("writing staged report artifact {path}: {source}")]
    WriteArtifact {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Artifact(#[from] ArtifactPathError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use mobench_domain::{
        ExpectedProviderBinding, ExpectedReportIdentity, ProviderReportBinding, ReportFailure,
        ReportIdentifier, ReportIdentity,
    };

    use super::*;

    fn id(value: &str) -> ReportIdentifier {
        ReportIdentifier::parse(value.to_owned()).expect("valid test identifier")
    }

    fn bound_report(device: &str, success: bool) -> BoundRunReportV2 {
        let counts = ReportCounts::new(3, 1).expect("counts");
        let identity = ReportIdentity::new(
            id("run-test"),
            id("nonce-test"),
            id("logical-test"),
            id("crate::bench"),
            id("test-runner"),
        );
        let expected = ExpectedReportIdentity::new(identity.clone(), counts);
        let outcome = if success {
            ReportOutcome::Success
        } else {
            ReportOutcome::Failure {
                error: ReportFailure::new(id("benchmark_error"), "failed").expect("failure"),
            }
        };
        let observed = if success {
            counts
        } else {
            ReportCounts::observed(0, 1).expect("observed counts")
        };
        let envelope = mobench_domain::ReportEnvelopeV2::new(
            identity,
            counts,
            observed,
            if success { vec![1, 2, 3] } else { Vec::new() },
            outcome,
        )
        .expect("envelope");
        let binding = ProviderReportBinding::new(
            id("test-provider"),
            id("provider-run"),
            id(&format!("transport-{device}")),
            id(device),
            id(device),
        );
        let expected_binding = ExpectedProviderBinding::new(
            id("test-provider"),
            id("provider-run"),
            id(&format!("transport-{device}")),
            id(device),
        );
        expected
            .bind(envelope, binding, &expected_binding)
            .expect("bound report")
    }

    fn plan(sessions: &[&str]) -> ResolvedRunPlan {
        ResolvedRunPlan::new(
            "run-test",
            MobileTarget::Android,
            "crate::bench",
            ReportCounts::new(3, 1).expect("counts"),
            sessions.iter().map(|session| (*session).to_owned()),
        )
        .expect("plan")
    }

    #[test]
    fn lifecycle_classifies_complete_partial_and_failed_runs() {
        let complete = plan(&["pixel", "galaxy"])
            .begin()
            .collect(vec![
                bound_report("pixel", true),
                bound_report("galaxy", true),
            ])
            .expect("complete collection");
        assert_eq!(
            complete.outcome,
            RunOutcome::Complete {
                expected_sessions: 2,
                successful_sessions: 2
            }
        );

        let partial = plan(&["pixel", "galaxy"])
            .begin()
            .collect(vec![bound_report("pixel", true)])
            .expect("partial collection");
        assert_eq!(
            partial.outcome,
            RunOutcome::Partial {
                expected_sessions: 2,
                successful_sessions: 1
            }
        );

        let failed = plan(&["pixel"])
            .begin()
            .collect(vec![bound_report("pixel", false)])
            .expect("failed collection");
        assert_eq!(
            failed.outcome,
            RunOutcome::Failed {
                expected_sessions: 1,
                successful_sessions: 0
            }
        );
    }

    #[test]
    fn lifecycle_rejects_duplicate_and_unexpected_reports() {
        let duplicate = plan(&["pixel"])
            .begin()
            .collect(vec![
                bound_report("pixel", true),
                bound_report("pixel", true),
            ])
            .expect_err("duplicate must fail");
        assert!(matches!(duplicate, LifecycleError::DuplicateReport { .. }));

        let unexpected = plan(&["pixel"])
            .begin()
            .collect(vec![bound_report("galaxy", true)])
            .expect_err("unexpected must fail");
        assert!(matches!(
            unexpected,
            LifecycleError::UnexpectedReport { .. }
        ));
    }

    #[test]
    fn prepared_run_commits_canonical_and_compatibility_reports_together() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json_path = temp.path().join("results.json");
        let markdown_path = temp.path().join("results.md");
        let csv_path = temp.path().join("results.csv");
        let canonical_path = temp.path().join("summary.v2.json");
        let artifacts = ReportArtifacts::new(
            &json_path,
            serde_json::json!({"compatibility": true}),
            &markdown_path,
            "# Results\n".to_owned(),
            Some((&csv_path, "device,value\npixel,1\n".to_owned())),
            &canonical_path,
        )
        .expect("artifacts");

        let committed = plan(&["pixel"])
            .begin()
            .collect(vec![bound_report("pixel", true)])
            .expect("collection")
            .prepare(artifacts)
            .expect("prepare")
            .commit()
            .expect("commit");

        assert!(committed.outcome().is_complete());
        assert!(committed.published_path().is_dir());
        assert_eq!(committed.stable_paths().len(), 4);
        assert_eq!(
            fs::read_to_string(&markdown_path).expect("markdown"),
            "# Results\n"
        );
        let canonical: Value =
            serde_json::from_slice(&fs::read(&canonical_path).expect("canonical report"))
                .expect("canonical json");
        assert_eq!(canonical["schema_version"], "mobench.summary/v2");
        assert_eq!(canonical["lifecycle"]["status"], "complete");
        assert_eq!(canonical["reports"].as_array().map(Vec::len), Some(1));
        assert!(
            committed
                .published_path()
                .join("mobench-run-manifest.json")
                .is_file()
        );
    }
}
