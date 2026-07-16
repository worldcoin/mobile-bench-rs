//! Canonical report envelopes and identity validation.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser::SerializeMap};
use thiserror::Error;

pub use mobench_runtime::MAX_BENCHMARK_COUNT as MAX_REPORT_COUNT;

/// Canonical schema identifier for strict Mobench run reports.
pub const REPORT_SCHEMA_V2: &str = "mobench.run/v2";

/// Maximum UTF-8 byte length of a report identity component.
pub const MAX_REPORT_IDENTIFIER_BYTES: usize = 255;

/// Maximum accepted number of samples in one report.
pub const MAX_REPORT_SAMPLES: usize = MAX_REPORT_COUNT as usize;

/// Maximum UTF-8 byte length of a failure message.
pub const MAX_REPORT_FAILURE_MESSAGE_BYTES: usize = 4 * 1024;

/// A bounded, path-safe, nonempty component used in report identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReportIdentifier(String);

impl ReportIdentifier {
    /// Parse and validate one identity component.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReportIdentifierError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ReportIdentifierError::Empty);
        }
        if value.len() > MAX_REPORT_IDENTIFIER_BYTES {
            return Err(ReportIdentifierError::TooLong {
                length: value.len(),
                limit: MAX_REPORT_IDENTIFIER_BYTES,
            });
        }
        if value == "." || value == ".." {
            return Err(ReportIdentifierError::DotComponent);
        }
        if value.contains('/') || value.contains('\\') {
            return Err(ReportIdentifierError::PathSeparator);
        }
        if value.chars().any(char::is_control) {
            return Err(ReportIdentifierError::ControlCharacter);
        }
        if value.trim() != value {
            return Err(ReportIdentifierError::SurroundingWhitespace);
        }

        Ok(Self(value))
    }

    /// Access the validated component.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ReportIdentifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for ReportIdentifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ReportIdentifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

/// Why an identity component was rejected.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReportIdentifierError {
    #[error("report identifier is empty")]
    Empty,
    #[error("report identifier is {length} bytes; maximum is {limit}")]
    TooLong { length: usize, limit: usize },
    #[error("report identifier must not be '.' or '..'")]
    DotComponent,
    #[error("report identifier must not contain path separators")]
    PathSeparator,
    #[error("report identifier must not contain control characters")]
    ControlCharacter,
    #[error("report identifier must not have surrounding whitespace")]
    SurroundingWhitespace,
}

/// A bounded count used by report requests and observations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ReportCount(u32);

impl ReportCount {
    /// Construct a count no larger than [`MAX_REPORT_COUNT`].
    pub fn new(value: u32) -> Result<Self, ReportConstructionError> {
        if value > MAX_REPORT_COUNT {
            return Err(ReportConstructionError::CountTooLarge {
                value,
                limit: MAX_REPORT_COUNT,
            });
        }
        Ok(Self(value))
    }

    /// Return the numeric count.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl<'de> Deserialize<'de> for ReportCount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u32::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// Requested or observed benchmark counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportCounts {
    iterations: ReportCount,
    warmup: ReportCount,
}

impl ReportCounts {
    /// Construct benchmark counts. Measured iterations must be nonzero.
    pub fn new(iterations: u32, warmup: u32) -> Result<Self, ReportConstructionError> {
        if iterations == 0 {
            return Err(ReportConstructionError::ZeroIterations);
        }
        Ok(Self {
            iterations: ReportCount::new(iterations)?,
            warmup: ReportCount::new(warmup)?,
        })
    }

    /// Construct counts observed by a producer.
    ///
    /// A failed run may observe zero measured iterations. The enclosing
    /// envelope validates that observed counts never exceed the request.
    pub fn observed(iterations: u32, warmup: u32) -> Result<Self, ReportConstructionError> {
        Ok(Self {
            iterations: ReportCount::new(iterations)?,
            warmup: ReportCount::new(warmup)?,
        })
    }

    /// Number of measured iterations.
    pub const fn iterations(self) -> u32 {
        self.iterations.get()
    }

