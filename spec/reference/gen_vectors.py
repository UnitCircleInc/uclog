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

"""Generate spec/vectors/ from the reference implementation.

  python3 spec/reference/gen_vectors.py          # (re)write spec/vectors/
  python3 spec/reference/gen_vectors.py --check  # fail if spec/vectors/ is out of date

All byte strings in the JSON files are lowercase hex. See spec/vectors/README.md.
"""

import argparse
import json
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ucref  # noqa: E402

VECTORS = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'vectors')


def h(b):
  return bytes(b).hex()


# ---------------------------------------------------------------------------
# COBS
# ---------------------------------------------------------------------------

def gen_cobs():
  r = lambda a, b: bytes(range(a, b))  # noqa: E731
  cases = [
    ('paper example', bytes.fromhex('4500002C4C79000040064F37')),
    ('empty', b''),
    ('single zero', b'\x00'),
    ('ends with zero', b'123\x00'),
    ('no zero', b'123'),
    ('one byte', b'1'),
    ('zero in middle', b'123\x00456'),
    ('only zeros', b'\x00' * 10),
    ('leading zero, 254 run, zero, tail', b'\x00' + b'1' * 254 + b'\x00123456'),
    ('1444 bytes no zero (max payload, encodes to 1450)', (b'0123456789' * 145)[:1444]),
    ('253 no zero', r(1, 254)),
    ('253 then zero', r(1, 254) + b'\x00'),
    ('254 no zero (no trailing 01)', r(1, 255)),
    ('254 then zero', r(1, 255) + b'\x00'),
    ('255 no zero', r(1, 256)),
    ('255 then zero', r(1, 256) + b'\x00'),
    ('508 no zero', (r(1, 255)) * 2),
  ]
  valid = []
  for name, dec in cases:
    enc = ucref.cobs_enc(dec)
    assert ucref.cobs_dec(enc) == dec, name
    assert 0 not in enc
    valid.append({'name': name, 'decoded': h(dec), 'encoded': h(enc)})
  invalid = [
    {'name': 'code runs past end', 'encoded': '05414243'},
    {'name': 'zero code byte', 'encoded': '024100'},
    {'name': 'empty input (not a frame; a pulse at the scanner level)', 'encoded': ''},
  ]
  for c in invalid:
    try:
      ucref.cobs_dec(bytes.fromhex(c['encoded']))
    except ucref.CobsError:
      continue
    raise AssertionError(c['name'])
  return {'description': 'COBS encode/decode (protocol.md §2). encode(decoded) must equal '
                         'encoded exactly; decode(encoded) must equal decoded. invalid: decode must fail.',
          'valid': valid, 'invalid': invalid}


# ---------------------------------------------------------------------------
# Frame scanner
# ---------------------------------------------------------------------------

def gen_scanner():
  f1 = ucref.wire(ucref.mux_enc(61, b'hi'))
  f2 = ucref.wire(ucref.mux_enc(62, ucref.cbor_dumps({'board': 'x'})))
  fmax = ucref.wire(ucref.mux_enc(61, bytes(range(1, 256)) * 5 + b'x' * (ucref.MAX_PORT_DATA - 1275)))
  assert len(fmax) == ucref.MAX_WIRE, len(fmax)
  cases = [
    ('one frame', [f1]),
    ('two frames back to back', [f1 + f2]),
    ('frame split across reads', [f1[:3], f1[3:]]),
    ('frame split one byte at a time', [bytes((b,)) for b in f1]),
    ('lone pulses only', [b'\x00', b'\x00\x00']),
    ('empty-payload pulse 00 01 00', [b'\x00\x01\x00']),
    ('garbage before first delimiter is dropped', [b'\x41\x42' + f1]),
    ('oversize run is discarded, next frame survives',
     [b'\x00' + b'\x01' * 5000 + b'\x00', f1]),
    ('largest frame: 1450-byte run, 1452 on the wire', [fmax]),
    ('1451-byte run is oversize', [b'\x00' + b'\x01' * 1451 + b'\x00', f1]),
  ]
  out = []
  for name, chunks in cases:
    sc = ucref.FrameScanner(max_frame=ucref.MAX_FRAME)
    ev = []
    for c in chunks:
      ev += sc.feed(c)
    out.append({'name': name, 'max_frame': ucref.MAX_FRAME, 'chunks': [h(c) for c in chunks],
                'events': [{'frame': h(v)} if k == 'frame' else {'oversize': v} for k, v in ev]})
  return {'description': 'Frame scanner (protocol.md §2.2). Feed chunks in order; the emitted '
                         'events must match. frame = COBS-encoded bytes between delimiters. '
                         'oversize = number of discarded bytes.',
          'cases': out}


