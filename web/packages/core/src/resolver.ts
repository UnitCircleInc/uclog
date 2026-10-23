// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Choosing logdata by the device's app_hash (protocol.md §6.4, plans README §5.3): manual files,
 * then the cache, then the logdata service, then a local build whose image matches (where the host
 * can see files: VS Code). The cache, the fetch and the local lookup are pluggable (IndexedDB in the
 * browser, a map in tests).
 */

import { hex } from './cbor.ts';
import { LogData, LogDataError } from './logdata.ts';

export const DEFAULT_LOGDATA_URL = 'https://fw.unitcircle.ca';

export interface LogdataCache {
  get(hashHex: string): Promise<Uint8Array | undefined>;
  put(hashHex: string, data: Uint8Array): Promise<void>;
}

export class MemoryCache implements LogdataCache {
  readonly map = new Map<string, Uint8Array>();
  async get(h: string) {
    return this.map.get(h);
  }
  async put(h: string, d: Uint8Array) {
    this.map.set(h, d);
  }
}

export interface ResolverConfig {
  domain?: string;
  url?: string;
  token?: string;
}

export type SourceKind = 'manual' | 'cache' | 'remote' | 'sibling' | 'fallback';

/** Finds logdata next to a local FW image whose hash is the app_hash (VS Code's workspace). */
export type SiblingLookup = (hashHex: string) => Promise<{ name: string; data: Uint8Array } | undefined>;
export interface Resolved {
  logdata: LogData[];
  source: { kind: SourceKind; name: string } | null;
  notes: string[];
}

export class HttpError extends Error {}

/** GET with an optional bearer token; undefined on 404. Problem details and the request id are
 * put into the error text (README §5.3). */
export async function fetchLogdata(url: string, token: string | undefined, f: typeof fetch = fetch): Promise<Uint8Array | undefined> {
  let r: Response;
  try {
    r = await f(url, { headers: token ? { Authorization: `Bearer ${token}` } : {}, redirect: 'follow' });
  } catch (e) {
    throw new HttpError(`${(e as Error).message} (network error or CORS)`);
  }
  if (r.status === 404) return undefined;
  if (!r.ok) {
    let detail = '';
    try {
      const body = await r.json();
      detail = body.detail || body.title || '';
    } catch {
      /* not JSON */
    }
    const rid = r.headers.get('x-request-id');
    const retry = r.status === 429 ? r.headers.get('retry-after') : null;
    throw new HttpError(
      `HTTP ${r.status}` +
        (detail ? `: ${detail}` : '') +
        (retry ? ` (retry after ${retry} s)` : '') +
        (rid ? ` (request id ${rid})` : ''),
    );
  }
  return new Uint8Array(await r.arrayBuffer());
}

export class Resolver {
  /** Manual logdata, in load order. */
  manual: LogData[] = [];

  constructor(
    public cfg: ResolverConfig = {},
    public cache: LogdataCache | null = null,
    public fetcher: typeof fetch | null = typeof fetch === 'function' ? fetch.bind(globalThis) : null,
    public sibling: SiblingLookup | null = null,
  ) {}

  async resolve(appHash: Uint8Array): Promise<Resolved> {
    const h = hex(appHash);
    const notes: string[] = [];
    const same = (a?: Uint8Array) => !a || hex(a) === h;
    const match = this.manual.filter((ld) => same(ld.appHash));
    for (const ld of this.manual) {
      if (!match.includes(ld)) notes.push(`${ld.source}: app_hash does not match the device; ignored`);
    }
    if (match.length) return { logdata: match, source: { kind: 'manual', name: match[0].source }, notes };

    if (this.cache) {
      try {
        const d = await this.cache.get(h);
        if (d) {
          const ld = new LogData(d, `cache ${h.slice(0, 16)}…`);
          ld.appHash = appHash;
          return { logdata: [ld], source: { kind: 'cache', name: `${h.slice(0, 16)}…` }, notes };
        }
      } catch (e) {
        notes.push(`cache: ${(e as Error).message}`);
      }
    }

    if (this.cfg.domain && this.fetcher) {
      const url = `${(this.cfg.url || DEFAULT_LOGDATA_URL).replace(/\/+$/, '')}/logdata/${this.cfg.domain}/${h}`;
      try {
        const d = await fetchLogdata(url, this.cfg.token, this.fetcher);
        if (d) {
          const ld = new LogData(d, url);
          ld.appHash = appHash;
          await this.cache?.put(h, d).catch((e) => notes.push(`cache write: ${(e as Error).message}`));
          return { logdata: [ld], source: { kind: 'remote', name: url }, notes };
        }
        notes.push(`${url}: not found`);
      } catch (e) {
        notes.push(`${url}: ${e instanceof LogDataError || e instanceof HttpError ? e.message : String(e)}`);
      }
    }

    if (this.sibling) {
      try {
        const f = await this.sibling(h);
        if (f) {
          const ld = LogData.fromFile(f.data, f.name);
          ld.appHash = appHash;
          return { logdata: [ld], source: { kind: 'sibling', name: f.name }, notes };
        }
      } catch (e) {
        notes.push(`local build: ${(e as Error).message}`);
      }
    }

    if (this.manual.length) {
      notes.push(`no logdata for app_hash ${h.slice(0, 16)}…; using ${this.manual[0].source}`);
      return { logdata: this.manual, source: { kind: 'fallback', name: this.manual[0].source }, notes };
    }
    return { logdata: [], source: null, notes };
  }

  /** Remember a manually loaded file under the device's app_hash (the next connection finds it). */
  async remember(appHash: Uint8Array, data: Uint8Array) {
    await this.cache?.put(hex(appHash), data);
  }
}