    /// Number of warmup iterations.
    pub const fn warmup(self) -> u32 {
        self.warmup.get()
    }

    fn validate_requested(self) -> Result<(), ReportValidationError> {
        if self.iterations() == 0 {
            return Err(ReportValidationError::ZeroIterations);
        }
        Ok(())
    }
}

/// All provenance fields that bind a report to one requested execution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportIdentity {
    run_id: ReportIdentifier,
    nonce: ReportIdentifier,
    logical_session_id: ReportIdentifier,
    function_id: ReportIdentifier,
    producer: ReportIdentifier,
}

impl ReportIdentity {
    /// Construct complete v2 provenance. Every component is required.
    pub fn new(
        run_id: ReportIdentifier,
        nonce: ReportIdentifier,
        logical_session_id: ReportIdentifier,
        function_id: ReportIdentifier,
        producer: ReportIdentifier,
    ) -> Self {
        Self {
            run_id,
            nonce,
            logical_session_id,
            function_id,
            producer,
        }
    }

    /// Requested run identifier.
    pub fn run_id(&self) -> &ReportIdentifier {
        &self.run_id
    }

    /// Unpredictable request nonce.
    pub fn nonce(&self) -> &ReportIdentifier {
        &self.nonce
    }

    /// Orchestrator-issued logical session, assigned before provider scheduling.
    ///
    /// This is deliberately distinct from any provider transport session ID,
    /// which is attached by a provider binding during collection.
    pub fn logical_session_id(&self) -> &ReportIdentifier {
        &self.logical_session_id
    }

    /// Requested benchmark function identifier.
    pub fn function_id(&self) -> &ReportIdentifier {
        &self.function_id
    }

    /// Component that produced the envelope.
    pub fn producer(&self) -> &ReportIdentifier {
        &self.producer
    }
}

/// Outcome of a completed report envelope.
///
/// Unknown status values retain their raw name for forward-compatible
/// decoding. Strict request validation still rejects them as non-terminal.
/// Additional fields are ignored for forward-compatible metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportOutcome {
    /// The samples are a successful benchmark result.
    Success,
    /// The bound run failed. Identity requirements are unchanged.
    Failure { error: ReportFailure },
    /// A future outcome retained for forward-compatible decoding.
    Unknown { status: String },
}

impl Serialize for ReportOutcome {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Success => map.serialize_entry("status", "success")?,
            Self::Failure { error } => {
                map.serialize_entry("status", "failure")?;
                map.serialize_entry("error", error)?;
            }
            Self::Unknown { status } => map.serialize_entry("status", status)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ReportOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct OutcomeWire {
            status: String,
            #[serde(default)]
            error: Option<ReportFailure>,
        }

        let wire = OutcomeWire::deserialize(deserializer)?;
        match wire.status.as_str() {
            "success" => Ok(Self::Success),
            "failure" => wire
                .error
                .map(|error| Self::Failure { error })
                .ok_or_else(|| de::Error::missing_field("error")),
            _ => Ok(Self::Unknown {
                status: wire.status,
            }),
        }
    }
}

/// Structured failure information for a v2 report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportFailure {
    code: ReportIdentifier,
    message: String,
}

impl ReportFailure {
    /// Construct validated failure information.
    pub fn new(
        code: ReportIdentifier,
        message: impl Into<String>,
    ) -> Result<Self, ReportConstructionError> {
        let message = message.into();
        validate_failure_message(&message).map_err(|error| match error {
            ReportValidationError::EmptyFailureMessage => {
                ReportConstructionError::EmptyFailureMessage
            }
            ReportValidationError::FailureMessageTooLong { length, limit } => {
                ReportConstructionError::FailureMessageTooLong { length, limit }
            }
            _ => unreachable!("failure message validation returned an unrelated error"),
        })?;
        Ok(Self { code, message })
    }

    /// Stable machine-readable failure code.
    pub fn code(&self) -> &ReportIdentifier {
        &self.code
    }

    /// Human-readable failure description.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Strict v2 report envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportEnvelopeV2 {
    schema_version: String,
    #[serde(flatten)]
    identity: ReportIdentity,
    requested: ReportCounts,
    observed: ReportCounts,
    samples_ns: BoundedSamples,
    outcome: ReportOutcome,
}