# ---------------------------------------------------------------------------
# Peek + mux
# ---------------------------------------------------------------------------

def gen_peek_mux():
  payloads = [
    ('pulse (empty payload)', b''),
    ('type-0 log, addr low byte 00', struct.pack('<I', 0xF0100000) + b'\x01\x00\x00\x00'),
    ('type-0 log, addr low byte 04', struct.pack('<I', 0xF0100004)),
    ('type-1 mem log', struct.pack('<I', 0xF0100049) + struct.pack('<I', 0x20000000) + b'\xde\xad'),
    ('type-2 reserved', struct.pack('<I', 0xF010000A)),
    ('port 0 text command', ucref.mux_enc(0, b'help\r')),
    ('port 0 CBOR command', ucref.mux_enc(0, ucref.cbor_dumps({'cmd': 'get-info'}))),
    ('port 3 (legacy fw)', ucref.mux_enc(3, b'\x00\x01\x00')),
    ('port 59 (fw)', ucref.mux_enc(59, ucref.cbor_dumps({'cmd': 'set-time', 'timestamp': 0}))),
    ('port 60 plot', ucref.mux_enc(60, ucref.cbor_dumps({'plot': 'p', 'x': [1], 'y11': [2]}))),
    ('port 61 terminal', ucref.mux_enc(61, b'\x1b[32mok\x1b[0m\r\n')),
    ('port 62 device info', ucref.mux_enc(62, ucref.cbor_dumps({'board': 'b'}))),
    ('port 63 syscall', ucref.mux_enc(63, b'\x10')),
    ('port 63, byte after header is zero', ucref.mux_enc(63, b'\x00\x00')),
    ('port frame with empty body', ucref.mux_enc(61, b'')),
    ('log frame too short', b'\x00\x01'),
  ]
  out = []
  for name, p in payloads:
    enc = ucref.cobs_enc(p)
    pk = ucref.peek(enc)
    m = ucref.mux_dec(p)
    # The peek must agree with a full decode.
    if m[0] == 'port':
      assert pk == ('port', m[1]), name
    elif m[0] == 'pulse':
      assert pk == ('pulse',), name
    else:
      assert pk[0] == 'log', name
    rec = {'name': name, 'payload': h(p), 'encoded': h(enc),
           'peek': {'kind': pk[0]}}
    if pk[0] == 'port':
      rec['peek']['port'] = pk[1]
    elif pk[0] == 'log':
      rec['peek']['type'] = pk[1]
    if m[0] == 'port':
      rec['mux'] = {'kind': 'port', 'port': m[1], 'data': h(m[2])}
    elif m[0] == 'log':
      rec['mux'] = {'kind': 'log', 'type': m[1], 'target': m[2], 'addr': m[3], 'args': h(m[4])}
    elif m[0] == 'pulse':
      rec['mux'] = {'kind': 'pulse'}
    else:
      rec['mux'] = {'kind': 'error'}
    out.append(rec)
  return {'description': 'Two-byte peek on the encoded frame and full mux decode of the payload '
                         '(protocol.md §3). peek must only read encoded[0..2].',
          'cases': out}


