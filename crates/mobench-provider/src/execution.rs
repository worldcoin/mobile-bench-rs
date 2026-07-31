//! Provider execution Interface and Module-owned lifecycle policy.

use std::collections::HashMap;
use std::error::Error;
use std::num::NonZeroU8;

use mobench_process::ProcessCancellation;
use thiserror::Error;

use crate::{
    CollectedSession, ExpectedSession, MatrixAssessment, ReconcileError, reconcile_sessions,
};

/// One collected Provider Session with its report payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectedOutput<Report> {
    /// Provider session identifier used for correlation.
    pub session_id: String,
    /// Reports attributed to this session before cardinality validation.
    pub reports: Vec<Report>,
    /// Optional already-sanitized failure summary from the Adapter.
    pub failure: Option<String>,
}

/// Terminal evidence returned by an Adapter before reconciliation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterRun<Report> {
    /// Terminal Provider Sessions expected for the requested matrix.
    pub expected: Vec<ExpectedSession>,
    /// Collected reports and failures, in arbitrary Adapter order.
    pub collected: Vec<CollectedOutput<Report>>,
}

impl<Report> AdapterRun<Report> {
    /// Reconcile terminal Adapter evidence into deterministic Provider receipts.
    pub fn reconcile(self) -> Result<ProviderRun<Report>, ReconcileError> {
        let summaries = self
            .collected
            .iter()
            .map(|session| CollectedSession {
                session_id: session.session_id.clone(),
                report_count: session.reports.len(),
                failure: session.failure.clone(),
            })
            .collect::<Vec<_>>();
        let assessment = reconcile_sessions(&self.expected, &summaries)?;

        let mut collected_by_session = self
            .collected
            .into_iter()
            .map(|session| (session.session_id.clone(), session))
            .collect::<HashMap<_, _>>();
        let sessions = self
            .expected
            .iter()
            .filter_map(|expected| collected_by_session.remove(&expected.session_id))
            .collect();

        Ok(ProviderRun {
            assessment,
            sessions,
        })
    }
}

/// Reconciled Provider Run with deterministic session ordering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRun<Report> {
    assessment: MatrixAssessment,
    sessions: Vec<CollectedOutput<Report>>,
}

impl<Report> ProviderRun<Report> {
    /// Complete/partial/failed assessment for the entire Provider Run.
    pub const fn assessment(&self) -> &MatrixAssessment {
        &self.assessment
    }

    /// Collected Provider Sessions in expected-matrix order.
    ///
    /// Missing sessions have no entry here and remain visible in
    /// [`Self::assessment`].
    pub fn sessions(&self) -> &[CollectedOutput<Report>] {
        &self.sessions
    }

    /// Consume the receipt and collected payloads.
    pub fn into_parts(self) -> (MatrixAssessment, Vec<CollectedOutput<Report>>) {
        (self.assessment, self.sessions)
    }
}

/// Adapter Interface at the provider Seam.
///
/// The Adapter owns provider-specific transport, credentials, artifact upload,
/// polling, and report extraction. [`ProviderEngine`] owns call ordering,
/// cooperative cancellation, collection retry policy, and reconciliation.
pub trait ProviderAdapter {
    /// Resolved request consumed by this Adapter.
    type Request;
    /// Durable handle returned after the Provider Run starts.
    type Handle;
    /// One collected report payload.
    type Report;
    /// Provider-specific failure.
    type Error: Error + Send + Sync + 'static;

    /// Start a Provider Run without waiting for terminal reports.
    fn start(
        &self,
        request: &Self::Request,
        cancellation: &ProcessCancellation,
    ) -> Result<Self::Handle, Self::Error>;

    /// Collect terminal session evidence for a previously started run.
    fn collect(
        &self,
        handle: &Self::Handle,
        cancellation: &ProcessCancellation,
    ) -> Result<AdapterRun<Self::Report>, Self::Error>;

