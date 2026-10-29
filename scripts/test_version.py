# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

# python3 -I -m unittest discover -s scripts

import contextlib
import io
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import version  # noqa: E402

TABLE = [
  # SoloVer, PEP 440, SemVer
  ('4', '4', '4.0.0'),
  ('12', '12', '12.0.0'),
  ('4-alpha1', '4a1', '4.0.0-alpha.1'),
  ('4-beta2', '4b2', '4.0.0-beta.2'),
  ('4-rc9', '4rc9', '4.0.0-rc.9'),
  ('4+hotfix3', '4.post3', '4.0.3'),
]


class Mapping(unittest.TestCase):
  def test_forms(self):
    for v, py, sem in TABLE:
      f = version.forms(v)
      self.assertEqual((f['pypi'], f['cargo'], f['npm'], f['vsix']), (py, sem, sem, sem), v)
      self.assertEqual(version.from_semver(sem), v)
      self.assertEqual(f['vsix_published'], '-' not in v, v)

  def test_order_is_kept(self):
    # SoloVer order: 4-alpha1 < 4-beta1 < 4-rc1 < 4 < 4+hotfix1 < 5; SemVer and PEP 440 agree.
    order = ['4-alpha1', '4-alpha2', '4-beta1', '4-rc1', '4', '4+hotfix1', '4+hotfix2', '5-alpha1', '5']
    def semkey(s):
      core, _, pre = s.partition('-')
      return tuple(map(int, core.split('.'))), (pre == '', pre)
    self.assertEqual(sorted(order, key=lambda v: semkey(version.semver(v))), order)

  def test_invalid(self):
    for v in ['', 'v4', '04', '4.0', '4-alpha', '4-alpha0', '4-alpha10', '4+alpha1', '4-hotfix1', '4-dev1', '4+hotfix0']:
      with self.assertRaises(version.VersionError, msg=v):
        version.parse(v)
    for s in ['0.1.0', '4.1.0', '4.0.10', '4.0.1-rc.1', '4.0.0-dev.1']:
      with self.assertRaises(version.VersionError, msg=s):
        version.from_semver(s)


class Manifests(unittest.TestCase):
  def setUp(self):
    self.tmp = tempfile.TemporaryDirectory()
    root = Path(self.tmp.name)
    paths = [version.CARGO_WORKSPACE, version.CARGO_LOCK, version.PYPROJECT, version.PY_INIT,
             version.DEVICE_HEADER, *version.PACKAGE_JSON]
    paths += [p.relative_to(version.ROOT).as_posix() for p in (version.ROOT / version.CARGO_CRATES).glob('*/Cargo.toml')]
    for p in paths:
      (root / p).parent.mkdir(parents=True, exist_ok=True)
      shutil.copy(version.ROOT / p, root / p)
    self.saved, version.ROOT = version.ROOT, root

  def tearDown(self):
    version.ROOT = self.saved
    self.tmp.cleanup()

  def run_cmd(self, *argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
      code = version.main(list(argv))
    return code, out.getvalue() + err.getvalue()

  def test_tree_is_consistent(self):
    self.assertEqual(self.run_cmd('show')[0], 0)

  def test_set_then_check(self):
    for v in ['4', '4-rc2', '7+hotfix1']:
      self.assertEqual(self.run_cmd('set', v)[0], 0)
      self.assertEqual(self.run_cmd('check', f'v{v}')[0], 0)
      self.assertTrue(all(found == version.forms(v)[k] for _, _, found, k in version.host_versions()))
    code, out = self.run_cmd('check', 'v8')
    self.assertEqual(code, 1)
    self.assertIn('native/Cargo.toml', out)

  def test_set_keeps_other_lock_entries(self):
    lock = (version.ROOT / version.CARGO_LOCK).read_text()
    self.run_cmd('set', '9')
    new = (version.ROOT / version.CARGO_LOCK).read_text()
    # Line by line: a line like version = "4.0.0" may also be another crate's.
    self.assertEqual(len(lock.splitlines()), len(new.splitlines()))
    changed = [(a, b) for a, b in zip(lock.splitlines(), new.splitlines()) if a != b]
    self.assertTrue(changed and all(a.startswith('version = ') and b == 'version = "9.0.0"' for a, b in changed))

  def test_device(self):
    self.assertEqual(self.run_cmd('set', '5-beta1', '--device')[0], 0)
    self.assertEqual(self.run_cmd('check', 'device-v5-beta1')[0], 0)
    self.assertEqual(self.run_cmd('check', 'device-v5')[0], 1)

  def test_old_and_bad_tags(self):
    self.assertEqual(self.run_cmd('check', 'v3')[0], 1)
    self.assertEqual(self.run_cmd('check', 'release-4')[0], 1)


if __name__ == '__main__':
  unittest.main()
