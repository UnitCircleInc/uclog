# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Platform wheels (plan 06 §2): with UCLOG_HUB_BINARY (the uclog-hub executable) and
UCLOG_WHEEL_PLAT (a wheel platform tag such as manylinux_2_28_x86_64, macosx_11_0_arm64 or
win_amd64), the wheel bundles uclog-hub as uclog/bin/uclog-hub and is tagged py3-none-<plat>.
Without them it is the pure-Python py3-none-any wheel.
"""

import os
import shutil

from setuptools import Distribution, setup
from setuptools.command.build_py import build_py

try:
  from setuptools.command.bdist_wheel import bdist_wheel
except ImportError:  # setuptools < 70.1
  from wheel.bdist_wheel import bdist_wheel

HUB = os.environ.get('UCLOG_HUB_BINARY')
PLAT = os.environ.get('UCLOG_WHEEL_PLAT')
if HUB and not PLAT:
  raise SystemExit('UCLOG_HUB_BINARY needs UCLOG_WHEEL_PLAT (the wheel platform tag)')


class BuildPy(build_py):
  def run(self):
    super().run()
    if HUB:
      d = os.path.join(self.build_lib, 'uclog', 'bin')
      os.makedirs(d, exist_ok=True)
      name = 'uclog-hub.exe' if HUB.lower().endswith('.exe') else 'uclog-hub'
      shutil.copy2(HUB, os.path.join(d, name))
      os.chmod(os.path.join(d, name), 0o755)


class BdistWheel(bdist_wheel):
  def finalize_options(self):
    super().finalize_options()
    if HUB:
      self.root_is_pure = False

  def get_tag(self):
    if HUB:
      return 'py3', 'none', PLAT
    return super().get_tag()


class BinaryDistribution(Distribution):
  def has_ext_modules(self):
    return bool(HUB)  # platlib, at the wheel root


setup(cmdclass={'build_py': BuildPy, 'bdist_wheel': BdistWheel}, distclass=BinaryDistribution)
