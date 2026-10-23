#! /usr/bin/env python3
# © 2026 Unit Circle Inc.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Run spec/vectors/*.json against the reference implementation.

This reads the vectors the same way a Rust or TypeScript test runner should, so it doubles
as the model for those runners. Exit status 0 = all vectors pass.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ucref  # noqa: E402

VECTORS = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'vectors')
B = bytes.fromhex
failures = []
count = 0


def load(name):
  with open(os.path.join(VECTORS, name)) as f:
    return json.load(f)


def check(cond, what):
  global count
  count += 1
  if not cond:
    failures.append(what)


def scan_one(wire):
  ev = ucref.FrameScanner().feed(wire)
  assert len(ev) == 1 and ev[0][0] == 'frame', ev
  return ucref.cobs_dec(ev[0][1])


def run_cobs():
  v = load('cobs.json')
  for c in v['valid']:
    check(ucref.cobs_enc(B(c['decoded'])).hex() == c['encoded'], f"cobs enc {c['name']}")
    check(ucref.cobs_dec(B(c['encoded'])).hex() == c['decoded'], f"cobs dec {c['name']}")
  for c in v['invalid']:
    try:
      ucref.cobs_dec(B(c['encoded']))
      check(False, f"cobs invalid {c['name']}")
    except ucref.CobsError:
      check(True, '')


def run_scanner():
  for c in load('scanner.json')['cases']:
    sc = ucref.FrameScanner(max_frame=c['max_frame'])
    ev = []
    for ch in c['chunks']:
      ev += sc.feed(B(ch))
    got = [{'frame': v.hex()} if k == 'frame' else {'oversize': v} for k, v in ev]
    check(got == c['events'], f"scanner {c['name']}")


def run_peek_mux():
  for c in load('peek_mux.json')['cases']:
    enc = B(c['encoded'])
    pk = ucref.peek(enc[:2] if len(enc) > 2 and enc != b'\x01' else enc)
    exp = c['peek']
    ok = pk[0] == exp['kind'] and (pk[0] != 'port' or pk[1] == exp['port']) and \
        (pk[0] != 'log' or pk[1] == exp['type'])
    check(ok, f"peek {c['name']}")
    m = ucref.mux_dec(ucref.cobs_dec(enc))
    check(m[0] == c['mux']['kind'], f"mux {c['name']}")
    if m[0] == 'port':
      check(m[1] == c['mux']['port'] and m[2].hex() == c['mux']['data'], f"mux port {c['name']}")
    if m[0] == 'log':
      check((m[1], m[2], m[3], m[4].hex()) ==
            (c['mux']['type'], c['mux']['target'], c['mux']['addr'], c['mux']['args']),
            f"mux log {c['name']}")


def run_logs():
  v = load('logs.json')
  with open(os.path.join(VECTORS, v['logdata']), 'rb') as f:
    ld = ucref.LogData(f.read())
  check(ld.target == v['logdata_target'], 'logdata target')
  for c in v['cases']:
    m = ucref.mux_dec(scan_one(B(c['wire'])))
    check(m[0] == 'log' and m[2] == c['target'], f"log mux {c['name']}")
    check(ucref.decode_log(ld, m[3], m[4]) == c['expect'], f"log {c['name']}")
  for c in v['errors']:
    m = ucref.mux_dec(scan_one(B(c['wire'])))
    try:
      ucref.decode_log(ld, m[3], m[4])
      check(False, f"log error {c['name']}")
    except ucref.DecodeError:
      check(True, '')
  c = v['no_logdata_for_target']
  m = ucref.mux_dec(scan_one(B(c['wire'])))
  check(m[2] == c['target'] != ld.target, 'no logdata for target')


def unjson(m):
  return {k: (B(x['hex']) if isinstance(x, dict) else x) for k, x in m.items()}


def run_fw():
  v = load('fw_update.json')
  check(ucref.crc32c(B(v['crc32c_check']['input'])) == v['crc32c_check']['crc'], 'crc32c')
  for c in v['requests'] + v['replies']:
    msg = unjson(c['msg'])
    m = ucref.mux_dec(scan_one(B(c['port59_wire'])))
    check(m[:2] == ('port', 59) and ucref.cbor_loads(m[2]) == msg, f"fw59 {msg}")
    m = ucref.mux_dec(scan_one(B(c['port3_wire'])))
    inner = scan_one(m[2])
    body, crc = inner[:-4], int.from_bytes(inner[-4:], 'little')
    check(m[:2] == ('port', 3) and crc == ucref.crc32c(body) and ucref.cbor_loads(body) == msg,
          f"fw3 {msg}")


def run_semihost():
  v = load('semihost.json')
  for c in v['requests']:
    m = ucref.mux_dec(scan_one(B(c['wire'])))
    check(m[:2] == ('port', 63) and m[2].hex() == c['request'], f"sh wire {c['name']}")
    op, name, f = ucref.sh_parse_request(B(c['request']))
    f = {k: (x.hex() if isinstance(x, bytes) else x) for k, x in f.items()}
    check((op, name, f) == (c['op'], c['op_name'], c['fields']), f"sh req {c['name']}")
    check((op in ucref.TIME_OPS) == c['time_op'], f"sh time_op {c['name']}")
  for c in v['time_replies']:
    check(ucref.sh_reply_time(c['op'], **c['inputs']).hex() == c['reply'],
          f"sh time {c['source']} {c['op_name']}")
  for c in v['errors']:
    check(ucref.NEWLIB_ERRNO[c['errno']] == c['value'] and ucref.sh_error(c['errno'], c['op']).hex() == c['reply'],
          f"sh err {c['name']}")


def run_links():
  v = load('links.json')
  for c in v['hid_out']:
    check([r.hex() for r in ucref.hid_reports(B(c['data']))] == c['reports'], f"hid {c['name']}")
  for c in v['stlink']:
    check([w.hex() for w in ucref.stlink_split(B(c['data']))] == c['writes'], f"stlink {c['name']}")
  e = v['hid_in_example']
  check(B(e['report'])[0] == ucref.HID_IN and B(e['report'])[1:].hex() == e['scanner_bytes'], 'hid in')


def run_device_info():
  for c in load('device_info.json')['cases']:
    m = ucref.mux_dec(scan_one(B(c['wire'])))
    info = ucref.cbor_loads(m[2])
    check(m[:2] == ('port', 62) and info['app_hash'].hex() == c['app_hash_hex'] and
          len(info['app_hash']) == 64, f"device info {c['name']}")
  m = ucref.mux_dec(scan_one(B(load('device_info.json')['request']['wire'])))
  check(m[:2] == ('port', 62) and m[2] == b'', 'device info request')


for fn in (run_cobs, run_scanner, run_peek_mux, run_logs, run_fw, run_semihost, run_links,
           run_device_info):
  fn()

if failures:
  print(f'{len(failures)} of {count} checks failed:', file=sys.stderr)
  for f in failures:
    print('  ' + f, file=sys.stderr)
  sys.exit(1)
print(f'all {count} checks passed')
