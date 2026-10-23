#! /usr/bin/env python3
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Send one CBOR command on port 0 and print the reply (replaces `StreamClient.txrx`).

    python command.py get-info
    python command.py set-led '{"on": true}'
"""

import argparse
import json
import sys

import uclog

ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument('cmd')
ap.add_argument('args', nargs='?', default='{}', help='JSON object with the arguments')
ap.add_argument('--host')
ap.add_argument('-d', '--device')
ap.add_argument('--timeout', type=float, default=2.0)
a = ap.parse_args()

with uclog.connect(a.host, a.device) as dev:
  rsp = dev.command({'cmd': a.cmd, **json.loads(a.args)}, timeout=a.timeout)
if rsp is None:
  sys.exit(f'no reply to {a.cmd!r} within {a.timeout} s')
print(json.dumps(rsp, indent=2, default=lambda b: b.hex() if isinstance(b, bytes) else repr(b)))