/// Authenticated provider evidence attached after a producer envelope is read
/// from one concrete provider transport session.
///
/// These fields are deliberately absent from [`ReportEnvelopeV2`]: remote
/// providers assign them after the mobile artifact has already been built.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderReportBinding {
    provider_id: ReportIdentifier,
    provider_run_id: ReportIdentifier,
    transport_session_id: ReportIdentifier,
    requested_device_id: ReportIdentifier,
    observed_device_id: ReportIdentifier,
}

impl ProviderReportBinding {
    /// Construct evidence returned by a provider Adapter.
    pub fn new(
        provider_id: ReportIdentifier,
        provider_run_id: ReportIdentifier,
        transport_session_id: ReportIdentifier,
        requested_device_id: ReportIdentifier,
        observed_device_id: ReportIdentifier,
    ) -> Self {
        Self {
            provider_id,
            provider_run_id,
            transport_session_id,
            requested_device_id,
            observed_device_id,
        }
    }

    /// Provider Adapter that supplied the evidence.
    pub fn provider_id(&self) -> &ReportIdentifier {
        &self.provider_id
    }

    /// Provider build or run identifier.
    pub fn provider_run_id(&self) -> &ReportIdentifier {
        &self.provider_run_id
    }

    /// Provider-assigned session identifier.
    pub fn transport_session_id(&self) -> &ReportIdentifier {
        &self.transport_session_id
    }

    /// Device requested for this transport session.
    pub fn requested_device_id(&self) -> &ReportIdentifier {
        &self.requested_device_id
    }

    /// Device reported by the provider for this transport session.
    pub fn observed_device_id(&self) -> &ReportIdentifier {
        &self.observed_device_id
    }
}

/// Provider transport evidence expected for one collected report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedProviderBinding {
    provider_id: ReportIdentifier,
    provider_run_id: ReportIdentifier,
    transport_session_id: ReportIdentifier,
    requested_device_id: ReportIdentifier,
}

impl ExpectedProviderBinding {
    /// Bind collection to one provider run, session, and requested device.
    pub fn new(
        provider_id: ReportIdentifier,
        provider_run_id: ReportIdentifier,
        transport_session_id: ReportIdentifier,
        requested_device_id: ReportIdentifier,
    ) -> Self {
        Self {
            provider_id,
            provider_run_id,
            transport_session_id,
            requested_device_id,
        }
    }

    /// Validate provider evidence without modifying producer identity.
    pub fn validate(&self, binding: &ProviderReportBinding) -> Result<(), ReportBindingError> {
        validate_binding_field("provider_id", &self.provider_id, &binding.provider_id)?;
        validate_binding_field(
            "provider_run_id",
            &self.provider_run_id,
            &binding.provider_run_id,
        )?;
        validate_binding_field(
            "transport_session_id",
            &self.transport_session_id,
            &binding.transport_session_id,
        )?;
        validate_binding_field(
            "requested_device_id",
            &self.requested_device_id,
            &binding.requested_device_id,
        )?;
        if binding.observed_device_id != binding.requested_device_id {
            return Err(ReportBindingError::ObservedDeviceMismatch {
                requested: binding.requested_device_id.clone(),
                observed: binding.observed_device_id.clone(),
            });
        }
        Ok(())
    }
}

/// Canonical report after producer identity and provider transport evidence
/// have both been validated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundRunReportV2 {
    envelope: ReportEnvelopeV2,
    binding: ProviderReportBinding,
}

impl BoundRunReportV2 {
    /// Validated producer envelope.
    pub fn envelope(&self) -> &ReportEnvelopeV2 {
        &self.envelope
    }

    /// Validated provider transport evidence.
    pub fn binding(&self) -> &ProviderReportBinding {
        &self.binding
    }
}

