# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Choosing logdata by the device's app_hash (protocol.md §6.4, plans README §5.3).

Order: the files given to `connect(logdata=...)` whose name carries no other hash, then the cache
directory (`<cache>/<hash>.logdata`), then fw.unitcircle.ca when a domain is set, then the first
given file as a fallback (with a warning).
"""

import json
import os
import urllib.error
import urllib.request

from .logdata import LogData, LogDataError

__all__ = ['Resolver', 'default_cache_dir']

DEFAULT_URL = 'https://fw.unitcircle.ca'


def default_cache_dir() -> str:
  """The logdata cache shared with the native tools and the device builds.

  `$UCLOG_CACHE_DIR`, else `$XDG_CACHE_HOME/uclog`, else `~/.cache/uclog`, on every platform.
  """
  if os.environ.get('UCLOG_CACHE_DIR'):
    return os.environ['UCLOG_CACHE_DIR']
  return os.path.join(os.environ.get('XDG_CACHE_HOME') or os.path.expanduser('~/.cache'), 'uclog')


class Resolver:
  """Finds the logdata for a device's app hash, in this order:

  1. the given `files` (`.logdata` or ELF); a file named `<app_hash>.logdata` is used for that hash
     only;
  2. `<cache_dir>/<app_hash>.logdata` (default: $UCLOG_CACHE_DIR, else $XDG_CACHE_HOME/uclog,
     else ~/.cache/uclog);
  3. the logdata service, when `domain` is set (`GET {url}/logdata/{domain}/{app_hash}`, with
     `token` if set; a refused token is retried without it, and `token_refused` is set). The
     download is saved in the cache;
  4. the first given file, with a note.

  `connect()` makes one; use it directly to resolve logdata without a device.
  """

  def __init__(self, files=(), cache_dir=None, domain=None, url=None, token=None):
    self.files = [os.fspath(f) for f in files]
    self.cache_dir = os.fspath(cache_dir) if cache_dir is not None else default_cache_dir()
    # Set when the service refused the token but read without it worked.
    self.token_refused = False
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
    """The logdata for `app_hash`: (list of LogData, where it came from, notes on the way)."""
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
        try:
          data = _fetch(url, self.token)
        except OSError as e:
          # A refused token: the account may allow reading without one; try once more without it.
          if not (self.token and str(e).startswith('HTTP 401')):
            raise
          try:
            data = _fetch(url, None)
          except OSError:
            raise e from None
          if data is None:
            raise e from None
          self.token_refused = True
          notes.append(f'{url}: the logdata token was refused, but the account allows reading '
                       'without a token; read without it (remove logdata_token / '
                       '$UCLOG_LOGDATA_TOKEN)')
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


def auth_hint(status, sent_token):
  """What to do about a refused logdata request."""
  if status == 401 and not sent_token:
    return ('No token was sent: set logdata_token (or $UCLOG_LOGDATA_TOKEN), or have the account '
            'allow reading without a token')
  if status == 401:
    return 'The token was not accepted (expired or revoked?): set a new logdata token'
  if status == 403:
    return 'The token is for another account than this domain, or lacks the logdata:read scope'
  return None


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
    hint = auth_hint(e.code, bool(token))
    raise OSError(f'HTTP {e.code}' + (f': {detail}' if detail else '') +
                  (f' (request id {rid})' if rid else '') + (f'. {hint}' if hint else '')) from None
  except urllib.error.URLError as e:
    raise OSError(str(e.reason)) from None
