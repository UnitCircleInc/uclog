# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""A pytest fixture for hardware tests: `dev`, one connection to the device for the whole run.

    pytest examples/pytest --uclog-device serial:/dev/ttyACM0 --uclog-logdata build/app.elf
    pytest examples/pytest --uclog-device loopback:rate=20 \\
        --uclog-logdata ../spec/vectors/sample.logdata       # the simulated device

Without --uclog-device it uses $UCLOG_DEVICE, else the one local device (as `uclog.connect()`).
A hub is started for the run and stopped at the end.
"""

import os

import pytest

import uclog


def pytest_addoption(parser):
  g = parser.getgroup('uclog')
  g.addoption('--uclog-device', help='device URI or serial number (default: $UCLOG_DEVICE, else the one local device)')
  g.addoption('--uclog-host', help='a hub to use, instead of finding or starting one')
  g.addoption('--uclog-logdata', action='append', help='.logdata or ELF file (repeatable)')


@pytest.fixture(scope='session')
def dev(request):
  o = request.config.option
  device = o.uclog_device or os.environ.get('UCLOG_DEVICE')
  with uclog.connect(o.uclog_host, device, logdata=o.uclog_logdata, skip_backlog=True) as d:
    yield d


@pytest.fixture
def no_errors(dev):
  """Fails the test if the device logs an ERROR (or worse) while it runs, or a log cannot be
  decoded."""
  for _ in dev.logs(timeout=0):  # what came before the test is not its business
    pass
  yield
  bad = [r for r in dev.logs(duration=0.5) if r.error or r.level in ('ERROR', 'FATAL', 'PANIC')]
  assert not bad, '\n'.join(map(str, bad))