impl ReportEnvelopeV2 {
    /// Construct a structurally valid v2 envelope.
    pub fn new(
        identity: ReportIdentity,
        requested: ReportCounts,
        observed: ReportCounts,
        samples_ns: Vec<u64>,
        outcome: ReportOutcome,
    ) -> Result<Self, ReportConstructionError> {
        let samples_ns = BoundedSamples::new(samples_ns)?;
        let report = Self {
            schema_version: REPORT_SCHEMA_V2.to_owned(),
            identity,
            requested,
            observed,
            samples_ns,
            outcome,
        };
        report
            .validate_structure()
            .map_err(ReportConstructionError::from_validation)?;
        Ok(report)
    }

    /// Schema identifier supplied by the producer.
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Complete report provenance.
    pub fn identity(&self) -> &ReportIdentity {
        &self.identity
    }

    /// Counts requested by the orchestrator.
    pub const fn requested(&self) -> ReportCounts {
        self.requested
    }

    /// Counts observed by the report producer.
    pub const fn observed(&self) -> ReportCounts {
        self.observed
    }

    /// Measured samples in nanoseconds.
    pub fn samples_ns(&self) -> &[u64] {
        &self.samples_ns.0
    }

    /// Success or failure outcome.
    pub fn outcome(&self) -> &ReportOutcome {
        &self.outcome
    }

    fn validate_structure(&self) -> Result<(), ReportValidationError> {
        if self.schema_version != REPORT_SCHEMA_V2 {
            return Err(ReportValidationError::WrongSchema {
                expected: REPORT_SCHEMA_V2,
                observed: self.schema_version.clone(),
            });
        }
        self.requested.validate_requested()?;
        if self.samples_ns.0.len() > MAX_REPORT_SAMPLES {
            return Err(ReportValidationError::TooManySamples {
                count: self.samples_ns.0.len(),
                limit: MAX_REPORT_SAMPLES,
            });
        }
        match &self.outcome {
            ReportOutcome::Success => {
                self.observed.validate_requested()?;
                if self.observed != self.requested {
                    return Err(ReportValidationError::ObservedCountsMismatch {
                        requested: self.requested,
                        observed: self.observed,
                    });
                }
                if self.samples_ns.0.is_empty() {
                    return Err(ReportValidationError::EmptySuccessSamples);
                }
                if self.samples_ns.0.len() != self.observed.iterations() as usize {
                    return Err(ReportValidationError::SampleCountMismatch {
                        expected: self.observed.iterations(),
                        observed: self.samples_ns.0.len(),
                    });
                }
            }
            ReportOutcome::Failure { error } => {
                if self.observed.iterations() > self.requested.iterations()
                    || self.observed.warmup() > self.requested.warmup()
                {
                    return Err(ReportValidationError::ObservedCountsExceedRequested {
                        requested: self.requested,
                        observed: self.observed,
                    });
                }
                validate_failure_message(&error.message)?;
            }
            ReportOutcome::Unknown { status } => {
                return Err(ReportValidationError::UnknownOutcome {
                    status: status.clone(),
                });
            }
        }
        Ok(())
    }
}

/// Identity and request metadata expected by the report consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedReportIdentity {
    identity: ReportIdentity,
    requested: ReportCounts,
}

impl ExpectedReportIdentity {
    /// Bind validation to one complete execution request.
    pub fn new(identity: ReportIdentity, requested: ReportCounts) -> Self {
        Self {
            identity,
            requested,
        }
    }

    /// Validate a parsed strict-v2 report.
    pub fn validate(&self, report: &ReportEnvelopeV2) -> Result<(), ReportValidationError> {
        report.validate_structure()?;
        validate_identity_field("run_id", self.identity.run_id(), report.identity.run_id())?;
        validate_identity_field("nonce", self.identity.nonce(), report.identity.nonce())?;
        validate_identity_field(
            "logical_session_id",
            self.identity.logical_session_id(),
            report.identity.logical_session_id(),
        )?;
        validate_identity_field(
            "function_id",
            self.identity.function_id(),
            report.identity.function_id(),
        )?;
        validate_identity_field(
            "producer",
            self.identity.producer(),
            report.identity.producer(),
        )?;
        if report.requested != self.requested {
            return Err(ReportValidationError::RequestedCountsMismatch {
                expected: self.requested,
                observed: report.requested,
            });
        }
        Ok(())
    }

