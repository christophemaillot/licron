#!/usr/bin/env bash
# licron: 5 1 * * *
# licron-timeout: 10m

set -euo pipefail

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
out_dir="${TMPDIR:-/tmp}/licron-backups"
mkdir -p "$out_dir"

echo "[$stamp] starting backup" >>"$out_dir/backup.log"
tar -czf "$out_dir/home-$stamp.tgz" "$HOME" >/dev/null 2>&1 || true
echo "[$stamp] backup done" >>"$out_dir/backup.log"
