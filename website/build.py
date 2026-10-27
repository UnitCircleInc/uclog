#!/usr/bin/env python3
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
"""Build uclog.unitcircle.ca (plan 08 §3).

  build.py docs VERSION OUT       one version of the docs (VERSION: vN... or dev) into OUT
  build.py assemble OUT [options] the whole Pages site from built parts (below)
  build.py serve                  a live preview of the docs (http://localhost:8000)
  build.py screenshots            regenerate docs/assets/screens (plan 08 task 11; then commit them)

The site (mike's layout, plan 08 §3.2):

  /                     redirects to /latest/ (or /dev/ before the first release)
  /vN/                  the docs of release vN, from its uclog-docs-N.tar.gz
  /latest/              a copy of the newest release that is not a pre-release
  /dev/                 the docs built from main
  /versions.json        the version selector's list (newest first)
  /app/                 the web app (web/apps/demo), with its service worker
  /app/uclog-app.html   the web app as one file (plus uclog-app-N.html per release)
  /element/N/, latest/  <uc-log> as one script (uclog-element.iife.js) per release; /element/dev/
  /CNAME                uclog.unitcircle.ca

The spec pages are generated from ../spec (one copy of the spec): links into the repository
become links to GitHub at the version's tag (main for dev). The <uc-log> reference is generated
from the element's Custom Elements Manifest (`cem analyze` in ../web: needs its node_modules).
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(ROOT / 'scripts'))
import version as solover  # noqa: E402

GITHUB = 'https://github.com/UnitCircleInc/uclog'
DOMAIN = 'uclog.unitcircle.ca'

# Generated pages: docs path -> spec file.
SPEC_PAGES = {
  'protocol/index.md': 'spec/protocol.md',
  'protocol/compatibility.md': 'spec/compatibility.md',
}


def ref_for(version):
  return 'main' if version == 'dev' else version


LIST_ITEM = re.compile(r'^(\s*)([-*+]|\d+\.)\s')


def python_markdown_lists(text: str) -> str:
  """GitHub-style lists for Python-Markdown: a blank line before a list that follows a paragraph,
  and 4-space nesting (the spec nests by 2). Code at the top level is left as it is."""
  out = []
  fence = None  # the indent of the open fence, or None
  prev = ''
  for line in text.split('\n'):
    stripped = line.lstrip(' ')
    indent = len(line) - len(stripped)
    is_fence = stripped.startswith('```')
    if fence is not None and fence == 0:
      out.append(line)
      if is_fence and indent == 0:
        fence = None
      prev = line
      continue
    if is_fence and fence is None:
      fence = indent
    elif is_fence and fence is not None:
      fence = None
    if LIST_ITEM.match(line) and prev.strip() and not LIST_ITEM.match(prev) and not prev.startswith(' ') \
        and not prev.lstrip().startswith('|'):
      out.append('')
    out.append(' ' * (indent * 2) + stripped if indent else line)
    prev = line
  return '\n'.join(out)


def spec_page(src: Path, ref: str) -> str:
  """The spec file with its links into the repository pointing at GitHub (at `ref`)."""
  text = src.read_text(encoding='utf-8')
  base = src.parent.relative_to(ROOT)

  def link(m):
    target = m[2]
    if re.match(r'[a-z]+:|#|/', target):
      return m[0]
    path, _, frag = target.partition('#')
    rel = os.path.normpath(base / path)
    if (ROOT / rel).resolve() == (ROOT / 'spec/compatibility.md').resolve():
      return f'{m[1]}(compatibility.md{"#" + frag if frag else ""})'
    if (ROOT / rel).resolve() == (ROOT / 'spec/protocol.md').resolve():
      return f'{m[1]}(index.md{"#" + frag if frag else ""})'
    kind = 'tree' if (ROOT / rel).is_dir() else 'blob'
    return f'{m[1]}({GITHUB}/{kind}/{ref}/{rel}{"#" + frag if frag else ""})'

  return python_markdown_lists(re.sub(r'(\]\s?)\(([^)\s]+)\)', link, text))


REFERENCE_PAGE = 'reference/uc-log.md'
PYTHON_REFERENCE_PAGE = 'reference/python.md'


def one_line(text):
  return ' '.join((text or '').split())


def element_manifest() -> dict:
  """<uc-log>'s Custom Elements Manifest, made now from the source (web/custom-elements-manifest.config.mjs)."""
  web = ROOT / 'web'
  if not (web / 'node_modules/.bin/cem').exists():
    raise SystemExit('web/node_modules is missing: run `npx -y pnpm@12.10.1 install` in web/ first')
  out = HERE / '.cache/cem'
  out.mkdir(parents=True, exist_ok=True)
  # --outdir is relative to web/ (the analyzer joins it to the working directory).
  subprocess.run(['npx', 'cem', 'analyze', '--outdir', os.path.relpath(out, web)], cwd=web, check=True,
                 stdout=subprocess.DEVNULL)
  return json.loads((out / 'custom-elements.json').read_text(encoding='utf-8'))


