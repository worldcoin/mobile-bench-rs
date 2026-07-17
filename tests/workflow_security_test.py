#!/usr/bin/env python3
"""Static regression tests for the reusable BrowserStack trust boundary."""

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / ".github/workflows/reusable-bench.yml").read_text()
PLOT_WORKFLOW = (ROOT / ".github/workflows/mobile-bench-publish-plots.yml").read_text()


def job(name: str, next_name: str | None = None) -> str:
    start = WORKFLOW.index(f"  {name}:\n")
    if next_name is None:
        return WORKFLOW[start:]
    return WORKFLOW[start : WORKFLOW.index(f"  {next_name}:\n", start + 1)]


def test_global_and_job_permissions() -> None:
    assert re.search(r"^permissions: \{\}$", WORKFLOW, re.MULTILINE)
    for name, following in (
        ("prepare-ios", "prepare-android"),
        ("prepare-android", "run-ios"),
    ):
        text = job(name, following)
        assert "permissions:\n      contents: read" in text
        assert "contents: write" not in text
        assert "pull-requests: write" not in text
        assert "checks: write" not in text
        assert "environment:" not in text
        assert "BROWSERSTACK_USERNAME" not in text
        assert "BROWSERSTACK_ACCESS_KEY" not in text
        assert "actions/cache" not in text


def test_pr_revision_is_current_and_exact() -> None:
    validation = job("validate-request", "trusted-mobench")
    assert "^[0-9a-fA-F]{40}$" in validation
    assert "pulls/${PR_NUMBER}" in validation
    assert "Requested SHA is not the current head" in validation
    for name, following in (
        ("prepare-ios", "prepare-android"),
        ("prepare-android", "run-ios"),
    ):
        text = job(name, following)
        assert "Revalidate current PR head" in text
        assert "persist-credentials: false" in text
        assert "ref: ${{ needs.validate-request.outputs.head_sha }}" in text
    for name, following in (("run-ios", "run-android"), ("run-android", "summarize")):
        assert "Revalidate current PR head before credential use" in job(name, following)


def test_trusted_control_plane_is_from_a_literal_commit() -> None:
    match = re.search(r"^  MOBENCH_TRUSTED_SHA: ([0-9a-f]{40})$", WORKFLOW, re.MULTILINE)
    assert match, "trusted mobench must be pinned to an immutable commit"
    trusted = job("trusted-mobench", "prepare-ios")
    assert "repository: worldcoin/mobile-bench-rs" in trusted
    assert "ref: ${{ env.MOBENCH_TRUSTED_SHA }}" in trusted
    assert "github.workflow_sha" not in trusted


def test_untrusted_uploads_are_enumerated() -> None:
    ios = job("prepare-ios", "prepare-android")
    android = job("prepare-android", "run-ios")
    assert "manifest.json" in ios and "entries/*/app.ipa" in ios
    assert "entries/*/test-suite.zip" in ios
    assert "manifest.json" in android and "entries/*/app.apk" in android
    assert "entries/*/test.apk" in android
    assert "target/mobench/prebuilt/ios/**" not in ios
    assert "target/mobench/prebuilt/android/**" not in android


def test_credentialed_jobs_never_checkout_or_build_caller_code() -> None:
    for name, following in (("run-ios", "run-android"), ("run-android", "summarize")):
        text = job(name, following)
        assert "environment: browserstack" in text
        assert "actions/checkout" not in text
        assert "caller" not in text.lower()
        assert not re.search(r"\b(cargo (?:build|install|run)|gradle|xcodebuild)\b", text)
        assert "ci run-prebuilt" in text
        assert "--expected-source-sha" in text
        assert "--expected-platform" in text
        assert "--expected-functions" in text
        assert "--expected-iterations" in text
        assert "--expected-warmup" in text
        secret_step = text.index("BROWSERSTACK_USERNAME")
        run_step = text.index("ci run-prebuilt")
        assert secret_step < run_step
        # One env binding (the key plus its secret expression) and nowhere else.
        assert text.count("BROWSERSTACK_USERNAME") == 2
        assert text.count("BROWSERSTACK_ACCESS_KEY") == 2


def test_reporting_is_separate_and_has_no_checkout() -> None:
    summarize = job("summarize", "report")
    report = job("report")
    assert "actions/checkout" not in summarize + report
    assert "pull-requests: write" not in summarize
    assert "checks: write" not in summarize
    assert "pull-requests: write" in report and "checks: write" in report
    assert "contents: write" not in report
    assert "contents: write" not in WORKFLOW
    assert "workflow_dispatch:" in PLOT_WORKFLOW
    assert "environment: mobench-plots" in PLOT_WORKFLOW
    assert "contents: write" in PLOT_WORKFLOW
    assert "--plots require" in PLOT_WORKFLOW


def test_downloaded_reports_are_treated_as_untrusted() -> None:
    summarize = job("summarize", "report")
    assert "Reject unsafe report paths and fields" in summarize
    assert "p.is_symlink()" in summarize
    assert "report nesting too deep" in summarize
    assert "unsafe report field" in summarize
    assert "unsafe SVG" in PLOT_WORKFLOW


def test_all_external_actions_are_immutable() -> None:
    refs = re.findall(
        r"^\s*uses:\s*([^\s#]+)", WORKFLOW + "\n" + PLOT_WORKFLOW, re.MULTILINE
    )
    assert refs
    for ref in refs:
        assert re.search(r"@[0-9a-f]{40}$", ref), ref


def test_malicious_fixture_covers_required_attack_surfaces() -> None:
    fixture = ROOT / "tests/fixtures/malicious-pr"
    texts = "\n".join(p.read_text() for p in fixture.rglob("*") if p.is_file())
    for needle in (
        "build.rs",
        "BROWSERSTACK_USERNAME",
        "BROWSERSTACK_ACCESS_KEY",
        "GITHUB_TOKEN",
        "fixture-hook",
        "dependency-build",
        "git push",
    ):
        assert needle in texts


if __name__ == "__main__":
    tests = [value for name, value in sorted(globals().items()) if name.startswith("test_")]
    for test in tests:
        test()
    print(f"ok: {len(tests)} workflow security tests")
