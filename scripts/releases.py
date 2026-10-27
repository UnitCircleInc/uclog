#!/usr/bin/env python3
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Releases, from the git tags (plan 08 tasks 17 and 18).

  releases.py notes TAG                 the release notes of host release TAG, as Markdown
                                        (release.yml uses them for the GitHub release)
  releases.py manifest TAG DIR          uclog-release-N.json for TAG: its assets in DIR with
                                        their sizes and SHA-256, and the signing status
                                        (UCLOG_SIGNED_MACOS, UCLOG_SIGNED_WINDOWS: 1 if signed)
  releases.py table                     the releases page for the docs (website/build.py)

Host releases are the tags vN (N a SoloVer version, 4 or later; scripts/version.py). The tags
v1–v3 are the old device repository's (spec/compatibility.md) and are not host releases. A
release's device library and wire protocol versions are read from the tree at its tag.
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'scripts'))
import version as solover  # noqa: E402

GITHUB = 'https://github.com/UnitCircleInc/uclog'
SITE = 'https://uclog.unitcircle.ca'
FIRST_HOST = 4  # v1–v3 are the old device repository's tags
MAX_CHANGES = 60

# Release archives: uclog-<tag>-<platform>.tar.gz / .zip (release.yml).
PLATFORMS = ['linux-x64', 'linux-arm64', 'macos-arm64', 'macos-x64', 'windows-x64', 'windows-arm64']


def git(*args) -> str:
  return subprocess.run(['git', '-C', str(ROOT), *args], check=True, capture_output=True,
                        text=True).stdout


def key(tag):
  n, kind, k = solover.parse(tag[1:])
  return (n, {'alpha': 0, 'beta': 1, 'rc': 2, None: 3, 'hotfix': 4}[kind], k or 0)


def is_prerelease(tag) -> bool:
  return solover.parse(tag[1:])[1] in ('alpha', 'beta', 'rc')


def host_tags(merged=None):
  """Host release tags, newest first; with `merged`, only those reachable from that commit."""
  out = []
  for t in git('tag', '--list', 'v*', *(['--merged', merged] if merged else [])).split():
    try:
      if solover.parse(t[1:])[0] >= FIRST_HOST:
        out.append(t)
    except solover.VersionError:
      pass
  return sorted(out, key=key, reverse=True)


def at(tag, path) -> str:
  return git('show', f'{tag}:{path}')


def device_version(tag) -> str:
  m = re.search(r'#define UCLOG_DEVICE_VERSION "([^"]+)"', at(tag, 'device/include/log.h'))
  return m[1] if m else '?'


def protocol_version(tag) -> str:
  m = re.search(r'^# ucLog wire protocol — version (\S+)', at(tag, 'spec/protocol.md'), re.M)
  return m[1] if m else '?'


def date(tag) -> str:
  return git('log', '-1', '--format=%cs', tag).strip()


def previous(tag):
  older = [t for t in host_tags(merged=tag) if key(t) < key(tag)]
  return older[0] if older else None


def changes(tag):
  """(previous tag or None, [(short sha, subject)]) — the first-parent history since it."""
  prev = previous(tag)
  rng = f'{prev}..{tag}' if prev else tag
  lines = git('log', '--first-parent', '--no-merges', '--format=%h %s', rng).splitlines()
  return prev, [tuple(line.split(' ', 1)) for line in lines if ' ' in line]


def notes(tag) -> str:
  n = tag[1:]
  dev, proto = device_version(tag), protocol_version(tag)
  prev, commits = changes(tag)
  out = [
    f'**Device library** `device-v{dev}` · **wire protocol** version {proto} · '
    f'[compatibility]({SITE}/v{n}/protocol/compatibility/) · [docs]({SITE}/v{n}/)',
    '',
    f'**Install:** the archive for your platform below (or the [download page]({SITE}/download/)), '
    f'`pip install uclog=={solover.pep440(n)}`, the `.vsix` for VS Code, or the web app at '
    f'[{SITE.removeprefix("https://")}/app/]({SITE}/app/) (also one file: `uclog-app-{n}.html`).',
  ]
  if is_prerelease(tag):
    out += ['', '*A pre-release.*']
  out += ['', f'## Changes since {prev}' if prev else '## Changes', '']
  shown = commits[:MAX_CHANGES]
  out += [f'- {subject} ({sha})' for sha, subject in shown]
  if len(commits) > len(shown):
    link = f'{GITHUB}/compare/{prev}...{tag}' if prev else f'{GITHUB}/commits/{tag}'
    out.append(f'- … and [{len(commits) - len(shown)} more]({link})')
  return '\n'.join(out) + '\n'


