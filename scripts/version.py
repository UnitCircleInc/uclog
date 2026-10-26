#!/usr/bin/env python3
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Versions (plan 08 §2.2a): SoloVer, mapped to each manifest.

Two version lines: the host tools (Rust, Python, the web packages and the VSIX, in step) are
tagged `vN`; the device library is tagged `device-vN`. N is a SoloVer version: an integer,
optionally followed by `-alphaK`, `-betaK`, `-rcK` or `+hotfixK` (K from 1 to 9).

  version.py show                    the in-tree versions; fails if the manifests disagree
  version.py check TAG               the manifests match TAG (vN... or device-vN...): run on a tag
  version.py set VERSION [--device]  write VERSION into the host manifests (or the device header)
  version.py map VERSION             each ecosystem's form of VERSION

Each ecosystem's form:

  SoloVer         PyPI (PEP 440)   Cargo, npm, VSIX (SemVer)
  N               N                N.0.0
  N-alphaK        NaK              N.0.0-alpha.K   (also -beta, -rc; the VSIX is not published)
  N+hotfixK       N.postK          N.0.K
"""

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

CARGO_WORKSPACE = 'native/Cargo.toml'
CARGO_LOCK = 'native/Cargo.lock'
CARGO_CRATES = 'native/crates'
PYPROJECT = 'python/pyproject.toml'
PY_INIT = 'python/src/uclog/__init__.py'
# Workspace packages with a version; the others (the root, the demo app) are never published.
PACKAGE_JSON = [
  'web/packages/core/package.json',
  'web/packages/element/package.json',
  'web/packages/transports-web/package.json',
  'web/packages/ui/package.json',
  'web/packages/workers/package.json',
  'web/apps/vscode/package.json',
]
# The device library's header (device/include/log.h after plan 08 task 4).
DEVICE_HEADER = 'firmware/uclog/log.h'

SOLOVER = re.compile(r'(0|[1-9][0-9]*)(?:-(alpha|beta|rc)([1-9])|\+(hotfix)([1-9]))?')
PEP440_PRE = {'alpha': 'a', 'beta': 'b', 'rc': 'rc'}


class VersionError(Exception):
  pass


def parse(v):
  """SoloVer string → (N, kind, K); kind is None, 'alpha', 'beta', 'rc' or 'hotfix'."""
  m = SOLOVER.fullmatch(v)
  if not m:
    raise VersionError(f'{v!r} is not a version: N, N-alphaK, N-betaK, N-rcK or N+hotfixK '
                       '(N a whole number, K from 1 to 9)')
  n = int(m[1])
  if m[2]:
    return n, m[2], int(m[3])
  if m[4]:
    return n, 'hotfix', int(m[5])
  return n, None, 0


def pep440(v):
  n, kind, k = parse(v)
  if kind is None:
    return str(n)
  if kind == 'hotfix':
    return f'{n}.post{k}'
  return f'{n}{PEP440_PRE[kind]}{k}'


def semver(v):
  n, kind, k = parse(v)
  if kind is None:
    return f'{n}.0.0'
  if kind == 'hotfix':
    return f'{n}.0.{k}'
  return f'{n}.0.0-{kind}.{k}'


def from_semver(s):
  """The SoloVer version a SemVer string came from (the inverse of `semver`)."""
  m = re.fullmatch(r'(0|[1-9][0-9]*)\.0\.(0|[1-9])(?:-(alpha|beta|rc)\.([1-9]))?', s)
  if not m or (m[2] != '0' and m[3]):
    raise VersionError(f'{s!r} is not a SemVer form of a SoloVer version (N.0.0, N.0.0-alpha.K, N.0.K)')
  if m[3]:
    return f'{m[1]}-{m[3]}{m[4]}'
  if m[2] != '0':
    return f'{m[1]}+hotfix{m[2]}'
  return m[1]


def forms(v):
  return {'solover': v, 'pypi': pep440(v), 'cargo': semver(v), 'npm': semver(v), 'vsix': semver(v),
          'vsix_published': parse(v)[1] in (None, 'hotfix')}


# ---------------------------------------------------------------------------------------------
# Manifests: each is read and written with a regular expression, so the rest of the file is kept
# byte for byte.
# ---------------------------------------------------------------------------------------------

def _read(path):
  return (ROOT / path).read_text(encoding='utf-8')


CARGO_VERSION = r'(^\[workspace\.package\][^\[]*?^version = ")([^"]*)(")'
PY_VERSION = r'(^\[project\][^\[]*?^version = ")([^"]*)(")'
PY_INIT_VERSION = r"(^__version__ = ')([^']*)(')"
NPM_VERSION = r'(^  "version": ")([^"]*)(")'
DEVICE_VERSION = r'(^#define UCLOG_DEVICE_VERSION ")([^"]*)(")'


def _get(path, pattern, what):
  m = re.search(pattern, _read(path), flags=re.M | re.S)
  if not m:
    raise VersionError(f'{path}: {what} not found')
  return m[2]


def _crates():
  """The workspace's own crate names."""
  names = []
  for toml in sorted((ROOT / CARGO_CRATES).glob('*/Cargo.toml')):
    m = re.search(r'^\[package\]\s*\nname = "([^"]+)"', toml.read_text(encoding='utf-8'), flags=re.M)
    if m:
      names.append(m[1])
  return names


