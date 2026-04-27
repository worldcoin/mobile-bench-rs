#!/usr/bin/env bash
set -euo pipefail

fixture="${1:-}"

if [ -z "${fixture}" ]; then
  echo "usage: $0 <basic|ffi>" >&2
  exit 1
fi

case "${fixture}" in
  basic)
    summary_path="examples/fixtures/basic/summary.json"
    output_dir="target/mobench/plot-fixtures/basic"
    expected_plots=("fibonacci.svg" "checksum.svg")
    ;;
  ffi)
    summary_path="examples/fixtures/ffi/summary.json"
    output_dir="target/mobench/plot-fixtures/ffi"
    expected_plots=("fibonacci.svg" "checksum.svg")
    ;;
  *)
    echo "unknown fixture: ${fixture}" >&2
    exit 1
    ;;
esac

rm -rf "${output_dir}"
mkdir -p "${output_dir}"

mobench report summarize \
  --summary "${summary_path}" \
  --output "${output_dir}/summary.md" \
  --plots require

markdown="${output_dir}/summary.md"

if ! grep -Fq "## Device Comparison Plots" "${markdown}"; then
  echo "expected Device Comparison Plots section in ${markdown}" >&2
  exit 1
fi

for plot in "${expected_plots[@]}"; do
  plot_path="${output_dir}/plots/${plot}"
  if [ ! -s "${plot_path}" ]; then
    echo "expected rendered plot at ${plot_path}" >&2
    exit 1
  fi
  if ! grep -Fq "](plots/${plot})" "${markdown}"; then
    echo "expected markdown link for ${plot}" >&2
    exit 1
  fi
done