def element_reference(manifest: dict, ref: str) -> str:
  """The reference page: attributes, properties, methods, events, parts and CSS properties."""
  el = next(d for m in manifest['modules'] for d in m.get('declarations', []) if d.get('tagName') == 'uc-log')
  src = f'{GITHUB}/blob/{ref}/web/packages/ui/src/uc-log.ts'
  members = [m for m in el.get('members', []) if m.get('privacy', 'public') == 'public' and not m.get('static')]

  def default(a):
    d = a.get('default', '')
    return f'`{d.strip(chr(39))}`' if re.fullmatch(r"'[^']+'|true|false|\d+", d) else ''

  def typ(t):
    return ' or '.join(f'`{x.strip()}`' for x in (t or {}).get('text', '').split('|') if x.strip())

  lines = [
    '# `<uc-log>` reference',
    '',
    f'{one_line(el.get("description"))} How to add it to a page: [Embedding](../embedding.md).',
    '',
    f'Generated from the element\'s source ([`uc-log.ts`]({src})) through its Custom Elements Manifest, '
    'which the package also ships (`custom-elements.json`) for editors.',
    '',
    '## Attributes',
    '',
    'Each is also a property, named in camelCase (`logdata-domain`: `logdataDomain`).',
    '',
    '| Attribute | |',
    '|---|---|',
  ]
  for a in el.get('attributes', []):
    d = default(a)
    lines.append(f'| `{a["name"]}` | {one_line(a.get("description"))}{f" Default: {d}." if d else ""} |')
  props = [m for m in members if m['kind'] == 'field' and not m.get('attribute')]
  if props:
    lines += ['', '## Properties', '', '| Property | Type | |', '|---|---|---|']
    for m in props:
      lines.append(f'| `{m["name"]}` | {typ(m.get("type"))} | {one_line(m.get("description"))} |')
  lines += ['', '## Methods', '']
  for m in (m for m in members if m['kind'] == 'method'):
    params = ', '.join(p['name'] for p in m.get('parameters', []))
    lines += [f'### `{m["name"]}({params})`', '', one_line(m.get('description')), '']
  lines += ['## Events', '', 'All bubble and are composed (they cross shadow roots).', '',
            '| Event | |', '|---|---|']
  for e in el.get('events', []):
    lines.append(f'| `{e["name"]}` | {one_line(e.get("description"))} |')
  lines += ['', '## CSS parts', '', 'Style them with `uc-log::part(name)`.', '', '| Part | |', '|---|---|']
  for p in el.get('cssParts', []):
    lines.append(f'| `{p["name"]}` | {one_line(p.get("description"))} |')
  lines += ['', '## CSS custom properties', '', 'Set them on the element, for example `uc-log { --uc-accent: #0b6e4f; }`.', '',
            '| Property | |', '|---|---|']
  for p in el.get('cssProperties', []):
    lines.append(f'| `{p["name"]}` | {one_line(p.get("description"))} |')
  return names_tables('\n'.join(lines) + '\n')


def names_tables(md: str) -> str:
  """Wrap each table in <div class="uc-names" markdown> (uclog.css keeps its names on one line)."""
  return re.sub(r'((?:^\|.*\n)+)', lambda m: f'<div class="uc-names" markdown>\n\n{m[1]}\n</div>\n', md, flags=re.M)


# The Python reference: uclog's public names (its __all__), grouped; each with its signature and
# docstring (the docstrings are Markdown: `code`, lists, indented examples).
PYTHON_GROUPS = [
  ('Connecting', ['connect', 'Connection', 'aio']),
  ('Hubs and devices', ['list_hubs', 'list_devices', 'start_hub', 'stop_hub', 'HubInfo', 'DeviceInfo']),
  ('Logs and logdata', ['LogRecord', 'LogData', 'Resolver']),
  ('Errors', ['UclogError', 'NoHubError', 'MultipleHubsError', 'HubBusyError', 'ApprovalTimeoutError',
              'DeviceNotRespondingError', 'HubStartError', 'ConnectionClosedError', 'DecodeError',
              'LogDataError']),
  ('Protocol helpers', ['cbor', 'proto']),
]