def _crate_dep_pattern(names):
  return r'^((?:' + '|'.join(map(re.escape, names)) + r') = \{ version = ")([^"]*)(", path = )'


def _lock_pattern(names):
  # A workspace crate's entry has no `source` line.
  return r'(^\[\[package\]\]\nname = "(?:' + '|'.join(map(re.escape, names)) + r')"\nversion = ")([^"]*)("\n(?!source))'


def host_versions():
  """Every host manifest's version: [(file, what, found, expected-from)] where expected-from is
  'cargo', 'pypi' or 'npm'."""
  out = [(CARGO_WORKSPACE, '[workspace.package] version', _get(CARGO_WORKSPACE, CARGO_VERSION, '[workspace.package] version'), 'cargo')]
  names = _crates()
  dep = re.compile(_crate_dep_pattern(names), flags=re.M)
  for toml in sorted((ROOT / CARGO_CRATES).glob('*/Cargo.toml')):
    rel = toml.relative_to(ROOT).as_posix()
    for m in dep.finditer(toml.read_text(encoding='utf-8')):
      out.append((rel, f'dependency {m[1].split(" ")[0]}', m[2], 'cargo'))
  for m in re.finditer(_lock_pattern(names), _read(CARGO_LOCK), flags=re.M):
    name = re.search(r'name = "([^"]+)"', m[1])[1]
    out.append((CARGO_LOCK, f'package {name}', m[2], 'cargo'))
  out.append((PYPROJECT, '[project] version', _get(PYPROJECT, PY_VERSION, '[project] version'), 'pypi'))
  out.append((PY_INIT, '__version__', _get(PY_INIT, PY_INIT_VERSION, '__version__'), 'pypi'))
  for p in PACKAGE_JSON:
    out.append((p, 'version', json.loads(_read(p))['version'], 'npm'))
  return out


def device_version():
  return _get(DEVICE_HEADER, DEVICE_VERSION, 'UCLOG_DEVICE_VERSION')


def mismatches(v):
  """Host manifests whose version is not the right form of SoloVer version `v`."""
  f = forms(v)
  return [f'{path}: {what} is {found!r}, expected {f[kind]!r}'
          for path, what, found, kind in host_versions() if found != f[kind]]


