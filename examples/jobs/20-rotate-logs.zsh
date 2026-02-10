#!/usr/bin/env zsh
# licron: 0 */6 * * *
# licron-timeout: 2m

set -euo pipefail

log_dir="${TMPDIR:-/tmp}/licron-app-logs"
archive_dir="$log_dir/archive"
mkdir -p "$archive_dir"

for f in "$log_dir"/*.log(.N); do
  mv "$f" "$archive_dir/${f:t}.$(date +%Y%m%d%H%M%S)"
done

echo "rotation finished at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
