use mobench_domain::{
    ExpectedProviderBinding, ExpectedReportIdentity, LegacyV1AdapterError, ProviderReportBinding,
    REPORT_SCHEMA_V2, ReportBindingError, ReportCounts, ReportEnvelopeV2, ReportIdentifier,
    ReportIdentity, ReportOutcome, ReportValidationError, adapt_legacy_v1_json,
};

fn identifier(value: &str) -> ReportIdentifier {
    ReportIdentifier::parse(value).expect("test identifier is valid")
}

fn provider_binding() -> ProviderReportBinding {
    ProviderReportBinding::new(
        identifier("browserstack"),
        identifier("build-456"),
        identifier("transport-session-789"),
        identifier("Google-Pixel-7-13.0"),
        identifier("Google-Pixel-7-13.0"),
    )
}

fn expected_provider_binding() -> ExpectedProviderBinding {
    ExpectedProviderBinding::new(
        identifier("browserstack"),
        identifier("build-456"),
        identifier("transport-session-789"),
        identifier("Google-Pixel-7-13.0"),
    )
}

fn identity() -> ReportIdentity {
    ReportIdentity::new(
        identifier("run-20260715-001"),
        identifier("nonce-7f31a9"),
        identifier("logical-session-123"),
        identifier("sample_fns::checksum"),
        identifier("android-runner"),
    )
}

fn expected() -> ExpectedReportIdentity {
    ExpectedReportIdentity::new(
        identity(),
        ReportCounts::new(3, 1).expect("test counts are valid"),
    )
}

#[test]
fn v2_producer_constructor_emits_logical_identity_without_transport_claims() {
    let counts = ReportCounts::new(3, 1).expect("test counts are valid");
    let report = ReportEnvelopeV2::new(
        identity(),
        counts,
        counts,
        vec![101, 99, 100],
        ReportOutcome::Success,
    )
    .expect("construct valid report");
    let json = serde_json::to_value(report).expect("serialize report");

    assert_eq!(json["schema_version"], REPORT_SCHEMA_V2);
    for field in [
        "run_id",
        "nonce",
        "logical_session_id",
        "function_id",
        "producer",
    ] {
        assert!(json.get(field).is_some(), "missing producer field {field}");
    }
    for field in [
        "provider_id",
        "provider_build_id",
        "transport_session_id",
        "device_id",
    ] {
        assert!(
            json.get(field).is_none(),
            "producer envelope must not claim transport field {field}"
        );
    }
}

#[test]
fn provider_transport_binding_is_attached_only_after_envelope_validation() {
    let counts = ReportCounts::new(3, 1).expect("test counts are valid");
    let report = ReportEnvelopeV2::new(
        identity(),
        counts,
        counts,
        vec![101, 99, 100],
        ReportOutcome::Success,
    )
    .expect("construct valid report");

    let bound = expected()
        .bind(report, provider_binding(), &expected_provider_binding())
        .expect("bind validated producer report to authenticated transport evidence");
    let json = serde_json::to_value(bound).expect("serialize bound report");

    assert_eq!(
        json["envelope"]["logical_session_id"],
        "logical-session-123"
    );
    assert_eq!(json["binding"]["provider_id"], "browserstack");
    assert_eq!(json["binding"]["provider_run_id"], "build-456");
    assert_eq!(
        json["binding"]["transport_session_id"],
        "transport-session-789"
    );
    assert_eq!(
        json["binding"]["requested_device_id"],
        "Google-Pixel-7-13.0"
    );
}

#[test]
fn provider_binding_rejects_observed_device_drift() {
    let counts = ReportCounts::new(3, 1).expect("test counts are valid");
    let report = ReportEnvelopeV2::new(
        identity(),
        counts,
        counts,
        vec![101, 99, 100],
        ReportOutcome::Success,
    )
    .expect("construct valid report");
    let binding = ProviderReportBinding::new(
        identifier("browserstack"),
        identifier("build-456"),
        identifier("transport-session-789"),
        identifier("Google-Pixel-7-13.0"),
        identifier("Google-Pixel-8-14.0"),
    );

    let error = expected()
        .bind(report, binding, &expected_provider_binding())
        .unwrap_err();

    assert!(matches!(
        error,
        ReportBindingError::ObservedDeviceMismatch { requested, observed }
            if requested.as_str() == "Google-Pixel-7-13.0"
                && observed.as_str() == "Google-Pixel-8-14.0"
    ));
}

#[test]
fn validates_success_golden_and_ignores_unknown_object_fields() {
    let report = expected()
        .validate_json(include_bytes!("golden/v2-success.json"))
        .expect("valid success envelope");

    assert_eq!(report.schema_version(), REPORT_SCHEMA_V2);
    assert_eq!(report.samples_ns(), [101, 99, 100]);
    assert_eq!(report.outcome(), &ReportOutcome::Success);
}

