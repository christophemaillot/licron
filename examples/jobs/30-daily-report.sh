#!/usr/bin/env bash
# licron: 30 7 * * 1-5
# licron-timeout: 90s

set -euo pipefail

report_dir="${TMPDIR:-/tmp}/licron-reports"
mkdir -p "$report_dir"

report_file="$report_dir/daily-$(date +%F).txt"
{
  echo "Daily report"
  echo "Generated at: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "Job name: ${LICRON_JOB_NAME:-unknown}"
  echo "Schedule: ${LICRON_SCHEDULE:-unknown}"
} >"$report_file"

echo "wrote $report_file"