# ---------------------------------------------------------------------------
# Logdata + log decode
# ---------------------------------------------------------------------------

SADDR = 0xF0100000


def sample_logdata():
  fn = 'src/main.c'
  fmts = {
    SADDR + 0x00: ['1', fn, '10', 'Hello World! %s', ['string']],
    SADDR + 0x40: ['2', fn, '11', 'temp=%d mV=%u flags=0x%08x', ['int32', 'uint32', 'uint32']],
    SADDR + 0x80: ['0', fn, '12', 'big=%ld ubig=%lu hex=0x%012lx', ['int64', 'uint64', 'uint64']],
    SADDR + 0xC0: ['3', fn, '13', 'reason: %s', [['enum', 'k_fatal_error_reason']]],
    SADDR + 0x100: ['1', fn, '14', 'cb=%s ptr=0x%08x', ['sym', 'pointer']],
    SADDR + 0x140: ['1', fn, '15', 'v=%.3f e=%e %c%%', ['double', 'double', 'uint32']],
    SADDR + 0x180: ['1', fn, '16', 'mode %s', [['enum', 'mode_t']]],
    SADDR + 0x1C0: ['1', fn, '17', 'buffer', []],
    SADDR + 0x200: ['1', 'zephyr', '<zephyr>', '', []],
    SADDR + 0x240: ['4', fn, '19', '|%5d|%-5d|%05d|%+d|', ['int32', 'int32', 'int32', 'int32']],
    SADDR + 0x280: ['1', fn, '20', 'no args', []],
    SADDR + 0x2C0: ['1', fn, '21', 'path "%s"', ['string']],
  }
  data = {
    'enums': {'k_fatal_error_reason': {0: 'K_ERR_CPU_EXCEPTION', 4: 'K_ERR_KERNEL_PANIC'}},
    'tdenums': {'mode_t': {1: 'MODE_FAST', 2: 'MODE_SLOW'}},
    'vars': {0x20000100: 'src/main.c:counter'},
    'fns': {(0x08001000, 0x40): 'main', (0x08001040, 0x20): 'on_timer'},
    'saddr': SADDR,
    'fmts': fmts,
  }
  return ucref.cbor_dumps(data)