def sha256(path) -> str:
  h = hashlib.sha256()
  with open(path, 'rb') as f:
    for b in iter(lambda: f.read(1 << 20), b''):
      h.update(b)
  return h.hexdigest()


def kind_of(name, tag):
  """(kind, platform) of a release asset by its name."""
  n = tag[1:]
  for p in PLATFORMS:
    if name in (f'uclog-{tag}-{p}.tar.gz', f'uclog-{tag}-{p}.zip'):
      return 'archive', p
  if name == f'uclog-{tag}-macos.dmg':
    return 'dmg', 'macos'
  if name.endswith('.vsix'):
    return 'vsix', None
  if name.endswith('.whl') or (name.endswith('.tar.gz') and name.startswith(f'uclog-{solover.pep440(n)}.')):
    return 'python', None
  if name == f'uclog-app-{n}.html':
    return 'app', None
  if name == f'uclog-element-{n}.iife.js':
    return 'element', None
  if name == f'uclog-docs-{n}.tar.gz':
    return 'docs', None
  return 'other', None


def manifest(tag, folder) -> dict:
  assets = []
  for f in sorted(Path(folder).iterdir()):
    if not f.is_file() or f.name.endswith('.sha256') or f.name.startswith('uclog-release-'):
      continue
    kind, platform = kind_of(f.name, tag)
    assets.append({'name': f.name, 'kind': kind, 'platform': platform, 'size': f.stat().st_size,
                   'sha256': sha256(f)})
  return {
    'tag': tag, 'version': tag[1:], 'date': date(tag), 'prerelease': is_prerelease(tag),
    'device': device_version(tag), 'protocol': protocol_version(tag),
    'pypi': solover.pep440(tag[1:]),
    'signed': {'macos': os.environ.get('UCLOG_SIGNED_MACOS') == '1',
               'windows': os.environ.get('UCLOG_SIGNED_WINDOWS') == '1'},
    'assets': assets,
  }


def table(merged='HEAD') -> str:
  tags = host_tags(merged=merged)
  out = ['# Releases', '',
         'Every host release (`vN`: `uclog`, the hub, Python, VS Code, the web app and `<uc-log>`, '
         'released together), with the device library and wire protocol in it. Which device '
         'libraries work with which hosts: [Compatibility](protocol/compatibility.md). Downloads: '
         f'[the download page]({SITE}/download/).', '']
  if not tags:
    return '\n'.join(out + ['No release yet: these docs are built from `main`.', ''])
  out += ['| Release | Date | Device library | Wire protocol |', '|---|---|---|---|']
  for t in tags:
    pre = ' (pre-release)' if is_prerelease(t) else ''
    out.append(f'| [{t}]({GITHUB}/releases/tag/{t}){pre} | {date(t)} | `device-v{device_version(t)}` '
               f'| {protocol_version(t)} |')
  for t in tags:
    body = notes(t).replace('## Changes', '### Changes')
    out += ['', f'## {t}', '', body]
  return '\n'.join(out)


def main(argv=None):
  ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
  sub = ap.add_subparsers(dest='cmd', required=True)
  n = sub.add_parser('notes')
  n.add_argument('tag')
  m = sub.add_parser('manifest')
  m.add_argument('tag')
  m.add_argument('dir')
  sub.add_parser('table')
  a = ap.parse_args(argv)
  if a.cmd == 'notes':
    sys.stdout.write(notes(a.tag))
  elif a.cmd == 'manifest':
    json.dump(manifest(a.tag, a.dir), sys.stdout, indent=1)
    sys.stdout.write('\n')
  else:
    sys.stdout.write(table())


if __name__ == '__main__':
  main()