def set_host(v):
  f = forms(v)
  names = _crates()
  edits = {
    CARGO_WORKSPACE: [(CARGO_VERSION, f['cargo'], '[workspace.package] version')],
    CARGO_LOCK: [],
    PYPROJECT: [(PY_VERSION, f['pypi'], '[project] version')],
    PY_INIT: [(PY_INIT_VERSION, f['pypi'], '__version__')],
  }
  for p in PACKAGE_JSON:
    edits[p] = [(NPM_VERSION, f['npm'], 'version')]
  for path, subs in edits.items():
    text = _read(path)
    for pattern, value, what in subs:
      new, n = re.subn(pattern, lambda m: m[1] + value + m[3], text, count=1, flags=re.M | re.S)
      if n != 1:
        raise VersionError(f'{path}: {what} not found')
      text = new
    if path == CARGO_LOCK:
      text = re.sub(_lock_pattern(names), lambda m: m[1] + f['cargo'] + m[3], text, flags=re.M)
    (ROOT / path).write_text(text, encoding='utf-8')
  dep = _crate_dep_pattern(names)
  for toml in sorted((ROOT / CARGO_CRATES).glob('*/Cargo.toml')):
    text = toml.read_text(encoding='utf-8')
    toml.write_text(re.sub(dep, lambda m: m[1] + f['cargo'] + m[3], text, flags=re.M), encoding='utf-8')


def set_device(v):
  parse(v)
  text = _read(DEVICE_HEADER)
  new, n = re.subn(DEVICE_VERSION, lambda m: m[1] + v + m[3], text, count=1, flags=re.M)
  if n != 1:
    raise VersionError(f'{DEVICE_HEADER}: UCLOG_DEVICE_VERSION not found')
  (ROOT / DEVICE_HEADER).write_text(new, encoding='utf-8')


# ---------------------------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------------------------

def cmd_show(_):
  host = from_semver(_get(CARGO_WORKSPACE, CARGO_VERSION, '[workspace.package] version'))
  bad = mismatches(host)
  dev = device_version()
  parse(dev)
  print(f'host   {host}')
  print(f'device {dev}')
  if bad:
    raise VersionError('manifests disagree with the Rust workspace version:\n  ' + '\n  '.join(bad))


def cmd_check(a):
  tag = a.tag.removeprefix('refs/tags/')
  if tag.startswith('device-v'):
    v = tag.removeprefix('device-v')
    parse(v)
    found = device_version()
    if found != v:
      raise VersionError(f'{DEVICE_HEADER}: UCLOG_DEVICE_VERSION is {found!r}, the tag says {v!r}')
  elif tag.startswith('v'):
    v = tag[1:]
    parse(v)
    if parse(v)[0] < 4:
      raise VersionError(f'{tag}: host versions start at 4 (v1-v3 are the old device repository)')
    bad = mismatches(v)
    if bad:
      raise VersionError(f'manifests do not match {tag} (run scripts/version.py set {v}):\n  ' + '\n  '.join(bad))
  else:
    raise VersionError(f'{tag!r}: expected a tag vN... or device-vN...')
  print(f'{tag}: ok')


def cmd_set(a):
  if a.device:
    set_device(a.version)
  else:
    set_host(a.version)
  cmd_show(a)


def cmd_map(a):
  print(json.dumps(forms(a.version), indent=2))


def main(argv=None):
  p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
  sub = p.add_subparsers(dest='cmd', required=True)
  sub.add_parser('show').set_defaults(f=cmd_show)
  c = sub.add_parser('check')
  c.add_argument('tag')
  c.set_defaults(f=cmd_check)
  s = sub.add_parser('set')
  s.add_argument('version')
  s.add_argument('--device', action='store_true', help='the device library version')
  s.set_defaults(f=cmd_set)
  m = sub.add_parser('map')
  m.add_argument('version')
  m.set_defaults(f=cmd_map)
  a = p.parse_args(argv)
  try:
    a.f(a)
  except VersionError as e:
    print(f'error: {e}', file=sys.stderr)
    return 1
  return 0


if __name__ == '__main__':
  sys.exit(main())
