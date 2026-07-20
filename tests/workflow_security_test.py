#!/usr/bin/env python3
"""Static regression tests for the reusable BrowserStack trust boundary."""

from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile
import textwrap


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / ".github/workflows/reusable-bench.yml").read_text()
PLOT_WORKFLOW = (ROOT / ".github/workflows/mobile-bench-publish-plots.yml").read_text()
PR_COMMAND = (ROOT / ".github/workflows/reusable-pr-command.yml").read_text()
PR_AUTO = (ROOT / ".github/workflows/reusable-pr-auto.yml").read_text()
SELFTEST_WORKFLOW = (ROOT / ".github/workflows/mobile-bench-selftest.yml").read_text()
GENERATED_WORKFLOW = (ROOT / "crates/mobench/templates/ci/mobile-bench.yml").read_text()
GENERATED_ACTION = (ROOT / "crates/mobench/templates/ci/action.yml").read_text()


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
        assert "pull-requests: read" in text
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
    assert "for attempt in 1 2 3 4 5" in validation
    assert "Unable to validate the current PR head after 5 attempts" in validation
    for name, following in (
        ("prepare-ios", "prepare-android"),
        ("prepare-android", "run-ios"),
    ):
        text = job(name, following)
        assert "Revalidate current PR head" in text
        assert "for attempt in 1 2 3 4 5" in text
        assert "Unable to revalidate the current PR head after 5 attempts" in text
        assert "persist-credentials: false" in text
        assert "ref: ${{ needs.validate-request.outputs.head_sha }}" in text
    for name, following in (("run-ios", "run-android"), ("run-android", "summarize")):
        text = job(name, following)
        assert "Revalidate current PR head before credential use" in text
        assert "for attempt in 1 2 3 4 5" in text
        assert "Unable to revalidate the current PR head after 5 attempts" in text
    assert WORKFLOW.count("for attempt in 1 2 3 4 5") == 5
    assert "pr_number is required unless allow_non_pr is explicitly enabled" in validation
    assert "${current,,}" not in WORKFLOW
    assert "tr '[:upper:]' '[:lower:]'" in WORKFLOW
    assert "allow_non_pr: true" in SELFTEST_WORKFLOW


def test_pr_command_dispatches_fork_heads() -> None:
    assert "head.repo.full_name" not in PR_COMMAND
    assert "skipping secret-bearing BrowserStack dispatch" not in PR_COMMAND
    assert "^[0-9a-fA-F]{40}$" in PR_COMMAND
    assert '-f head_sha="$HEAD_SHA"' in PR_COMMAND
    assert '-f pr_number="$PR_NUMBER"' in PR_COMMAND
    assert "skipping secret-bearing BrowserStack dispatch" not in PR_AUTO
    assert '-f head_sha="$HEAD_SHA"' in PR_AUTO
    assert '-f pr_number="$PR_NUMBER"' in PR_AUTO