def docstring_md(text: str) -> str:
  """A docstring as Python-Markdown: a blank line before a list that follows a paragraph, and
  indented examples (after a blank line) as fenced Python."""
  out, block, prev = [], None, ''
  for line in text.split('\n') + ['']:
    if block is not None:
      if line.startswith('    ') or (not line.strip() and block):
        block.append(line[4:])
        continue
      while block and not block[-1].strip():
        block.pop()
      out += ['```python', *block, '```', '']
      block = None
    if line.startswith('    ') and not prev.strip() and not LIST_ITEM.match(line):
      block = [line[4:]]
      continue
    if LIST_ITEM.match(line) and prev.strip() and not LIST_ITEM.match(prev) and not prev.startswith(' '):
      out.append('')
    out.append(line)
    prev = line
  return '\n'.join(out).rstrip() + '\n'


def python_reference(ref: str) -> str:
  import dataclasses
  import inspect
  sys.path.insert(0, str(ROOT / 'python/src'))
  try:
    import uclog
  finally:
    sys.path.pop(0)
  grouped = [n for _, names in PYTHON_GROUPS for n in names]
  missing = sorted(set(uclog.__all__) - set(grouped))
  if missing:
    raise SystemExit(f'build.py: put {missing} in PYTHON_GROUPS (uclog.__all__ changed)')

  def doc(o) -> str:
    return docstring_md(inspect.getdoc(o) or '*(no description)*')

  def sig(name, f) -> str:
    try:
      s = inspect.signature(f)
    except (TypeError, ValueError):
      return f'{name}(…)'
    plain = s.replace(parameters=[p.replace(annotation=p.empty) for p in s.parameters.values()],
                      return_annotation=s.empty)
    return f'{name}{plain}'.replace('(self, ', '(').replace('(self)', '()')

  def fn(level, name, f, body) -> list:
    """A function's heading (its name), its signature, then its description."""
    return [f'{level} `{name}`', '', '```python', sig(name, f), '```', '', body, '']

  def own_members(cls):
    for n, m in vars(cls).items():
      if n.startswith('_'):
        continue
      if isinstance(m, property):
        yield n, 'property', m
      elif isinstance(m, (staticmethod, classmethod)):
        yield n, 'method', m.__func__
      elif inspect.isfunction(m):
        yield n, 'method', m

  out = ['# Python reference', '',
         'The `uclog` package, generated from its docstrings '
         f'([source]({GITHUB}/tree/{ref}/python/src/uclog)). A guide: [Python](../python.md).', '',
         doc(uclog), '']
  for title, names in PYTHON_GROUPS:
    out += [f'## {title}', '']
    for n in names:
      o = getattr(uclog, n)
      if inspect.ismodule(o):
        out += [f'### `uclog.{n}`', '', doc(o), '']
        consts = []
        for m in getattr(o, '__all__', []):
          f = getattr(o, m)
          if inspect.isfunction(f):
            out += fn('####', f'{n}.{m}', f, doc(f))
          elif inspect.isclass(f) and not issubclass(f, BaseException):
            out += [f'#### `{n}.{m}`', '', doc(f), '']
            for mm, kind, ff in own_members(f):
              if kind == 'method':
                # aio.Connection's methods are the sync ones, awaited.
                body = doc(ff) if ff.__doc__ or n != 'aio' else f'As `Connection.{mm}`, awaited.'
                out += ['```python', sig(f'{m}.{mm}', ff), '```', '', body, '']
          elif isinstance(f, int):
            consts.append(f'| `{n}.{m}` | {f} |')
        if consts:
          out += [f'#### Constants', '', '| Name | Value |', '|---|---|', *consts, '']
      elif inspect.isclass(o) and issubclass(o, BaseException):
        base = o.__mro__[1].__name__
        out += [f'### `{n}`', '', f'*Exception, a `{base}`.* ' + doc(o), '']
      elif inspect.isclass(o):
        out += [f'### `{n}`', '', doc(o), '']
        if dataclasses.is_dataclass(o):
          out += ['| Field | Type |', '|---|---|']
          out += [f'| `{f.name}` | `{f.type if isinstance(f.type, str) else f.type.__name__}` |'
                  for f in dataclasses.fields(o)]
          out.append('')
        for m, kind, f in own_members(o):
          out += [f'#### `{n}.{m}`', '', doc(f), ''] if kind == 'property' else fn('####', f'{n}.{m}', f, doc(f))
      else:
        out += fn('###', f'uclog.{n}', o, doc(o))
  return '\n'.join(out)


