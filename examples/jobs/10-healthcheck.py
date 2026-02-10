#!/usr/bin/env python3
# licron: */15 * * * *
# licron-timeout: 45s

from datetime import datetime, timezone
from pathlib import Path

stamp = datetime.now(timezone.utc).isoformat()
status_file = Path('/tmp/licron-healthcheck.txt')
status_file.write_text(f'healthcheck ok at {stamp}\n', encoding='utf-8')
print(f'updated {status_file}')