def test_trusted_control_plane_is_from_a_literal_commit() -> None:
    match = re.search(r"^  MOBENCH_TRUSTED_SHA: ([0-9a-f]{40})$", WORKFLOW, re.MULTILINE)
    assert match, "trusted mobench must be pinned to an immutable commit"
    trusted = job("trusted-mobench", "prepare-ios")
    assert "repository: worldcoin/mobile-bench-rs" in trusted
    assert "ref: ${{ env.MOBENCH_TRUSTED_SHA }}" in trusted
    assert "github.workflow_sha" not in trusted
    cargo_toml = subprocess.run(
        ["git", "show", f"{match.group(1)}:Cargo.toml"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=True,
    ).stdout
    assert 'version = "0.1.46"' in cargo_toml


def test_caller_toolchain_is_explicit_and_confined_to_prepare_jobs() -> None:
    assert re.search(
        r"rust_toolchain:\n(?:.*\n){3}\s+default: stable", WORKFLOW
    )
    assert "default: aarch64-apple-ios,aarch64-apple-ios-sim,x86_64-apple-ios" in WORKFLOW
    validation = job("validate-request", "trusted-mobench")
    assert "rust_toolchain contains unsupported characters" in validation

    trusted = job("trusted-mobench", "prepare-ios")
    assert "toolchain: stable" in trusted
    assert "needs.validate-request.outputs.rust_toolchain" not in trusted

    for name, following, targets in (
        ("prepare-ios", "prepare-android", "inputs.rust_targets_ios"),
        ("prepare-android", "run-ios", "inputs.rust_targets_android"),
    ):
        text = job(name, following)
        assert "toolchain: ${{ needs.validate-request.outputs.rust_toolchain }}" in text
        assert f"targets: ${{{{ {targets} }}}}" in text
        assert "rustup target add" not in text

    credentialed = job("run-ios", "summarize")
    assert "rust_toolchain" not in credentialed
    assert "nightly-2026-03-04" in SELFTEST_WORKFLOW
    assert "crate_path: examples/ffi-benchmark" in SELFTEST_WORKFLOW
    assert "export_native_c_abi!()" in (
        ROOT / "examples/ffi-benchmark/src/lib.rs"
    ).read_text()


def test_ffi_backend_is_validated_and_passed_without_config_rewrite() -> None:
    validation = job("validate-request", "trusted-mobench")
    assert "ffi_backend must be uniffi, native-c-abi, or boltffi" in validation
    assert "bolt-ffi) FFI_BACKEND=boltffi" in validation
    for name, following in (
        ("prepare-ios", "prepare-android"),
        ("prepare-android", "run-ios"),
    ):
        text = job(name, following)
        assert 'backend_args+=(--ffi-backend "$FFI_BACKEND")' in text
        assert '"${backend_args[@]}"' in text
        assert "Configure requested FFI backend" not in text
        assert "path.write_text" not in text
    assert "ffi_backend: native-c-abi" in SELFTEST_WORKFLOW


def test_binding_generator_uses_crate_workspace_lockfile() -> None:
    for name, following in (
        ("prepare-ios", "prepare-android"),
        ("prepare-android", "run-ios"),
    ):
        text = job(name, following)
        assert "find caller -name Cargo.lock" not in text
        assert 'candidate = (root / os.environ[\'CRATE_PATH\']).resolve(strict=True)' in text
        assert "candidate.relative_to(root)" in text
        assert 'cargo metadata --manifest-path "$manifest"' in text
        assert 'lockfile="$workspace_root/Cargo.lock"' in text
        assert "native-c-abi) exit 0" in text
        bolt = text.index("boltffi|bolt-ffi)")
        uniffi = text.index("uniffi) ;;", bolt)
        metadata = text.index('cargo metadata --manifest-path "$manifest"', uniffi)
        assert bolt < uniffi < metadata


def test_untrusted_uploads_are_enumerated() -> None:
    ios = job("prepare-ios", "prepare-android")
    android = job("prepare-android", "run-ios")
    assert "manifest.json" in ios and "entries/*/app.ipa" in ios
    assert "entries/*/test-suite.zip" in ios
    assert "manifest.json" in android and "entries/*/app.apk" in android
    assert "entries/*/test.apk" in android
    assert "target/mobench/prebuilt/ios/**" not in ios
    assert "target/mobench/prebuilt/android/**" not in android


def test_prepare_hook_is_confined_validated_and_fail_closed() -> None:
    assert 'prepare_script:' in WORKFLOW
    for name, following, platform, label in (
        ("prepare-ios", "prepare-android", "ios", "iOS"),
        ("prepare-android", "run-ios", "android", "Android"),
    ):
        text = job(name, following)
        hook = text.index("Run caller preparation hook (untrusted)")
        prepare = text.index(f"Prepare {label} packages and manifest")
        upload = text.index(f"Upload explicitly enumerated {label} packages")
        assert hook < prepare < upload
        assert 'MOBENCH_CI_PREPARE: "1"' in text
        assert f"MOBENCH_PLATFORM: {platform}" in text
        assert "prepare_script must be a normalized repository-relative path" in text
        assert "prepare_script is missing or escapes the caller checkout" in text
        assert "resolved.relative_to(root)" in text
        assert "resolved.is_file()" in text
        assert 'bash -- "$script"' in text
        assert "continue-on-error" not in text

    credentialed = job("run-ios", "summarize")
    assert "prepare_script" not in credentialed
    assert "MOBENCH_CI_PREPARE" not in credentialed


def test_prepare_hook_path_validator_rejects_unsafe_targets() -> None:
    prepare = job("prepare-ios", "prepare-android")
    marker = 'script="$(python3 - <<\'PY\'\n'
    start = prepare.index(marker) + len(marker)
    end = prepare.index("\n          PY", start)
    validator = textwrap.dedent(prepare[start:end])

    with tempfile.TemporaryDirectory(prefix="mobench-hook-path-") as temp:
        root = Path(temp)
        safe = root / "scripts/prepare.sh"
        safe.parent.mkdir()
        safe.write_text("#!/bin/sh\n")
        outside = root.parent / f"{root.name}-outside.sh"
        outside.write_text("#!/bin/sh\n")
        (root / "escape.sh").symlink_to(outside)

        def validate(value: str) -> subprocess.CompletedProcess[str]:
            env = os.environ.copy()
            env["PREPARE_SCRIPT"] = value
            return subprocess.run(
                ["python3", "-c", validator],
                cwd=root,
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )

        assert validate("scripts/prepare.sh").returncode == 0
        for unsafe in (
            "/tmp/prepare.sh",
            "../prepare.sh",
            "./scripts/prepare.sh",
            "scripts//prepare.sh",
            "scripts\\prepare.sh",
            "missing.sh",
            "scripts",
            "escape.sh",
            "bad\nname.sh",
        ):
            assert validate(unsafe).returncode != 0, unsafe
        outside.unlink()


def test_platform_functions_and_structured_devices_are_bound_end_to_end() -> None:
    validation = job("validate-request", "trusted-mobench")
    assert "FUNCTIONS_IOS" in validation and "FUNCTIONS_ANDROID" in validation
    assert "ios_functions = os.environ['FUNCTIONS_IOS'].strip() or shared" in validation
    assert "android_functions = os.environ['FUNCTIONS_ANDROID'].strip() or shared" in validation
    assert "functions_ios: ${{ steps.validate.outputs.functions_ios }}" in validation
    assert "functions_android: ${{ steps.validate.outputs.functions_android }}" in validation
    assert "set(item) != {'device', 'os_version'}" in validation
    assert "normalize_devices(os.environ['IOS_DEVICES'], 'ios_devices')" in validation
    assert "normalize_devices(os.environ['ANDROID_DEVICES'], 'android_devices')" in validation

    ios_prepare = job("prepare-ios", "prepare-android")
    android_prepare = job("prepare-android", "run-ios")
    ios_run = job("run-ios", "run-android")
    android_run = job("run-android", "summarize")
    assert "needs.validate-request.outputs.functions_ios" in ios_prepare + ios_run
    assert "needs.validate-request.outputs.functions_android" in android_prepare + android_run
    assert "needs.validate-request.outputs.ios_devices" in ios_run
    assert "needs.validate-request.outputs.android_devices" in android_run
    for text in (ios_run, android_run):
        assert 'device_args+=(--devices "$spec")' in text
        assert '"${device_args[@]}"' in text
        assert "unique | length" in text
        assert "DEVICE_OVERRIDE" in text and "DEVICE_PROFILE" in text


def test_reporting_uses_one_fixed_complete_result_set_per_platform() -> None:
    summarize = job("summarize", "report")
    assert 'summary="results/$platform/summary.json"' in summarize
    assert 'csv="results/$platform/results.csv"' in summarize
    assert "find " not in summarize
    assert "-print -quit" not in summarize


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
        assert "--max-completion-timeout-secs" in text
        assert "MAX_COMPLETION_TIMEOUT_SECS: ${{ inputs.max_completion_timeout_secs }}" in text
        secret_step = text.index("BROWSERSTACK_USERNAME")
        run_step = text.index("ci run-prebuilt")
        assert secret_step < run_step
        # One env binding (the key plus its secret expression) and nowhere else.
        assert text.count("BROWSERSTACK_USERNAME") == 2
        assert text.count("BROWSERSTACK_ACCESS_KEY") == 2


def test_completion_timeout_has_a_bounded_trusted_default() -> None:
    assert "max_completion_timeout_secs:" in WORKFLOW
    assert 'description: "Trusted upper bound for BrowserStack completion waits (maximum 21600)"' in WORKFLOW
    assert "type: number\n        default: 1800" in WORKFLOW
    assert WORKFLOW.count('--max-completion-timeout-secs "$MAX_COMPLETION_TIMEOUT_SECS"') == 2


def test_browserstack_platform_jobs_are_serialized() -> None:
    ios = job("run-ios", "run-android")
    android = job("run-android", "summarize")
    assert "if: inputs.platform == 'ios' || inputs.platform == 'both'" in ios
    assert "needs: [validate-request, trusted-mobench, prepare-android, run-ios]" in android
    assert "needs.run-ios.result == 'success'" in android
    assert "needs.run-ios.result == 'failure'" in android
    assert "needs.run-ios.result == 'skipped'" in android


def test_reporting_is_separate_and_has_no_checkout() -> None:
    summarize = job("summarize", "report")
    report = job("report")
    assert "actions/checkout" not in summarize + report
    assert "pull-requests: write" not in summarize
    assert "checks: write" not in summarize
    assert "pull-requests: write" in report and "checks: write" in report
    assert "if: always() && inputs.pr_number != ''" in report
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


def test_security_boundary_external_actions_are_immutable() -> None:
    refs = re.findall(
        r"^\s*uses:\s*([^\s#]+)", WORKFLOW + "\n" + PLOT_WORKFLOW, re.MULTILINE
    )
    assert refs
    for ref in refs:
        assert re.search(r"@[0-9a-f]{40}$", ref), ref


def test_generated_workflow_uses_secure_reusable_boundary() -> None:
    assert "actions/checkout" not in GENERATED_WORKFLOW
    assert "runs-on:" not in GENERATED_WORKFLOW
    assert "worldcoin/mobile-bench-rs/.github/workflows/reusable-bench.yml@" in GENERATED_WORKFLOW
    workflow_ref = re.search(r"reusable-bench\.yml@([0-9a-f]{40})", GENERATED_WORKFLOW)
    assert workflow_ref
    pinned_workflow = subprocess.run(
        ["git", "show", f"{workflow_ref.group(1)}:.github/workflows/reusable-bench.yml"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=True,
    ).stdout
    assert "MOBENCH_TRUSTED_SHA: b7e7203c5ea50f1b109c94b5c04f2a43b6111977" in pinned_workflow
    assert "rust_toolchain:" in GENERATED_WORKFLOW
    assert "ffi_backend:" in GENERATED_WORKFLOW
    assert "prepare_script:" in GENERATED_WORKFLOW
    assert "functions_ios:" in GENERATED_WORKFLOW
    assert "functions_android:" in GENERATED_WORKFLOW
    assert "ios_devices:" in GENERATED_WORKFLOW
    assert "android_devices:" in GENERATED_WORKFLOW
    assert "secrets: inherit" not in GENERATED_WORKFLOW
    assert "BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}" in GENERATED_WORKFLOW
    assert "BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}" in GENERATED_WORKFLOW
    for ref in re.findall(r"^\s*uses:\s*([^\s#]+)", GENERATED_ACTION, re.MULTILINE):
        assert ref.startswith("./") or re.search(r"@[0-9a-f]{40}$", ref), ref


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
    dependency_build = (fixture / "dependency/build.rs").read_text()
    assert "BROWSERSTACK_USERNAME" in dependency_build
    assert "git push origin HEAD:refs/heads/malicious-dependency-build" in dependency_build


def test_malicious_fixture_executes_without_secrets_or_repository_write() -> None:
    source_fixture = ROOT / "tests/fixtures/malicious-pr"
    with tempfile.TemporaryDirectory(prefix="mobench-malicious-pr-") as temp:
        temp_root = Path(temp)
        fixture = temp_root / "fixture"
        shutil.copytree(source_fixture, fixture)
        log_dir = temp_root / "logs"
        shim_dir = temp_root / "bin"
        log_dir.mkdir()
        shim_dir.mkdir()
        git_shim = shim_dir / "git"
        git_shim.write_text(
            "#!/bin/sh\n"
            "printf '%s\\n' \"$*\" >> \"$MOBENCH_ATTACK_LOG_DIR/git-push-attempts.txt\"\n"
            "exit 1\n"
        )
        git_shim.chmod(0o755)

        env = os.environ.copy()
        for name in (
            "BROWSERSTACK_USERNAME",
            "BROWSERSTACK_ACCESS_KEY",
            "GITHUB_TOKEN",
            "GH_TOKEN",
        ):
            env.pop(name, None)
        env["MOBENCH_ATTACK_LOG_DIR"] = str(log_dir)
        env["MOBENCH_CI_PREPARE"] = "1"
        env["CARGO_TARGET_DIR"] = str(temp_root / "target")
        env["PATH"] = f"{shim_dir}{os.pathsep}{env['PATH']}"

        subprocess.run(
            ["cargo", "build", "--quiet", "--manifest-path", str(fixture / "Cargo.toml")],
            cwd=ROOT,
            env=env,
            check=True,
        )
        hook = subprocess.run(
            ["bash", str(fixture / "fixture-hook.sh")],
            cwd=fixture,
            env=env,
            check=False,
        )
        assert hook.returncode != 0, "fixture hook push unexpectedly succeeded"
        subprocess.run(
            [
                "cargo",
                "run",
                "--quiet",
                "--manifest-path",
                str(fixture / "Cargo.toml"),
                "--bin",
                "probe",
            ],
            cwd=ROOT,
            env=env,
            check=True,
        )

        for name in (
            "build.rs-BROWSERSTACK_USERNAME",
            "build.rs-BROWSERSTACK_ACCESS_KEY",
            "build.rs-GITHUB_TOKEN",
        ):
            assert (log_dir / name).read_text() == ""
        assert (log_dir / "dependency-build-secrets.txt").read_text() == "dependency-build:::"
        assert (log_dir / "fixture-hook-secrets.txt").read_text() == "fixture-hook::::prepare=1\n"
        assert (log_dir / "benchmark-secrets.txt").read_text() == "::"
        attempts = (log_dir / "git-push-attempts.txt").read_text().splitlines()
        assert len(attempts) == 4
        assert all(line.startswith("push origin HEAD:refs/heads/malicious-") for line in attempts)


if __name__ == "__main__":
    tests = [value for name, value in sorted(globals().items()) if name.startswith("test_")]
    for test in tests:
        test()
    print(f"ok: {len(tests)} workflow security tests")