    /// Parse JSON as v2 and validate it against this execution request.
    ///
    /// Missing identity fields and malformed values fail closed here. This API
    /// never adapts legacy input; use [`adapt_legacy_v1_json`] explicitly.
    pub fn validate_json(&self, json: &[u8]) -> Result<ReportEnvelopeV2, ReportValidationError> {
        let report = serde_json::from_slice(json).map_err(|error| {
            ReportValidationError::InvalidEnvelope {
                message: error.to_string(),
            }
        })?;
        self.validate(&report)?;
        Ok(report)
    }

    /// Validate a producer envelope and bind it to authenticated provider
    /// transport evidence without rewriting either identity layer.
    pub fn bind(
        &self,
        report: ReportEnvelopeV2,
        binding: ProviderReportBinding,
        expected_binding: &ExpectedProviderBinding,
    ) -> Result<BoundRunReportV2, ReportBindingError> {
        self.validate(&report)?;
        expected_binding.validate(&binding)?;
        Ok(BoundRunReportV2 {
            envelope: report,
            binding,
        })
    }
}

fn validate_binding_field(
    field: &'static str,
    expected: &ReportIdentifier,
    observed: &ReportIdentifier,
) -> Result<(), ReportBindingError> {
    if observed != expected {
        return Err(ReportBindingError::ProviderFieldMismatch {
            field,
            expected: expected.clone(),
            observed: observed.clone(),
        });
    }
    Ok(())
}

fn validate_identity_field(
    field: &'static str,
    expected: &ReportIdentifier,
    observed: &ReportIdentifier,
) -> Result<(), ReportValidationError> {
    if observed != expected {
        return Err(ReportValidationError::IdentityMismatch {
            field,
            expected: expected.clone(),
            observed: observed.clone(),
        });
    }
    Ok(())
}

fn validate_failure_message(message: &str) -> Result<(), ReportValidationError> {
    if message.trim().is_empty() {
        return Err(ReportValidationError::EmptyFailureMessage);
    }
    if message.len() > MAX_REPORT_FAILURE_MESSAGE_BYTES {
        return Err(ReportValidationError::FailureMessageTooLong {
            length: message.len(),
            limit: MAX_REPORT_FAILURE_MESSAGE_BYTES,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BoundedSamples(Vec<u64>);

impl BoundedSamples {
    fn new(samples: Vec<u64>) -> Result<Self, ReportConstructionError> {
        if samples.len() > MAX_REPORT_SAMPLES {
            return Err(ReportConstructionError::TooManySamples {
                count: samples.len(),
                limit: MAX_REPORT_SAMPLES,
            });
        }
        Ok(Self(samples))
    }
}

impl Serialize for BoundedSamples {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BoundedSamples {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SamplesVisitor;

        impl<'de> de::Visitor<'de> for SamplesVisitor {
            type Value = BoundedSamples;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "at most {MAX_REPORT_SAMPLES} nanosecond samples")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let mut samples =
                    Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(MAX_REPORT_SAMPLES));
                while let Some(sample) = sequence.next_element()? {
                    if samples.len() == MAX_REPORT_SAMPLES {
                        return Err(de::Error::custom(format_args!(
                            "report has more than {MAX_REPORT_SAMPLES} samples"
                        )));
                    }
                    samples.push(sample);
                }
                Ok(BoundedSamples(samples))
            }
        }

        deserializer.deserialize_seq(SamplesVisitor)
    }
}

/// Strict validation or parsing failure for a v2 report.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReportValidationError {
    #[error("invalid v2 report envelope: {message}")]
    InvalidEnvelope { message: String },
    #[error("wrong report schema: expected {expected}, observed {observed}")]
    WrongSchema {
        expected: &'static str,
        observed: String,
    },
    #[error("report identity field {field} mismatched: expected {expected}, observed {observed}")]
    IdentityMismatch {
        field: &'static str,
        expected: ReportIdentifier,
        observed: ReportIdentifier,
    },
    #[error("requested report counts mismatched: expected {expected:?}, observed {observed:?}")]
    RequestedCountsMismatch {
        expected: ReportCounts,
        observed: ReportCounts,
    },
    #[error("observed report counts do not match requested counts")]
    ObservedCountsMismatch {
        requested: ReportCounts,
        observed: ReportCounts,
    },
    #[error("observed report counts exceed requested counts")]
    ObservedCountsExceedRequested {
        requested: ReportCounts,
        observed: ReportCounts,
    },
    #[error("unknown report outcome status {status}")]
    UnknownOutcome { status: String },
    #[error("a report must request at least one measured iteration")]
    ZeroIterations,
    #[error("successful report has no samples")]
    EmptySuccessSamples,
    #[error("successful report expected {expected} samples, observed {observed}")]
    SampleCountMismatch { expected: u32, observed: usize },
    #[error("report has {count} samples; maximum is {limit}")]
    TooManySamples { count: usize, limit: usize },
    #[error("failure report message is empty")]
    EmptyFailureMessage,
    #[error("failure report message is {length} bytes; maximum is {limit}")]
    FailureMessageTooLong { length: usize, limit: usize },
}

