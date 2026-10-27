# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Hardware tests with the `dev` fixture (conftest.py). These hold for any ucLog device; add your
own: commands and their replies, logs a command should produce, limits on values in logs."""

import re

import pytest


def test_device_info(dev):
  info = dev.device_info
  assert info['board']
  assert len(info['app_hash']) == 64


def test_logs_decode(dev, no_errors):
  recs = list(dev.logs(duration=3.0))
  assert recs, 'no logs in 3 s'
  assert all(r.error is None for r in recs), [str(r) for r in recs if r.error]


def test_terminal_answers(dev, no_errors):
  dev.send(61, 'help\r')
  reply = dev.recv(61, timeout=2.0)
  assert reply, 'no reply to help on the terminal'


def test_values_in_logs(dev):
  # Checking what a device reports: every `temp=` value above a limit. Skipped when the device
  # logs no temperatures.
  temps = [int(m[1]) for r in dev.logs(duration=3.0) if (m := re.search(r'temp=(-?\d+)', r.text))]
  if not temps:
    pytest.skip('no temp= logs from this device')
  assert min(temps) > -40, temps
