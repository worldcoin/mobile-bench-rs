use mobench_webhook::ingest::manifest::HistoryManifest;

#[test]
fn parses_manifest_with_dispatch_and_pr_comment_metadata() {
    let raw = include_str!("fixtures/mobench-history-v1/manifest.json");
    let manifest: HistoryManifest = serde_json::from_str(raw).unwrap();

    assert_eq!(manifest.schema_version, "mobench-history-v1");
    assert_eq!(manifest.request.trigger_source, "pr_comment");
    assert_eq!(
        manifest.request.request_command.as_deref(),
        Some("/mobench platform=both iterations=30 warmup=5 device_profile=low-spec")
    );
}
