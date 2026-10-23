#! /usr/bin/env python3
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Print decoded logs for a while (replaces `uclog-cli.py`).

    python capture_logs.py -l build/zephyr/zephyr.elf --seconds 10
"""

import argparse

import uclog

ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument('-l', '--logdata', action='append', help='.logdata or ELF file (repeatable)')
ap.add_argument('--host', help='hub address (default: find or start one)')
ap.add_argument('-d', '--device', help='device URI or serial number')
ap.add_argument('--seconds', type=float, help='stop after this long (default: run until Ctrl+C)')
ap.add_argument('--level', default='DEBUG', choices=uclog.logdata.LEVELS)
a = ap.parse_args()

min_level = uclog.logdata.LEVELS.index(a.level)
with uclog.connect(a.host, a.device, logdata=a.logdata) as dev:
  try:
    for rec in dev.logs(duration=a.seconds):
      if rec.error or rec.level not in uclog.logdata.LEVELS or \
          uclog.logdata.LEVELS.index(rec.level) >= min_level:
        print(rec, flush=True)
  except KeyboardInterrupt:
    pass