def generate(version):
  ref = ref_for(version)
  for page, src in SPEC_PAGES.items():
    out = HERE / 'docs' / page
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(spec_page(ROOT / src, ref), encoding='utf-8')
  out = HERE / 'docs' / REFERENCE_PAGE
  out.parent.mkdir(parents=True, exist_ok=True)
  out.write_text(element_reference(element_manifest(), ref), encoding='utf-8')
  (HERE / 'docs' / PYTHON_REFERENCE_PAGE).write_text(python_reference(ref), encoding='utf-8')


def zensical(*args, env=None):
  subprocess.run(['zensical', *args, '-f', str(HERE / 'zensical.toml')], cwd=HERE, check=True,
                 env={**os.environ, **(env or {})})


MARK = '.uclog-build'  # written into each output; only a directory with it (or empty) is replaced


def fresh(out: Path):
  """Empty `out` for a new build, refusing a directory this script did not make."""
  if out.exists():
    if any(out.iterdir()) and not (out / MARK).exists():
      raise SystemExit(f'{out}: not empty and not made by build.py; not replacing it')
    shutil.rmtree(out)


def cmd_docs(a):
  if a.version != 'dev':
    solover.parse(a.version.removeprefix('v'))
  generate(a.version)
  # MIKE_DOCS_VERSION: Zensical puts the version into site_url (canonical links, the selector).
  zensical('build', '--clean', '--strict', env={'MIKE_DOCS_VERSION': a.version})  # warnings fail
  out = Path(a.out)
  fresh(out)
  shutil.copytree(HERE / 'site', out)
  (out / MARK).touch()
  print(f'docs {a.version} -> {out}')


def cmd_serve(a):
  generate('dev')
  zensical('serve')


SCREENS = HERE / 'docs/assets/screens'


def run(cmd, cwd, **env):
  print('+', ' '.join(str(c) for c in cmd), flush=True)
  subprocess.run([str(c) for c in cmd], cwd=cwd, check=True, env={**os.environ, **env})


def cmd_screenshots(a):
  """The screenshots in the docs, from the hosts themselves: the web app and VS Code (Playwright),
  the GUI (egui_kittest, rendered with wgpu) and the TUI (ratatui's TestBackend, as SVG). Needs
  Chrome, VS Code, and the Rust toolchain; the VS Code one runs from a VSIX
  (web/apps/vscode/uclog-*.vsix, built here if there is none)."""
  web, native = ROOT / 'web', ROOT / 'native'
  SCREENS.mkdir(parents=True, exist_ok=True)
  with tempfile.TemporaryDirectory() as t:
    shots = Path(t)
    run(['npx', 'playwright', 'test', 'screens'], web, UCLOG_SHOTS=str(shots))
    for src, dst in [('light-logs', 'web-logs-light'), ('dark-logs', 'web-logs-dark'), ('light-plots', 'web-plots-light'),
                     ('light-terminal', 'web-terminal-light')]:
      shutil.copy(shots / f'{src}.png', SCREENS / f'{dst}.png')
    vsix = sorted((web / 'apps/vscode').glob('uclog-*.vsix'), key=lambda p: p.stat().st_mtime)
    if a.rebuild_vsix or not vsix:
      vs = web / 'apps/vscode'
      run(['npx', 'vite', 'build', '--mode', 'host'], vs)
      run(['npx', 'vite', 'build'], vs)
      run(['node', 'scripts/package.mjs', '--local'], vs)
      vsix = sorted(vs.glob('uclog-*.vsix'), key=lambda p: p.stat().st_mtime)
    run(['npx', 'playwright', 'test', '-c', 'apps/vscode/playwright.config.ts', 'screens'], web,
        UCLOG_SHOTS=str(shots), UCLOG_VSIX=str(vsix[-1]))
    run(['cargo', 'test', '-p', 'uclog', '--bin', 'uclog', 'gui_screens', '--', '--ignored'], native, UCLOG_SHOTS=str(shots))
    run(['cargo', 'test', '-p', 'uclog', '--bin', 'uclog', 'tui_screens', '--', '--ignored'], native, UCLOG_SHOTS=str(shots))
    for f in ['vscode-logs.png', 'gui-logs-light.png', 'gui-logs-dark.png', 'gui-plots-light.png', 'tui-logs.svg',
              'tui-terminal.svg']:
      shutil.copy(shots / f, SCREENS / f)
  print(f'screenshots -> {SCREENS}')


def unpack(tar: Path, dest: Path):
  with tarfile.open(tar) as t:
    t.extractall(dest, filter='data')


def version_key(v):
  n, kind, k = solover.parse(v.removeprefix('v'))
  rank = {'alpha': 0, 'beta': 1, 'rc': 2, None: 3, 'hotfix': 4}[kind]
  return (n, rank, k)