#[test]
fn validates_failure_golden_with_the_same_complete_identity() {
    let report = expected()
        .validate_json(include_bytes!("golden/v2-failure.json"))
        .expect("valid failure envelope");

    let ReportOutcome::Failure { error } = report.outcome() else {
        panic!("expected failure outcome");
    };
    assert_eq!(error.code().as_str(), "worker-exit");
    assert_eq!(error.message(), "benchmark worker exited before sampling");
    assert!(report.samples_ns().is_empty());
    assert_eq!(report.observed().iterations(), 0);
    assert_eq!(report.observed().warmup(), 0);
}

#[test]
fn rejects_each_mismatched_identity_field_from_golden_json() {
    let cases: [(&str, &[u8]); 5] = [
        ("run_id", include_bytes!("golden/mismatch-run-id.json")),
        ("nonce", include_bytes!("golden/mismatch-nonce.json")),
        (
            "logical_session_id",
            include_bytes!("golden/mismatch-session-id.json"),
        ),
        (
            "function_id",
            include_bytes!("golden/mismatch-function-id.json"),
        ),
        ("producer", include_bytes!("golden/mismatch-producer.json")),
    ];

    for (expected_field, json) in cases {
        let error = expected().validate_json(json).unwrap_err();
        assert!(
            matches!(
                error,
                ReportValidationError::IdentityMismatch { field, .. }
                    if field == expected_field
            ),
            "unexpected error for {expected_field}: {error:?}"
        );
    }
}

#[test]
fn rejects_failure_with_mismatched_logical_session() {
    let error = expected()
        .validate_json(include_bytes!("golden/failure-mismatch-session-id.json"))
        .unwrap_err();

    assert!(matches!(
        error,
        ReportValidationError::IdentityMismatch {
            field: "logical_session_id",
            ..
        }
    ));
}

#[test]
fn rejects_wrong_schema_golden() {
    let error = expected()
        .validate_json(include_bytes!("golden/wrong-schema.json"))
        .unwrap_err();

    assert_eq!(
        error,
        ReportValidationError::WrongSchema {
            expected: REPORT_SCHEMA_V2,
            observed: "mobench.run/v3".to_owned(),
        }
    );
}

#[test]
fn rejects_requested_counts_that_do_not_match_the_request() {
    let error = expected()
        .validate_json(include_bytes!("golden/wrong-requested-counts.json"))
        .unwrap_err();

    assert!(matches!(
        error,
        ReportValidationError::RequestedCountsMismatch { .. }
    ));
}

#[test]
fn rejects_observed_counts_that_do_not_match_requested_counts() {
    let error = expected()
        .validate_json(include_bytes!("golden/wrong-observed-counts.json"))
        .unwrap_err();

    assert!(matches!(
        error,
        ReportValidationError::ObservedCountsMismatch { .. }
    ));
}

#[test]
fn rejects_missing_logical_session_instead_of_fabricating_it() {
    let error = expected()
        .validate_json(include_bytes!("golden/missing-session-id.json"))
        .unwrap_err();

    assert!(matches!(
        error,
        ReportValidationError::InvalidEnvelope { .. }
    ));
    assert!(error.to_string().contains("logical_session_id"));
}

#[test]
fn rejects_empty_success_samples() {
    let error = expected()
        .validate_json(include_bytes!("golden/empty-success-samples.json"))
        .unwrap_err();

    assert_eq!(error, ReportValidationError::EmptySuccessSamples);
}

#[test]
fn legacy_golden_requires_explicit_reduced_provenance_adapter() {
    let json = include_bytes!("golden/legacy-v1.json");
    assert!(matches!(
        expected().validate_json(json),
        Err(ReportValidationError::InvalidEnvelope { .. })
    ));

    let legacy = adapt_legacy_v1_json(json).expect("explicitly adapt legacy report");
    assert_eq!(legacy.function_id().as_str(), "sample_fns::checksum");
    assert_eq!(legacy.samples_ns(), [101, 99, 100]);
    assert_eq!(legacy.reported_iterations(), Some(3));
    assert_eq!(legacy.reported_warmup(), Some(1));
}

#[test]
fn legacy_adapter_refuses_versioned_envelopes() {
    let error = adapt_legacy_v1_json(include_bytes!("golden/v2-success.json")).unwrap_err();

    assert_eq!(
        error,
        LegacyV1AdapterError::VersionedEnvelope {
            schema_version: REPORT_SCHEMA_V2.to_owned(),
        }
    );
}

#[test]
fn unknown_outcome_status_fails_closed() {
    let json = include_bytes!("golden/unknown-outcome.json");
    let error = expected().validate_json(json).unwrap_err();

    assert_eq!(
        error,
        ReportValidationError::UnknownOutcome {
            status: "partial".to_owned(),
        }
    );
}

#[test]
fn failure_observations_cannot_exceed_requested_counts() {
    let mut json: serde_json::Value =
        serde_json::from_slice(include_bytes!("golden/v2-failure.json")).expect("parse fixture");
    json["observed"]["iterations"] = 4.into();

    let error = expected()
        .validate_json(&serde_json::to_vec(&json).expect("serialize fixture"))
        .unwrap_err();

    assert!(matches!(
        error,
        ReportValidationError::ObservedCountsExceedRequested { .. }
    ));
}
