# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Choosing logdata by the device's app_hash (protocol.md §6.4, plans README §5.3).

Order: the files given to `connect(logdata=...)` whose name carries no other hash, then the cache
directory (`<cache>/<hash>.logdata`), then fw.unitcircle.ca when a domain is set, then the first
given file as a fallback (with a warning).
"""

import json
import os
import sys
import urllib.error
import urllib.request

from .logdata import LogData, LogDataError

__all__ = ['Resolver', 'default_cache_dir']

DEFAULT_URL = 'https://fw.unitcircle.ca'


def default_cache_dir() -> str:
  """The logdata cache shared with the native tools (`<platform cache dir>/logdata`)."""
  if sys.platform == 'darwin':
    base = os.path.expanduser('~/Library/Caches/ca.unitcircle.uclog')
  elif sys.platform == 'win32':
    local = os.environ.get('LOCALAPPDATA') or os.path.expanduser('~\\AppData\\Local')
    base = os.path.join(local, 'unitcircle', 'uclog', 'cache')
  else:
    base = os.path.join(os.environ.get('XDG_CACHE_HOME') or os.path.expanduser('~/.cache'), 'uclog')
  return os.path.join(base, 'logdata')


class Resolver:
  def __init__(self, files=(), cache_dir=None, domain=None, url=None, token=None):
    self.files = [os.fspath(f) for f in files]
    self.cache_dir = os.fspath(cache_dir) if cache_dir is not None else default_cache_dir()
    self.domain = domain if domain is not None else os.environ.get('UCLOG_LOGDATA_DOMAIN')
    self.url = (url or os.environ.get('UCLOG_LOGDATA_URL') or DEFAULT_URL).rstrip('/')
    self.token = token if token is not None else os.environ.get('UCLOG_LOGDATA_TOKEN')

  def manual(self):
    """The given files, loaded (used before any device info arrives)."""
    out = []
    for f in self.files:
      out.append(LogData.load(f))
    return out

  def resolve(self, app_hash: bytes):
    """Returns (list of LogData, source text, notes)."""
    h = app_hash.hex()
    notes = []
    loaded = []
    for f in self.files:
      try:
        loaded.append(LogData.load(f))
      except (OSError, LogDataError) as e:
        notes.append(f'{f}: {e}')
    match = [ld for ld in loaded if ld.app_hash in (None, app_hash)]
    for ld in loaded:
      if ld not in match:
        notes.append(f'{ld.source}: app_hash does not match the device; ignored')
    if match:
      return match, f'manual {match[0].source}', notes

    p = os.path.join(self.cache_dir, f'{h}.logdata')
    if os.path.exists(p):
      try:
        return [LogData.load(p)], f'cache {p}', notes
      except (OSError, LogDataError) as e:
        notes.append(f'cache {p}: {e}')

    if self.domain:
      url = f'{self.url}/logdata/{self.domain}/{h}'
      try:
        data = _fetch(url, self.token)
        if data is not None:
          ld = LogData(data, url)
          ld.app_hash = app_hash
          try:
            os.makedirs(self.cache_dir, exist_ok=True)
            with open(p, 'wb') as f:
              f.write(data)
          except OSError as e:
            notes.append(f'cache write {p}: {e}')
          return [ld], f'remote {url}', notes
      except (OSError, LogDataError) as e:
        notes.append(f'{url}: {e}')

    if loaded:
      notes.append(f'no logdata for app_hash {h[:16]}…; using {loaded[0].source}')
      return loaded, f'fallback {loaded[0].source}', notes
    return [], None, notes


def _fetch(url, token):
  """GET with an optional bearer token; None on 404. urllib follows the redirect to the download
  link."""
  req = urllib.request.Request(url)
  if token:
    req.add_header('Authorization', f'Bearer {token}')
  try:
    with urllib.request.urlopen(req, timeout=30) as r:
      return r.read()
  except urllib.error.HTTPError as e:
    if e.code == 404:
      return None
    detail = ''
    try:
      body = json.loads(e.read().decode('utf-8', 'replace'))
      detail = body.get('detail') or body.get('title') or ''
    except (ValueError, AttributeError):
      pass
    rid = e.headers.get('x-request-id') if e.headers else None
    raise OSError(f'HTTP {e.code}' + (f': {detail}' if detail else '') +
                  (f' (request id {rid})' if rid else '')) from None
  except urllib.error.URLError as e:
    raise OSError(str(e.reason)) from None