def is_prerelease(v):
  return solover.parse(v.removeprefix('v'))[1] in ('alpha', 'beta', 'rc')


def redirect(to):
  return (f'<!doctype html><meta charset="utf-8"><title>ucLog</title>'
          f'<meta http-equiv="refresh" content="0; url={to}"><link rel="canonical" href="{to}">'
          f'<a href="{to}">{to}</a>\n')


def cmd_assemble(a):
  out = Path(a.out)
  fresh(out)
  out.mkdir(parents=True)
  (out / MARK).touch()
  releases = {}
  for tar in a.release_docs or []:
    m = re.fullmatch(r'uclog-docs-(.+)\.tar\.gz', Path(tar).name)
    if not m:
      raise SystemExit(f'{tar}: expected uclog-docs-N.tar.gz')
    v = 'v' + m[1]
    with tempfile.TemporaryDirectory() as t:
      unpack(Path(tar), Path(t))
      inner = [p for p in Path(t).iterdir() if p.is_dir()]
      shutil.copytree(inner[0] if len(inner) == 1 else Path(t), out / v)
    releases[v] = out / v
  if a.dev:
    shutil.copytree(a.dev, out / 'dev')

  ordered = sorted(releases, key=version_key, reverse=True)
  stable = [v for v in ordered if not is_prerelease(v)]
  latest = stable[0] if stable else None
  versions = [{'version': v, 'title': f'{v} (pre-release)' if is_prerelease(v) else v,
               'aliases': ['latest'] if v == latest else []} for v in ordered]
  if a.dev:
    versions.append({'version': 'dev', 'title': 'dev (main)', 'aliases': []})
  (out / 'versions.json').write_text(json.dumps(versions, indent=1) + '\n')
  if latest:
    shutil.copytree(out / latest, out / 'latest')
  home = 'latest/' if latest else ('dev/' if a.dev else None)
  if home:
    (out / 'index.html').write_text(redirect(home))
    if (out / home / '404.html').exists():
      shutil.copy(out / home / '404.html', out / '404.html')

  if a.app:
    shutil.copytree(a.app, out / 'app')
  for spec in a.app_file or []:  # VERSION=path: uclog-app-N.html
    v, _, path = spec.partition('=')
    shutil.copy(path, out / 'app' / f'uclog-app-{v.removeprefix("v")}.html')
  for spec in a.element or []:  # VERSION=path to uclog-element.iife.js
    v, _, path = spec.partition('=')
    d = out / 'element' / v.removeprefix('v')
    d.mkdir(parents=True, exist_ok=True)
    shutil.copy(path, d / 'uclog-element.iife.js')
  elements = sorted((p.name for p in (out / 'element').glob('*') if p.name != 'dev'),
                    key=lambda n: version_key(n), reverse=True) if (out / 'element').exists() else []
  stable_elements = [n for n in elements if not is_prerelease(n)]
  if stable_elements:
    shutil.copytree(out / 'element' / stable_elements[0], out / 'element' / 'latest')
  (out / 'CNAME').write_text(DOMAIN + '\n')
  (out / '.nojekyll').write_text('')
  print(f'site -> {out}: docs {", ".join(ordered + (["dev"] if a.dev else [])) or "none"}; '
        f'latest {latest or "none"}; app {"yes" if a.app else "no"}; elements {", ".join(elements) or "none"}')


def main():
  p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
  sub = p.add_subparsers(dest='cmd', required=True)
  d = sub.add_parser('docs')
  d.add_argument('version')
  d.add_argument('out')
  d.set_defaults(f=cmd_docs)
  s = sub.add_parser('serve')
  s.set_defaults(f=cmd_serve)
  sc = sub.add_parser('screenshots')
  sc.add_argument('--rebuild-vsix', action='store_true', help='build the VSIX from the current code first')
  sc.set_defaults(f=cmd_screenshots)
  x = sub.add_parser('assemble')
  x.add_argument('out')
  x.add_argument('--dev', help='the docs built from main (build.py docs dev)')
  x.add_argument('--release-docs', nargs='*', help='uclog-docs-N.tar.gz files')
  x.add_argument('--app', help='the web app build (web/apps/demo dist)')
  x.add_argument('--app-file', nargs='*', help='VERSION=uclog-app-N.html')
  x.add_argument('--element', nargs='*', help='VERSION=uclog-element.iife.js (dev too)')
  x.set_defaults(f=cmd_assemble)
  a = p.parse_args()
  a.f(a)


if __name__ == '__main__':
  main()
