#!/usr/bin/env python3
# licron: 0 3 * * 0
# licron-timeout: 5m

from pathlib import Path

root = Path('/tmp/licron-cleanup')
root.mkdir(parents=True, exist_ok=True)

removed = 0
for p in root.glob('*.tmp'):
    p.unlink(missing_ok=True)
    removed += 1

print(f'removed {removed} temp files from {root}')
