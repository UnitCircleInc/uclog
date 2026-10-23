# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

import json
import os
import sys

import pytest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
VECTORS = os.path.join(ROOT, 'spec', 'vectors')
sys.path.insert(0, os.path.join(HERE, '..', 'src'))


def load_vectors(name):
  with open(os.path.join(VECTORS, name)) as f:
    return json.load(f)


def hub_executable():
  """$UCLOG_HUB, else the workspace build."""
  if os.environ.get('UCLOG_HUB'):
    return os.environ['UCLOG_HUB']
  exe = 'uclog-hub.exe' if sys.platform == 'win32' else 'uclog-hub'
  for profile in ('release', 'debug'):
    p = os.path.join(ROOT, 'native', 'target', profile, exe)
    if os.path.isfile(p):
      return p
  return None


@pytest.fixture
def hub_exe(monkeypatch):
  exe = hub_executable()
  if exe is None:
    pytest.skip('no uclog-hub (build native/ or set UCLOG_HUB)')
  monkeypatch.setenv('UCLOG_HUB', exe)
  return exe


@pytest.fixture
def sample_logdata():
  return os.path.join(VECTORS, 'sample.logdata')