/// Failure while attaching provider transport evidence to a producer report.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReportBindingError {
    /// Producer envelope validation failed.
    #[error(transparent)]
    Report(#[from] ReportValidationError),
    /// Provider evidence did not match the expected collection context.
    #[error("provider binding field {field} mismatched: expected {expected}, observed {observed}")]
    ProviderFieldMismatch {
        field: &'static str,
        expected: ReportIdentifier,
        observed: ReportIdentifier,
    },
    /// Provider reported a different device from the requested matrix entry.
    #[error("provider observed device {observed}, expected requested device {requested}")]
    ObservedDeviceMismatch {
        requested: ReportIdentifier,
        observed: ReportIdentifier,
    },
}

/// Failure constructing typed report values locally.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReportConstructionError {
    #[error("report count {value} exceeds maximum {limit}")]
    CountTooLarge { value: u32, limit: u32 },
    #[error("a report must request at least one measured iteration")]
    ZeroIterations,
    #[error("report has {count} samples; maximum is {limit}")]
    TooManySamples { count: usize, limit: usize },
    #[error("successful report has no samples")]
    EmptySuccessSamples,
    #[error("successful report expected {expected} samples, observed {observed}")]
    SampleCountMismatch { expected: u32, observed: usize },
    #[error("observed report counts do not match requested counts")]
    ObservedCountsMismatch {
        requested: ReportCounts,
        observed: ReportCounts,
    },
    #[error("observed report counts exceed requested counts")]
    ObservedCountsExceedRequested {
        requested: ReportCounts,
        observed: ReportCounts,
    },
    #[error("unknown report outcome status {status}")]
    UnknownOutcome { status: String },
    #[error("failure report message is empty")]
    EmptyFailureMessage,
    #[error("failure report message is {length} bytes; maximum is {limit}")]
    FailureMessageTooLong { length: usize, limit: usize },
}

impl ReportConstructionError {
    fn from_validation(error: ReportValidationError) -> Self {
        match error {
            ReportValidationError::ZeroIterations => Self::ZeroIterations,
            ReportValidationError::EmptySuccessSamples => Self::EmptySuccessSamples,
            ReportValidationError::SampleCountMismatch { expected, observed } => {
                Self::SampleCountMismatch { expected, observed }
            }
            ReportValidationError::TooManySamples { count, limit } => {
                Self::TooManySamples { count, limit }
            }
            ReportValidationError::ObservedCountsMismatch {
                requested,
                observed,
            } => Self::ObservedCountsMismatch {
                requested,
                observed,
            },
            ReportValidationError::ObservedCountsExceedRequested {
                requested,
                observed,
            } => Self::ObservedCountsExceedRequested {
                requested,
                observed,
            },
            ReportValidationError::UnknownOutcome { status } => Self::UnknownOutcome { status },
            ReportValidationError::EmptyFailureMessage => Self::EmptyFailureMessage,
            ReportValidationError::FailureMessageTooLong { length, limit } => {
                Self::FailureMessageTooLong { length, limit }
            }
            ReportValidationError::InvalidEnvelope { .. }
            | ReportValidationError::WrongSchema { .. }
            | ReportValidationError::IdentityMismatch { .. }
            | ReportValidationError::RequestedCountsMismatch { .. } => {
                unreachable!("local v2 construction sets schema and has no expected identity")
            }
        }
    }
}

