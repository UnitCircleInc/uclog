# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

# python3 -I -m unittest discover -s scripts

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import releases  # noqa: E402

ENV = {**os.environ, 'GIT_AUTHOR_NAME': 't', 'GIT_AUTHOR_EMAIL': 't@x', 'GIT_COMMITTER_NAME': 't',
       'GIT_COMMITTER_EMAIL': 't@x'}


class Releases(unittest.TestCase):
  """Against a scratch repository: v1 (the old device repository's), v4-alpha1, then v4."""

  def setUp(self):
    self.tmp = tempfile.TemporaryDirectory()
    self.root = Path(self.tmp.name)
    self.old_root = releases.ROOT
    releases.ROOT = self.root
    self.git('init', '-q')
    self.commit('legacy device', device='3', proto='1')
    self.git('tag', 'v1')
    self.commit('first monorepo commit', device='4-alpha1', proto='1')
    self.git('tag', 'v4-alpha1')
    self.commit('add a thing', device='4', proto='1')
    self.commit('fix a thing', device='4', proto='1')
    self.git('tag', 'v4')

  def tearDown(self):
    releases.ROOT = self.old_root
    self.tmp.cleanup()

  def git(self, *args):
    subprocess.run(['git', '-C', str(self.root), *args], check=True, capture_output=True, env=ENV)

  def commit(self, subject, device, proto):
    (self.root / 'device/include').mkdir(parents=True, exist_ok=True)
    (self.root / 'spec').mkdir(exist_ok=True)
    (self.root / 'device/include/log.h').write_text(f'#define UCLOG_DEVICE_VERSION "{device}"\n')
    (self.root / 'spec/protocol.md').write_text(f'# ucLog wire protocol — version {proto}\n\n{subject}\n')
    self.git('add', '-A')
    self.git('commit', '-q', '-m', subject)

  def test_host_tags_skip_the_old_device_tags(self):
    self.assertEqual(releases.host_tags(), ['v4', 'v4-alpha1'])
    self.assertEqual(releases.host_tags(merged='v4-alpha1'), ['v4-alpha1'])

  def test_notes(self):
    n = releases.notes('v4')
    self.assertIn('`device-v4`', n)
    self.assertIn('## Changes since v4-alpha1', n)
    self.assertIn('- fix a thing', n)
    self.assertIn('- add a thing', n)
    self.assertNotIn('first monorepo commit', n)
    self.assertIn('pip install uclog==4`', n)
    first = releases.notes('v4-alpha1')
    self.assertIn('*A pre-release.*', first)
    self.assertIn('pip install uclog==4a1`', first)

  def test_manifest(self):
    d = self.root / 'dist'
    d.mkdir()
    for name in ['uclog-v4-linux-x64.tar.gz', 'uclog-v4-windows-arm64.zip', 'uclog-v4-linux-x64.tar.gz.sha256',
                 'uclog-4.vsix', 'uclog-4-py3-none-any.whl', 'uclog-4.tar.gz', 'uclog-app-4.html',
                 'uclog-element-4.iife.js', 'uclog-docs-4.tar.gz', 'uclog-v4-macos.dmg']:
      (d / name).write_bytes(b'abc')
    os.environ.pop('UCLOG_SIGNED_MACOS', None)
    m = json.loads(json.dumps(releases.manifest('v4', d)))
    kinds = {a['name']: (a['kind'], a['platform']) for a in m['assets']}
    self.assertEqual(kinds['uclog-v4-linux-x64.tar.gz'], ('archive', 'linux-x64'))
    self.assertEqual(kinds['uclog-v4-windows-arm64.zip'], ('archive', 'windows-arm64'))
    self.assertEqual(kinds['uclog-4.tar.gz'], ('python', None))
    self.assertEqual(kinds['uclog-docs-4.tar.gz'], ('docs', None))
    self.assertEqual(kinds['uclog-v4-macos.dmg'], ('dmg', 'macos'))
    self.assertNotIn('uclog-v4-linux-x64.tar.gz.sha256', kinds)
    self.assertEqual(m['assets'][0]['sha256'],
                     'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')  # "abc"
    self.assertEqual((m['device'], m['protocol'], m['prerelease'], m['signed']['macos']), ('4', '1', False, False))

  def test_table(self):
    t = releases.table(merged='v4')
    self.assertLess(t.index('[v4]'), t.index('[v4-alpha1]'))
    self.assertIn('(pre-release)', t)
    self.assertNotIn('[v1]', t)


if __name__ == '__main__':
  unittest.main()
