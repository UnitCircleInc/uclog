#! /usr/bin/env python3
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Decode a raw capture of what a device sent (no device or hub needed), with its logdata.

    python decode_capture.py -l build/app.elf capture.bin
    cat /dev/ttyACM0 > capture.bin       # a capture: the bytes as they came off the link
    python decode_capture.py -l build/app.elf - < capture.bin

Logs are printed as decoded; terminal text (port 61) is printed as it is; other ports are summed
up at the end. `uclog decode` does the same from the command line.
"""

import argparse
import collections
import sys

import uclog
from uclog import proto

ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument('capture', help="the captured bytes, or - for stdin")
ap.add_argument('-l', '--logdata', action='append', required=True, help='.logdata or ELF file (repeatable)')
a = ap.parse_args()

# Logdata by target digit: a bootloader and an app can log on the same link.
decoders = {ld.target: ld for ld in map(uclog.LogData.load, a.logdata)}
data = sys.stdin.buffer.read() if a.capture == '-' else open(a.capture, 'rb').read()

ports = collections.Counter()
for event in proto.FrameScanner().feed(data):
  if event[0] != 'frame':
    print(f'<frame of {event[1]} bytes dropped: too long>')
    continue
  try:
    payload = proto.cobs_decode(event[1])
  except proto.CobsError:
    print(f'<invalid frame: {event[1][:16].hex(" ")}…>')
    continue
  m = proto.mux_decode(payload)
  if m[0] == 'log':
    _, _, target, addr, args = m
    ld = decoders.get(target)
    if ld is None:
      print(f'<no logdata for target {target}>')
      continue
    try:
      level, file, line, text = ld.decode(addr, args)
      print(f'{level}:{file}:{line}:{text}')
    except uclog.DecodeError as e:
      print(f'<{e.kind}: {e}>')
  elif m[0] == 'port' and m[1] == proto.PORT_TERMINAL:
    sys.stdout.write(m[2].decode('utf-8', 'replace'))
  elif m[0] == 'port':
    ports[m[1]] += 1

for port, n in sorted(ports.items()):
  print(f'port {port}: {n} frames')
