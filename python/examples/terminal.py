#! /usr/bin/env python3
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""A line-mode terminal on port 61 (or 0), on every OS (replaces `logcmd.py`).

Type a line and press Enter to send it; device output is printed as it arrives. Ctrl+D (Ctrl+Z
then Enter on Windows) or Ctrl+C quits.
"""

import argparse
import sys
import threading
import time

import uclog

ap = argparse.ArgumentParser(description=__doc__)
ap.add_argument('--host', help='hub address (default: find or start one)')
ap.add_argument('-d', '--device', help='device URI or serial number')
ap.add_argument('--port', type=int, default=61, help='61 = terminal (default), 0 = commands')
a = ap.parse_args()

with uclog.connect(a.host, a.device) as dev:
  def show():
    try:
      while True:
        d = dev.recv(a.port)
        sys.stdout.write(d.decode('utf-8', 'replace'))
        sys.stdout.flush()
    except uclog.ConnectionClosedError:
      pass

  threading.Thread(target=show, daemon=True).start()
  try:
    for line in sys.stdin:
      dev.send(a.port, line.rstrip('\r\n') + '\r')
    time.sleep(1.0)  # end of input (e.g. piped commands): show the last replies
  except KeyboardInterrupt:
    pass