/// Explicitly reduced-provenance data adapted from the legacy v1 shape.
///
/// This type intentionally lacks v2 run, nonce, device, session, and producer
/// identity and cannot be passed to [`ExpectedReportIdentity::validate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReducedProvenanceReport {
    function_id: ReportIdentifier,
    samples_ns: Vec<u64>,
    reported_iterations: Option<ReportCount>,
    reported_warmup: Option<ReportCount>,
}

impl ReducedProvenanceReport {
    /// Legacy function label, not cryptographically or session-bound.
    pub fn function_id(&self) -> &ReportIdentifier {
        &self.function_id
    }

    /// Legacy samples in nanoseconds.
    pub fn samples_ns(&self) -> &[u64] {
        &self.samples_ns
    }

    /// Iterations declared by legacy input, when present.
    pub fn reported_iterations(&self) -> Option<u32> {
        self.reported_iterations.map(ReportCount::get)
    }

    /// Warmup count declared by legacy input, when present.
    pub fn reported_warmup(&self) -> Option<u32> {
        self.reported_warmup.map(ReportCount::get)
    }
}

#[derive(Deserialize)]
struct LegacyV1WireReport {
    function: ReportIdentifier,
    samples_ns: BoundedSamples,
    iterations: Option<ReportCount>,
    warmup: Option<ReportCount>,
}

#[derive(Deserialize)]
struct VersionProbe {
    #[serde(default)]
    schema_version: Option<String>,
}

/// Parse only the legacy v1 report shape into an explicitly reduced type.
pub fn adapt_legacy_v1_json(json: &[u8]) -> Result<ReducedProvenanceReport, LegacyV1AdapterError> {
    let version: VersionProbe = serde_json::from_slice(json)
        .map_err(|error| LegacyV1AdapterError::InvalidLegacyReport(error.to_string()))?;
    if let Some(schema_version) = version.schema_version {
        return Err(LegacyV1AdapterError::VersionedEnvelope { schema_version });
    }
    let legacy: LegacyV1WireReport = serde_json::from_slice(json)
        .map_err(|error| LegacyV1AdapterError::InvalidLegacyReport(error.to_string()))?;
    Ok(ReducedProvenanceReport {
        function_id: legacy.function,
        samples_ns: legacy.samples_ns.0,
        reported_iterations: legacy.iterations,
        reported_warmup: legacy.warmup,
    })
}

/// Failure adapting an explicitly requested legacy v1 report.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum LegacyV1AdapterError {
    #[error("invalid legacy v1 report: {0}")]
    InvalidLegacyReport(String),
    #[error("legacy v1 adapter refuses versioned envelope {schema_version}")]
    VersionedEnvelope { schema_version: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_reject_unsafe_components() {
        for value in ["", ".", "..", "a/b", "a\\b", " a", "a\n"] {
            assert!(
                ReportIdentifier::parse(value).is_err(),
                "accepted {value:?}"
            );
        }
        assert!(ReportIdentifier::parse("sample_fns::checksum").is_ok());
        assert!(ReportIdentifier::parse("Pixel 9-16.0").is_ok());
    }

    #[test]
    fn counts_are_bounded() {
        assert_eq!(
            ReportCount::new(MAX_REPORT_COUNT).unwrap().get(),
            MAX_REPORT_COUNT
        );
        assert_eq!(
            ReportCount::new(MAX_REPORT_COUNT + 1),
            Err(ReportConstructionError::CountTooLarge {
                value: MAX_REPORT_COUNT + 1,
                limit: MAX_REPORT_COUNT,
            })
        );
        assert_eq!(
            ReportCounts::observed(0, 0)
                .expect("failed run may observe no iterations")
                .iterations(),
            0
        );
    }
}
