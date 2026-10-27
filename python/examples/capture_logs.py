#! /usr/bin/env python3
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Print a device's decoded logs, filtered (replaces `uclog-cli.py`).

    python capture_logs.py -l build/app.elf --seconds 10
    python capture_logs.py --level WARN --source 'sensor' --grep 'temp=\\d+'

Error records (logs that could not be decoded, framing errors) are always printed.
"""

import argparse
import re

import uclog
from uclog.logdata import LEVELS

ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument('-l', '--logdata', action='append', help='.logdata or ELF file (repeatable)')
ap.add_argument('--host', help='hub address (default: find or start one)')
ap.add_argument('-d', '--device', help='device URI or serial number')
ap.add_argument('--seconds', type=float, help='stop after this long (default: run until Ctrl+C)')
ap.add_argument('--level', default='DEBUG', choices=LEVELS, help='lowest level shown')
ap.add_argument('--source', type=re.compile, help='only files matching this regex')
ap.add_argument('--grep', type=re.compile, help='only texts matching this regex')
a = ap.parse_args()


def wanted(rec: uclog.LogRecord) -> bool:
  if rec.error or rec.level not in LEVELS:
    return True
  return (LEVELS.index(rec.level) >= LEVELS.index(a.level)
          and (a.source is None or a.source.search(rec.file) is not None)
          and (a.grep is None or a.grep.search(rec.text) is not None))


with uclog.connect(a.host, a.device, logdata=a.logdata) as dev:
  try:
    for rec in dev.logs(duration=a.seconds):
      if wanted(rec):
        print(rec, flush=True)
  except KeyboardInterrupt:
    pass
