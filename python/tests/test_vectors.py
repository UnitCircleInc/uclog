# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""spec/vectors against the library (the same checks as spec/reference/check_vectors.py)."""

import os

import pytest
from conftest import VECTORS, load_vectors

from uclog import cbor, proto, semihost
from uclog.logdata import DecodeError, LogData

B = bytes.fromhex


def scan_one(wire):
  ev = proto.FrameScanner().feed(wire)
  assert len(ev) == 1 and ev[0][0] == 'frame', ev
  return proto.cobs_decode(ev[0][1])


def test_cobs():
  v = load_vectors('cobs.json')
  for c in v['valid']:
    assert proto.cobs_encode(B(c['decoded'])).hex() == c['encoded'], c['name']
    assert proto.cobs_decode(B(c['encoded'])).hex() == c['decoded'], c['name']
  for c in v['invalid']:
    with pytest.raises(proto.CobsError):
      proto.cobs_decode(B(c['encoded']))


def test_scanner():
  for c in load_vectors('scanner.json')['cases']:
    sc = proto.FrameScanner(max_frame=c['max_frame'])
    ev = []
    for ch in c['chunks']:
      ev += sc.feed(B(ch))
    got = [{'frame': v.hex()} if k == 'frame' else {'oversize': v} for k, v in ev]
    assert got == c['events'], c['name']


def test_scanner_byte_by_byte():
  # Chunk boundaries anywhere give the same frames.
  for c in load_vectors('scanner.json')['cases']:
    whole = b''.join(B(ch) for ch in c['chunks'])
    sc = proto.FrameScanner(max_frame=c['max_frame'])
    ev = []
    for i in range(len(whole)):
      ev += sc.feed(whole[i:i + 1])
    got = [{'frame': v.hex()} if k == 'frame' else {'oversize': v} for k, v in ev]
    assert got == c['events'], c['name']


def test_peek_mux():
  for c in load_vectors('peek_mux.json')['cases']:
    enc = B(c['encoded'])
    pk = proto.peek(enc[:2] if len(enc) > 2 and enc != b'\x01' else enc)
    exp = c['peek']
    assert pk[0] == exp['kind'], c['name']
    if pk[0] == 'port':
      assert pk[1] == exp['port']
    if pk[0] == 'log':
      assert pk[1] == exp['type']
    m = proto.mux_decode(proto.cobs_decode(enc))
    assert m[0] == c['mux']['kind']
    if m[0] == 'port':
      assert (m[1], m[2].hex()) == (c['mux']['port'], c['mux']['data'])
      assert proto.mux_encode(m[1], m[2]) == proto.cobs_decode(enc)
    if m[0] == 'log':
      assert (m[1], m[2], m[3], m[4].hex()) == (c['mux']['type'], c['mux']['target'],
                                                c['mux']['addr'], c['mux']['args'])


def test_logs():
  v = load_vectors('logs.json')
  ld = LogData.load(os.path.join(VECTORS, v['logdata']))
  assert ld.target == v['logdata_target']
  for c in v['cases']:
    m = proto.mux_decode(scan_one(B(c['wire'])))
    assert m[0] == 'log' and m[2] == c['target'], c['name']
    level, file, line, text = ld.decode(m[3], m[4])
    assert dict(level=level, file=file, line=line, text=text) == c['expect'], c['name']
  for c in v['errors']:
    m = proto.mux_decode(scan_one(B(c['wire'])))
    with pytest.raises(DecodeError) as e:
      ld.decode(m[3], m[4])
    assert e.value.kind == c['error'], c['name']
  c = v['no_logdata_for_target']
  m = proto.mux_decode(scan_one(B(c['wire'])))
  assert m[2] == c['target'] != ld.target


def test_fw_update_cbor():
  # Port 59 messages are plain CBOR maps; our codec must read them the same way.
  v = load_vectors('fw_update.json')
  for c in v['requests'] + v['replies']:
    msg = {k: (B(x['hex']) if isinstance(x, dict) else x) for k, x in c['msg'].items()}
    m = proto.mux_decode(scan_one(B(c['port59_wire'])))
    assert m[:2] == ('port', 59) and cbor.loads(m[2]) == msg
    assert cbor.loads(cbor.dumps(msg)) == msg


def test_semihost_wire_and_errors():
  v = load_vectors('semihost.json')
  for c in v['requests']:
    m = proto.mux_decode(scan_one(B(c['wire'])))
    assert m[:2] == ('port', 63) and m[2].hex() == c['request'], c['name']
  for c in v['errors']:
    assert semihost.ERRNO[c['errno']] == c['value']
    assert semihost.error_reply(c['op'], c['errno']).hex() == c['reply'], c['name']


def test_device_info():
  v = load_vectors('device_info.json')
  for c in v['cases']:
    m = proto.mux_decode(scan_one(B(c['wire'])))
    info = cbor.loads(m[2])
    assert m[:2] == ('port', 62)
    assert info['app_hash'].hex() == c['app_hash_hex'] and info['board'] == c['board']
    assert cbor.dumps(info).hex() == c['cbor']
  assert proto.wire_port(62, b'').hex() == v['request']['wire']
