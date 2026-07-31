//! Strict producer-envelope validation and provider transport binding.

use std::fmt::Write;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{MobileTarget, RunSpec};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RunEnvelopeIdentity {
    pub(crate) run_id: String,
    pub(crate) nonce: String,
    pub(crate) logical_session_id: String,
    pub(crate) producer: String,
}

impl RunEnvelopeIdentity {
    pub(crate) fn generate(target: MobileTarget) -> Result<Self> {
        Ok(Self {
            run_id: format!("run-{}", random_hex::<16>()?),
            nonce: format!("nonce-{}", random_hex::<32>()?),
            logical_session_id: format!("logical-session-{}", random_hex::<16>()?),
            producer: match target {
                MobileTarget::Android => "android-runner",
                MobileTarget::Ios => "ios-runner",
            }
            .to_owned(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn bind_report_value(
    report: &Value,
    identity: &RunEnvelopeIdentity,
    spec: &RunSpec,
    provider_id: &str,
    provider_run_id: &str,
    transport_session_id: &str,
    requested_device_id: &str,
    observed_device_id: &str,
) -> Result<mobench_domain::BoundRunReportV2> {
    let identifier = |value: &str, field: &str| {
        mobench_domain::ReportIdentifier::parse(value.to_owned())
            .with_context(|| format!("invalid {field} in v2 report binding"))
    };
    let expected = mobench_domain::ExpectedReportIdentity::new(
        mobench_domain::ReportIdentity::new(
            identifier(&identity.run_id, "run_id")?,
            identifier(&identity.nonce, "nonce")?,
            identifier(&identity.logical_session_id, "logical_session_id")?,
            identifier(&spec.function, "function_id")?,
            identifier(&identity.producer, "producer")?,
        ),
        mobench_domain::ReportCounts::new(spec.iterations, spec.warmup)
            .context("invalid requested v2 report counts")?,
    );
    let encoded = serde_json::to_vec(report).context("serializing collected v2 report")?;
    let envelope = expected
        .validate_json(&encoded)
        .context("collected producer report failed strict v2 validation")?;
    let binding = mobench_domain::ProviderReportBinding::new(
        identifier(provider_id, "provider_id")?,
        identifier(provider_run_id, "provider_run_id")?,
        identifier(transport_session_id, "transport_session_id")?,
        identifier(requested_device_id, "requested_device_id")?,
        identifier(observed_device_id, "observed_device_id")?,
    );
    let expected_binding = mobench_domain::ExpectedProviderBinding::new(
        identifier(provider_id, "provider_id")?,
        identifier(provider_run_id, "provider_run_id")?,
        identifier(transport_session_id, "transport_session_id")?,
        identifier(requested_device_id, "requested_device_id")?,
        identifier(observed_device_id, "observed_device_id")?,
    );
    expected
        .bind(envelope, binding, &expected_binding)
        .context("collected report failed provider binding validation")
}

fn random_hex<const N: usize>() -> Result<String> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|error| anyhow!("generating run identity: {error}"))?;
    let mut encoded = String::with_capacity(N * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(encoded)
}