def gen_logs():
  raw = sample_logdata()
  ld = ucref.LogData(raw)
  P = struct.pack
  cases = [
    ('string arg', SADDR + 0x00, b'there\x00'),
    ('int32/uint32/hex', SADDR + 0x40, P('<iII', -12, 3300, 0xBEEF)),
    ('64-bit args; %lu carries uint64', SADDR + 0x80, P('<qQQ', -(2**40), 2**63, 0xABCDEF012345)),
    ('enum known', SADDR + 0xC0, P('<i', 4)),
    ('enum unknown value', SADDR + 0xC0, P('<i', 9)),
    ('typedef enum', SADDR + 0x180, P('<i', 2)),
    ('sym in function (thumb bit cleared)', SADDR + 0x100, P('<II', 0x08001045, 0x20000000)),
    ('sym in variable range', SADDR + 0x100, P('<II', 0x20000104, 0)),
    ('sym unresolved', SADDR + 0x100, P('<II', 0x30000000, 1)),
    ('double, %e, %c and %%', SADDR + 0x140, P('<ddI', 3.14159, 12345.678, ord('A'))),
    ('mem dump (type 1)', SADDR + 0x1C0 + 1, P('<I', 0x20000010) + bytes(range(8))),
    ('zephyr text via type 1', SADDR + 0x200 + 1, P('<I', 0) + b'zephyr says hi'),
    ('width/flags', SADDR + 0x240, P('<iiii', 42, 42, 42, 42)),
    ('no args', SADDR + 0x280, b''),
    ('string escaping: quote kept, backslash/newline/control escaped, UTF-8 kept',
     SADDR + 0x2C0, 'a"b\\\nc\x01 \u00e9'.encode() + b'\x00'),
    ('invalid UTF-8 becomes U+FFFD', SADDR + 0x2C0, b'x\xffy\x00'),
  ]
  errors = [
    ('missing arg', SADDR + 0x40, P('<i', 1), 'missing_arg'),
    ('extra bytes', SADDR + 0x280, b'\x01\x02', 'extra_bytes'),
    ('unknown format address', SADDR + 0x3C0, b'', 'unknown_format'),
    ('missing string terminator', SADDR + 0x00, b'abc', 'missing_arg'),
  ]
  ok = []
  for name, addr, args in cases:
    payload = P('<I', addr) + args
    rec = ucref.decode_log(ld, addr, args)
    ok.append({'name': name, 'wire': h(ucref.wire(payload)), 'target': ld.target,
               'expect': rec})
  bad = []
  for name, addr, args, kind in errors:
    payload = P('<I', addr) + args
    try:
      ucref.decode_log(ld, addr, args)
      raise AssertionError(name)
    except ucref.DecodeError:
      pass
    bad.append({'name': name, 'wire': h(ucref.wire(payload)), 'error': kind})
  other_target = P('<I', 0xF0200000) + b''
  return raw, {
    'description': 'Log decoding with sample.logdata (protocol.md §6). For each case, scan the '
                   'wire bytes, decode the frame, and compare level/file/line/text. Text must '
                   'match exactly for these cases; other formatting is C-printf best effort. '
                   'errors: decoding must fail with an error record of the given kind (§7).',
    'logdata': 'sample.logdata',
    'logdata_target': ld.target,
    'cases': ok,
    'errors': bad,
    'no_logdata_for_target': {'wire': h(ucref.wire(other_target)), 'target': 2,
                              'error': 'no_logdata'},
  }


# ---------------------------------------------------------------------------
# FW update (ports 59 and 3)
# ---------------------------------------------------------------------------

def gen_fw():
  msgs = [
    {'cmd': 'set-time', 'timestamp': 1767225600},
    {'cmd': 'fw-start'},
    {'cmd': 'fw-block', 'data': bytes(range(16))},
    {'cmd': 'fw-schedule', 'timestamp': 1767225605},
  ]
  replies = [
    {'rsp': 'set-time'},
    {'rsp': 'fw-start', 'block-size': 512},
    {'rsp': 'fw-block'},
    {'rsp': 'fw-schedule'},
    {'rsp': 'fw-schedule', 'error': 'image verification failed'},
    {'event': 'progress'},
  ]

  def p59(m):
    return ucref.wire(ucref.mux_enc(59, ucref.cbor_dumps(m)))

  def p3(m):
    c = ucref.cbor_dumps(m)
    inner = b'\x00' + ucref.cobs_enc(c + struct.pack('<I', ucref.crc32c(c))) + b'\x00'
    return ucref.wire(ucref.mux_enc(3, inner))

  def jsonable(m):
    return {k: ({'hex': h(v)} if isinstance(v, bytes) else v) for k, v in m.items()}

  assert ucref.crc32c(b'123456789') == 0xE3069283
  return {
    'description': 'FW update messages (protocol.md §5.4). port59 = mux(59, cbor). '
                   'port3 = mux(3, 00 cobs(cbor || crc32c_le(cbor)) 00). CBOR here uses '
                   'insertion-order keys and shortest integers; decoders must accept any '
                   'valid CBOR, so compare decoded values, not bytes, for received messages. '
                   'Byte fields are shown as {"hex": ...}.',
    'crc32c_check': {'input': h(b'123456789'), 'crc': 0xE3069283},
    'requests': [{'msg': jsonable(m), 'cbor': h(ucref.cbor_dumps(m)), 'port59_wire': h(p59(m)),
                  'port3_wire': h(p3(m))} for m in msgs],
    'replies': [{'msg': jsonable(m), 'cbor': h(ucref.cbor_dumps(m)), 'port59_wire': h(p59(m)),
                 'port3_wire': h(p3(m)),
                 'ignored': 'event' in m} for m in replies],
  }