    /// Best-effort cancellation of a started Provider Run.
    fn cancel(
        &self,
        handle: &Self::Handle,
        cancellation: &ProcessCancellation,
    ) -> Result<(), Self::Error>;

    /// Whether collection can safely be retried after this failure.
    fn is_collect_retryable(&self, _error: &Self::Error) -> bool {
        false
    }
}

/// A Provider Run that has started but has not yet been collected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartedRun<Handle> {
    handle: Handle,
}

impl<Handle> StartedRun<Handle> {
    /// Resume collection from an Adapter-specific durable handle.
    pub const fn from_handle(handle: Handle) -> Self {
        Self { handle }
    }

    /// Borrow the Adapter-specific durable handle.
    pub const fn handle(&self) -> &Handle {
        &self.handle
    }

    /// Consume the typestate and return its durable handle.
    pub fn into_handle(self) -> Handle {
        self.handle
    }
}

/// Provider lifecycle stage attached to execution failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionStage {
    /// Before or while starting the Provider Run.
    Start,
    /// While collecting terminal evidence.
    Collect,
    /// While cancelling a started Provider Run.
    Cancel,
}

/// Provider execution failure with stable lifecycle context.
#[derive(Debug, Error)]
pub enum ProviderExecutionError<AdapterError>
where
    AdapterError: Error + 'static,
{
    /// Cooperative cancellation was requested.
    #[error("provider execution cancelled during {stage:?}")]
    Cancelled { stage: ExecutionStage },
    /// The Adapter failed during one lifecycle stage.
    #[error("provider adapter failed during {stage:?} after {attempts} attempt(s): {source}")]
    Adapter {
        /// Lifecycle stage that failed.
        stage: ExecutionStage,
        /// Number of calls made at this stage.
        attempts: u8,
        /// Adapter-specific source error.
        #[source]
        source: AdapterError,
    },
    /// Cancellation was requested but the Adapter could not cancel the run.
    #[error("provider adapter failed to cancel a started run: {source}")]
    Cancellation {
        /// Adapter-specific source error.
        #[source]
        source: AdapterError,
    },
    /// Terminal evidence was structurally ambiguous.
    #[error("provider session reconciliation failed: {0}")]
    Reconcile(#[from] ReconcileError),
}

/// Module-owned provider lifecycle with bounded collection retries.
#[derive(Clone, Debug)]
pub struct ProviderEngine<Adapter> {
    adapter: Adapter,
    max_collect_attempts: NonZeroU8,
}

impl<Adapter> ProviderEngine<Adapter>
where
    Adapter: ProviderAdapter,
{
    /// Construct an engine with one collection attempt.
    pub fn new(adapter: Adapter) -> Self {
        Self {
            adapter,
            max_collect_attempts: NonZeroU8::MIN,
        }
    }

    /// Set the maximum number of collection attempts.
    ///
    /// Start is never retried because remote scheduling is not assumed to be
    /// idempotent. Only failures explicitly marked retryable by the Adapter are
    /// retried.
    pub fn with_max_collect_attempts(adapter: Adapter, max_collect_attempts: NonZeroU8) -> Self {
        Self {
            adapter,
            max_collect_attempts,
        }
    }

    /// Borrow the concrete Adapter.
    pub const fn adapter(&self) -> &Adapter {
        &self.adapter
    }

    /// Start a Provider Run.
    pub fn start(
        &self,
        request: &Adapter::Request,
        cancellation: &ProcessCancellation,
    ) -> Result<StartedRun<Adapter::Handle>, ProviderExecutionError<Adapter::Error>> {
        if cancellation.is_cancelled() {
            return Err(ProviderExecutionError::Cancelled {
                stage: ExecutionStage::Start,
            });
        }
        let handle = self
            .adapter
            .start(request, cancellation)
            .map_err(|source| ProviderExecutionError::Adapter {
                stage: ExecutionStage::Start,
                attempts: 1,
                source,
            })?;
        Ok(StartedRun { handle })
    }

    /// Collect and reconcile a started Provider Run.
    pub fn collect(
        &self,
        started: StartedRun<Adapter::Handle>,
        cancellation: &ProcessCancellation,
    ) -> Result<ProviderRun<Adapter::Report>, ProviderExecutionError<Adapter::Error>> {
        let mut attempts = 0u8;
        loop {
            if cancellation.is_cancelled() {
                self.adapter
                    .cancel(&started.handle, cancellation)
                    .map_err(|source| ProviderExecutionError::Cancellation { source })?;
                return Err(ProviderExecutionError::Cancelled {
                    stage: ExecutionStage::Collect,
                });
            }

            attempts = attempts.saturating_add(1);
            match self.adapter.collect(&started.handle, cancellation) {
                Ok(run) => return run.reconcile().map_err(Into::into),
                Err(source) => {
                    if cancellation.is_cancelled() {
                        self.adapter
                            .cancel(&started.handle, cancellation)
                            .map_err(|source| ProviderExecutionError::Cancellation { source })?;
                        return Err(ProviderExecutionError::Cancelled {
                            stage: ExecutionStage::Collect,
                        });
                    }
                    let retryable = self.adapter.is_collect_retryable(&source);
                    if !retryable || attempts >= self.max_collect_attempts.get() {
                        return Err(ProviderExecutionError::Adapter {
                            stage: ExecutionStage::Collect,
                            attempts,
                            source,
                        });
                    }
                }
            }
        }
    }

    /// Start, collect, and reconcile one Provider Run.
    pub fn execute(
        &self,
        request: &Adapter::Request,
        cancellation: &ProcessCancellation,
    ) -> Result<ProviderRun<Adapter::Report>, ProviderExecutionError<Adapter::Error>> {
        let started = self.start(request, cancellation)?;
        self.collect(started, cancellation)
    }

    /// Cancel a started Provider Run.
    pub fn cancel(
        &self,
        started: StartedRun<Adapter::Handle>,
        cancellation: &ProcessCancellation,
    ) -> Result<(), ProviderExecutionError<Adapter::Error>> {
        self.adapter
            .cancel(&started.handle, cancellation)
            .map_err(|source| ProviderExecutionError::Adapter {
                stage: ExecutionStage::Cancel,
                attempts: 1,
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fmt;

    use super::*;
    use crate::{MatrixOutcome, SessionDisposition};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FakeFailure {
        Retryable,
        Permanent,
        Cancel,
    }

    impl fmt::Display for FakeFailure {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "{self:?}")
        }
    }

    impl Error for FakeFailure {}

    struct FakeAdapter {
        start_calls: Cell<u8>,
        collect_calls: Cell<u8>,
        cancel_calls: Cell<u8>,
        retryable_failures: Cell<u8>,
        permanent_failure: bool,
        cancel_failure: bool,
        unexpected_session: bool,
    }

    impl FakeAdapter {
        fn successful() -> Self {
            Self {
                start_calls: Cell::new(0),
                collect_calls: Cell::new(0),
                cancel_calls: Cell::new(0),
                retryable_failures: Cell::new(0),
                permanent_failure: false,
                cancel_failure: false,
                unexpected_session: false,
            }
        }

        fn terminal_run(&self) -> AdapterRun<u64> {
            let expected = vec![
                ExpectedSession {
                    session_id: "session-a".to_owned(),
                    device_id: "device-a".to_owned(),
                    status: "passed".to_owned(),
                },
                ExpectedSession {
                    session_id: "session-b".to_owned(),
                    device_id: "device-b".to_owned(),
                    status: "passed".to_owned(),
                },
            ];
            let mut collected = vec![
                CollectedOutput {
                    session_id: "session-b".to_owned(),
                    reports: vec![22],
                    failure: None,
                },
                CollectedOutput {
                    session_id: "session-a".to_owned(),
                    reports: vec![11],
                    failure: None,
                },
            ];
            if self.unexpected_session {
                collected.push(CollectedOutput {
                    session_id: "unexpected".to_owned(),
                    reports: vec![33],
                    failure: None,
                });
            }
            AdapterRun {
                expected,
                collected,
            }
        }
    }

    impl ProviderAdapter for FakeAdapter {
        type Request = ();
        type Handle = &'static str;
        type Report = u64;
        type Error = FakeFailure;

        fn start(
            &self,
            _request: &Self::Request,
            _cancellation: &ProcessCancellation,
        ) -> Result<Self::Handle, Self::Error> {
            self.start_calls.set(self.start_calls.get() + 1);
            Ok("run-1")
        }

        fn collect(
            &self,
            _handle: &Self::Handle,
            _cancellation: &ProcessCancellation,
        ) -> Result<AdapterRun<Self::Report>, Self::Error> {
            self.collect_calls.set(self.collect_calls.get() + 1);
            if self.permanent_failure {
                return Err(FakeFailure::Permanent);
            }
            if self.retryable_failures.get() > 0 {
                self.retryable_failures
                    .set(self.retryable_failures.get() - 1);
                return Err(FakeFailure::Retryable);
            }
            Ok(self.terminal_run())
        }

        fn cancel(
            &self,
            _handle: &Self::Handle,
            _cancellation: &ProcessCancellation,
        ) -> Result<(), Self::Error> {
            self.cancel_calls.set(self.cancel_calls.get() + 1);
            if self.cancel_failure {
                Err(FakeFailure::Cancel)
            } else {
                Ok(())
            }
        }

        fn is_collect_retryable(&self, error: &Self::Error) -> bool {
            matches!(error, FakeFailure::Retryable)
        }
    }

    #[test]
    fn execute_orders_collected_payloads_by_expected_matrix() {
        let adapter = FakeAdapter::successful();
        let engine = ProviderEngine::new(adapter);
        let run = engine
            .execute(&(), &ProcessCancellation::default())
            .unwrap();

        assert_eq!(run.assessment().outcome(), MatrixOutcome::Complete);
        assert_eq!(run.sessions()[0].session_id, "session-a");
        assert_eq!(run.sessions()[0].reports, vec![11]);
        assert_eq!(run.sessions()[1].session_id, "session-b");
        assert_eq!(run.sessions()[1].reports, vec![22]);
        assert_eq!(engine.adapter().start_calls.get(), 1);
        assert_eq!(engine.adapter().collect_calls.get(), 1);
    }

    #[test]
    fn explicit_start_then_collect_uses_the_same_handle() {
        let engine = ProviderEngine::new(FakeAdapter::successful());
        let cancellation = ProcessCancellation::default();
        let started = engine.start(&(), &cancellation).unwrap();
        assert_eq!(*started.handle(), "run-1");
        let run = engine.collect(started, &cancellation).unwrap();
        assert!(run.assessment().is_complete());
    }

    #[test]
    fn retryable_collection_failure_is_bounded() {
        let adapter = FakeAdapter::successful();
        adapter.retryable_failures.set(2);
        let engine = ProviderEngine::with_max_collect_attempts(adapter, NonZeroU8::new(3).unwrap());
        let run = engine
            .execute(&(), &ProcessCancellation::default())
            .unwrap();
        assert!(run.assessment().is_complete());
        assert_eq!(engine.adapter().collect_calls.get(), 3);
    }

    #[test]
    fn retryable_collection_failure_stops_at_the_limit() {
        let adapter = FakeAdapter::successful();
        adapter.retryable_failures.set(3);
        let engine = ProviderEngine::with_max_collect_attempts(adapter, NonZeroU8::new(2).unwrap());
        let error = engine
            .execute(&(), &ProcessCancellation::default())
            .unwrap_err();
        assert!(matches!(
            error,
            ProviderExecutionError::Adapter {
                stage: ExecutionStage::Collect,
                attempts: 2,
                source: FakeFailure::Retryable,
            }
        ));
    }

    #[test]
    fn permanent_collection_failure_is_not_retried() {
        let mut adapter = FakeAdapter::successful();
        adapter.permanent_failure = true;
        let engine = ProviderEngine::with_max_collect_attempts(adapter, NonZeroU8::new(5).unwrap());
        let error = engine
            .execute(&(), &ProcessCancellation::default())
            .unwrap_err();
        assert!(matches!(
            error,
            ProviderExecutionError::Adapter {
                stage: ExecutionStage::Collect,
                attempts: 1,
                source: FakeFailure::Permanent,
            }
        ));
    }

    #[test]
    fn cancellation_before_start_never_calls_the_adapter() {
        let cancellation = ProcessCancellation::default();
        cancellation.cancel();
        let engine = ProviderEngine::new(FakeAdapter::successful());
        let error = engine.start(&(), &cancellation).unwrap_err();
        assert!(matches!(
            error,
            ProviderExecutionError::Cancelled {
                stage: ExecutionStage::Start
            }
        ));
        assert_eq!(engine.adapter().start_calls.get(), 0);
    }

    #[test]
    fn cancellation_after_start_cancels_the_adapter_before_returning() {
        let cancellation = ProcessCancellation::default();
        let engine = ProviderEngine::new(FakeAdapter::successful());
        let started = engine.start(&(), &cancellation).unwrap();
        cancellation.cancel();
        let error = engine.collect(started, &cancellation).unwrap_err();
        assert!(matches!(
            error,
            ProviderExecutionError::Cancelled {
                stage: ExecutionStage::Collect
            }
        ));
        assert_eq!(engine.adapter().cancel_calls.get(), 1);
        assert_eq!(engine.adapter().collect_calls.get(), 0);
    }

    #[test]
    fn cancellation_failure_is_not_hidden() {
        let cancellation = ProcessCancellation::default();
        let mut adapter = FakeAdapter::successful();
        adapter.cancel_failure = true;
        let engine = ProviderEngine::new(adapter);
        let started = engine.start(&(), &cancellation).unwrap();
        cancellation.cancel();
        let error = engine.collect(started, &cancellation).unwrap_err();
        assert!(matches!(
            error,
            ProviderExecutionError::Cancellation {
                source: FakeFailure::Cancel
            }
        ));
    }

    #[test]
    fn reconciliation_ambiguity_is_owned_by_the_engine() {
        let mut adapter = FakeAdapter::successful();
        adapter.unexpected_session = true;
        let engine = ProviderEngine::new(adapter);
        let error = engine
            .execute(&(), &ProcessCancellation::default())
            .unwrap_err();
        assert!(matches!(
            error,
            ProviderExecutionError::Reconcile(ReconcileError::UnexpectedCollectedSession { .. })
        ));
    }

    #[test]
    fn partial_runs_preserve_receipts_and_present_payloads() {
        let run = AdapterRun {
            expected: vec![
                ExpectedSession {
                    session_id: "session-a".to_owned(),
                    device_id: "device-a".to_owned(),
                    status: "passed".to_owned(),
                },
                ExpectedSession {
                    session_id: "session-b".to_owned(),
                    device_id: "device-b".to_owned(),
                    status: "passed".to_owned(),
                },
            ],
            collected: vec![CollectedOutput {
                session_id: "session-a".to_owned(),
                reports: vec![11],
                failure: None,
            }],
        }
        .reconcile()
        .unwrap();

        assert_eq!(run.assessment().outcome(), MatrixOutcome::Partial);
        assert_eq!(run.sessions().len(), 1);
        assert_eq!(
            run.assessment().sessions()[1].disposition,
            SessionDisposition::Missing
        );
    }
}
