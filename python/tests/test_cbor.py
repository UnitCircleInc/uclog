# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Malformed CBOR and logdata entries fail with the library's own errors only."""

import pytest

from uclog import cbor
from uclog.logdata import DecodeError, LogData


@pytest.mark.parametrize('data', [
  b'', b'\x18', b'\x62a', b'\x82\x01', b'\xbf\x01', b'\x9f', b'\x5f', b'\x5f\x41',
  b'\xfa\x00', b'\xfb\x00\x00',
  b'\xa1\xa0\x01',            # map key is a map
  b'\xa1\x81\xa0\x01',        # map key is an array holding a map
  b'\x5f\x61a\xff',           # text chunk inside indefinite bytes
  b'\x7f\x41a\xff',           # bytes chunk inside indefinite text
  b'\x5f\x5f\xff\xff',        # nested indefinite chunk
  b'\x81' * 100 + b'\x00',    # nested too deep
  b'\xc1' * 100 + b'\x00',    # tags nested too deep
  b'\x62\xff\xfe',            # invalid UTF-8
  b'\x1c', b'\xf8\x00', b'\x01\x02',
])
def test_malformed_cbor_raises_cbor_error(data):
  with pytest.raises(cbor.CborError):
    cbor.loads(data)


def test_depth_limit_allows_normal_nesting():
  assert cbor.loads(b'\x81' * 60 + b'\x00') is not None
  assert cbor.loads(b'\x5f\x41a\x42bc\xff') == b'abc'
  assert cbor.loads(b'\xa1\x82\x01\x02\x03') == {(1, 2): 3}


def _ld(kinds, fmt='%s'):
  return LogData(cbor.dumps({'saddr': 0x00100000, 'enums': {'e': 5}, 'tdenums': {}, 'fns': {},
                             'vars': {}, 'fmts': {0x00100000: ['1', 'a.c', '1', fmt, kinds]}}))


@pytest.mark.parametrize('kinds,fmt', [
  ([['enum']], '%s'), ([['enum', 'x', 'y']], '%s'), ([['enum', 3]], '%s'), ([5], '%s'),
  (5, '%s'), ('int32', '%s'), ([['enum', 'e']], '%s'),  # enums['e'] is not a map
  (['int32'], 7),
])
def test_malformed_kinds_raise_decode_error(kinds, fmt):
  with pytest.raises(DecodeError):
    _ld(kinds, fmt).decode(0x00100000, b'\x01\x00\x00\x00')


def test_bytes_kind_and_hex_float():
  ld = _ld(['uint32', 'bytes'], 'id=%u data=%s %a')
  with pytest.raises(DecodeError):  # one conversion too many
    ld.decode(0x00100000, b'\x07\x00\x00\x00\x01\xab')
  from uclog.logdata import format_c
  assert format_c('%a %a %A', [1.0, 0.0, 1.5]) == '0x1p+0 0x0p+0 0X1.8P+0'
  assert format_c('%a', [3.14159]) == '0x1.921f9f01b866ep+1'