# ---------------------------------------------------------------------------
# Semihost (port 63)
# ---------------------------------------------------------------------------

def gen_semihost():
  P = struct.pack
  reqs = [
    ('OPEN', P('<Bi', 8, 0x601) + b'out/log.txt'),
    ('WRITE needs reply', P('<BI', 9, 3) + b'abc'),
    ('WRITE no reply (bit 31)', P('<BI', 9, 0x80000003) + b'abc'),
    ('WRITE fid above 27 bits keeps all 31 bits', P('<BI', 9, 0x08000005) + b'x'),
    ('READ', P('<BiI', 11, 3, 64)),
    ('CLOSE', P('<Bi', 7, 3)),
    ('ISATTY', P('<Bi', 2, 1)),
    ('FSTAT', P('<Bi', 6, 3)),
    ('MKDIR', P('<Bi', 12, 0o755) + b'out'),
    ('TIMEOFDAY', P('<B', 13)),
    ('SYS_CLOCK', P('<B', 0x10)),
    ('SYS_TIME', P('<B', 0x11)),
    ('SYS_ELAPSED', P('<B', 0x30)),
    ('SYS_TICKFREQ', P('<B', 0x31)),
    ('unknown op 0x20', P('<B', 0x20) + b'\x01\x02'),
  ]
  rq = []
  for name, frame in reqs:
    op, opname, f = ucref.sh_parse_request(frame)
    f = {k: (h(v) if isinstance(v, bytes) else v) for k, v in f.items()}
    rq.append({'name': name, 'request': h(frame), 'wire': h(ucref.wire(ucref.mux_enc(63, frame))),
               'op': op, 'op_name': opname, 'fields': f, 'time_op': op in ucref.TIME_OPS})

  posix = {'mono_ns': 123_456_789_012_345, 'wall': 1_767_225_600.25}
  win = {'qpc': 98_765_432_101, 'qpf': 10_000_000, 'wall': 4_102_444_800.999_999_7}
  time_replies = []
  for src_name, src in (('posix CLOCK_MONOTONIC ns', posix), ('windows QPC', win)):
    for op in (0x10, 0x11, 0x30, 0x31, 13):
      time_replies.append({'source': src_name, 'inputs': src, 'op': op,
                           'op_name': ucref.SH_OPS[op],
                           'reply': h(ucref.sh_reply_time(op, **src))})
  wrap = {'mono_ns': (2**32 + 5) * 10_000_000, 'wall': 0.0}
  time_replies.append({'source': 'SYS_CLOCK wraps at 2^32 centiseconds', 'inputs': wrap,
                       'op': 0x10, 'op_name': 'SYS_CLOCK',
                       'reply': h(ucref.sh_reply_time(0x10, **wrap))})

  errors = [{'name': n, 'op': op, 'errno': e, 'value': ucref.NEWLIB_ERRNO[e],
             'reply': h(ucref.sh_error(e, op))}
            for n, op, e in (('unknown op', 0x20, 'ENOSYS'),
                             ('OPEN path outside sandbox', 8, 'EACCES'),
                             ('client went away with READ outstanding', 11, 'EIO'),
                             ('CLOSE bad fid', 7, 'EBADF'),
                             ('OPEN no such file', 8, 'ENOENT'),
                             ('FSTAT error is zero padded to 48 bytes', 6, 'EBADF'),
                             ('malformed request', 7, 'EINVAL'))]
  return {
    'description': 'Port 63 semihost codec (protocol.md §5.5). requests: parse and compare '
                   'op/fields. time_replies: given the clock inputs, the reply bytes must match. '
                   'errors: <i32 -1, i32 newlib errno>, zero padded to 48 bytes for FSTAT.',
    'newlib_errno': ucref.NEWLIB_ERRNO,
    'requests': rq,
    'time_replies': time_replies,
    'errors': errors,
  }


# ---------------------------------------------------------------------------
# Link specifics
# ---------------------------------------------------------------------------

def gen_links():
  frame = ucref.wire(ucref.mux_enc(61, b'x' * 100))
  pulse = b'\x00'
  hid = [{'name': n, 'data': h(d), 'reports': [h(r) for r in ucref.hid_reports(d)]}
         for n, d in (('pulse', pulse), ('100-byte payload frame', frame),
                      ('exactly 63 bytes', bytes(range(1, 64))),
                      ('64 bytes', bytes(range(1, 65))))]
  st = [{'name': n, 'data': h(d), 'writes': [h(w) for w in ucref.stlink_split(d)]}
        for n, d in (('len 8', bytes(range(1, 9))), ('len 7', bytes(range(1, 8))),
                     ('len 16', bytes(range(1, 17))), ('len 1', b'\x00'))]
  hid_in = bytes((ucref.HID_IN,)) + frame[:63]
  return {
    'description': 'Link chunking (protocol.md §4). hid_out: host->device output reports '
                   '(report id 0xA2, 63-byte payload, zero padded). hid_in: strip the 0xA1 '
                   'report id and feed the 63 bytes to the scanner (trailing zero padding is '
                   'harmless). stlink: split writes when len % 8 == 0 (pause >= 10 ms between).',
    'hid_out': hid,
    'hid_in_example': {'report': h(hid_in), 'scanner_bytes': h(hid_in[1:])},
    'stlink': st,
  }


def gen_device_info():
  info = {'board': 'nrf5340dk', 'image_type': 'app', 'version': '1.2.3',
          'app_hash': bytes(range(64))}
  c = ucref.cbor_dumps(info)
  return {
    'description': 'Port 62 device info (protocol.md §5.2). app_hash is 64 bytes (SHA-512 of '
                   'the signed image after its 512-byte header). Other keys are informational.',
    'cases': [{'name': 'typical', 'cbor': h(c), 'wire': h(ucref.wire(ucref.mux_enc(62, c))),
               'app_hash_hex': h(info['app_hash']), 'board': info['board'],
               'image_type': info['image_type']}],
    # F7: the host's request is an empty port-62 frame.
    'request': {'description': 'host -> device: an empty port-62 frame asks for the device info',
                'wire': h(ucref.wire(ucref.mux_enc(62, b'')))},
  }


def build():
  raw_logdata, logs = gen_logs()
  files = {
    'cobs.json': gen_cobs(),
    'scanner.json': gen_scanner(),
    'peek_mux.json': gen_peek_mux(),
    'logs.json': logs,
    'fw_update.json': gen_fw(),
    'semihost.json': gen_semihost(),
    'links.json': gen_links(),
    'device_info.json': gen_device_info(),
  }
  out = {k: (json.dumps(v, indent=1, sort_keys=False) + '\n').encode() for k, v in files.items()}
  out['sample.logdata'] = raw_logdata
  return out


def main():
  ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
  ap.add_argument('--check', action='store_true', help='verify spec/vectors/ is up to date')
  args = ap.parse_args()
  out = build()
  stale = []
  for name, data in out.items():
    path = os.path.join(VECTORS, name)
    if args.check:
      try:
        with open(path, 'rb') as f:
          if f.read() != data:
            stale.append(name)
      except FileNotFoundError:
        stale.append(name)
    else:
      os.makedirs(VECTORS, exist_ok=True)
      with open(path, 'wb') as f:
        f.write(data)
  if stale:
    print('out of date:', ', '.join(stale), file=sys.stderr)
    sys.exit(1)
  print(('checked ' if args.check else 'wrote ') + f'{len(out)} files in {os.path.normpath(VECTORS)}')


if __name__ == '__main__':
  main()
