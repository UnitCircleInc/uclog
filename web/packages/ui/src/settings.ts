// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings (plan 02 §6): `localStorage`, namespaced by the element's `storage-key`. The logdata
 * token is kept for the session only, unless the user chooses to remember it (IndexedDB).
 */

import { encode, type CborValue } from '@uclog/core';
import type { SettingsBackend } from './host.ts';

export interface Button {
  label: string;
  /** A text line (sent with `\r`), or a map sent as CBOR whose string values may contain
   * `$name` placeholders that are asked for. */
  cmd: string | Record<string, unknown>;
}

export interface Settings {
  logdata: { domain: string; url: string };
  maxLines: number;
  terminal: { buttons: Button[]; history: string[]; onConnect: string[] };
  commands: { buttons: Button[] };
  /** Background colour per log source (file name). */
  colours: Record<string, string>;
  recentHubs: string[];
  lastHub: string;
  layout: 'tabs' | 'grid';
}

export const DEFAULT_SETTINGS: Settings = {
  logdata: { domain: '', url: '' },
  maxLines: 100_000,
  terminal: { buttons: [{ label: 'Help', cmd: 'help' }], history: [], onConnect: [] },
  commands: { buttons: [] },
  colours: {},
  recentHubs: [],
  lastHub: 'ws://localhost:9000',
  layout: 'tabs',
};

const HISTORY_MAX = 200;

function merge<T>(base: T, v: unknown): T {
  if (typeof base !== 'object' || base === null || Array.isArray(base)) return (v === undefined ? base : v) as T;
  const out: Record<string, unknown> = { ...(base as Record<string, unknown>) };
  if (v && typeof v === 'object' && !Array.isArray(v)) {
    for (const [k, x] of Object.entries(v)) out[k] = k in out ? merge(out[k], x) : x;
  }
  return out as T;
}

export class SettingsStore extends EventTarget {
  s: Settings;
  /** The logdata token (not in localStorage). */
  token = '';
  rememberToken = false;

  constructor(
    readonly key = 'uclog',
    /** Settings kept by the host (VS Code) instead of localStorage; the token stays with the host. */
    readonly backend: SettingsBackend | null = null,
  ) {
    super();
    let raw: unknown;
    try {
      raw = backend ? backend.load() : JSON.parse(localStorage.getItem(`${key}:settings`) || 'null');
    } catch {
      raw = null;
    }
    this.s = merge(structuredClone(DEFAULT_SETTINGS), raw);
    if (backend) return;
    try {
      this.token = sessionStorage.getItem(`${key}:token`) || '';
    } catch {
      /* storage blocked */
    }
    void loadToken(key).then((t) => {
      if (t && !this.token) {
        this.token = t;
        this.rememberToken = true;
        this.changed();
      }
    });
  }

  save() {
    try {
      if (this.backend) this.backend.save(structuredClone(this.s));
      else localStorage.setItem(`${this.key}:settings`, JSON.stringify(this.s));
    } catch {
      /* storage blocked or full */
    }
    this.changed();
  }

  /** New settings from the host (its settings changed). */
  replace(raw: unknown) {
    this.s = merge(structuredClone(DEFAULT_SETTINGS), raw);
    this.changed();
  }

  setToken(t: string, remember: boolean) {
    this.token = t;
    this.rememberToken = remember;
    try {
      if (t) sessionStorage.setItem(`${this.key}:token`, t);
      else sessionStorage.removeItem(`${this.key}:token`);
    } catch {
      /* ignore */
    }
    void storeToken(this.key, remember ? t : '');
    this.changed();
  }

  pushHistory(line: string) {
    const h = this.s.terminal.history.filter((x) => x !== line);
    h.push(line);
    this.s.terminal.history = h.slice(-HISTORY_MAX);
    this.save();
  }

  addHub(url: string) {
    this.s.lastHub = url;
    this.s.recentHubs = [url, ...this.s.recentHubs.filter((u) => u !== url)].slice(0, 10);
    this.save();
  }

  private changed() {
    this.dispatchEvent(new Event('change'));
  }
}

// The remembered token lives in IndexedDB ("uclog-settings" database).
function db(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open('uclog-settings', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('kv');
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

async function loadToken(key: string): Promise<string> {
  try {
    const d = await db();
    return await new Promise((resolve) => {
      const r = d.transaction('kv').objectStore('kv').get(`${key}:token`);
      r.onsuccess = () => resolve((r.result as string) || '');
      r.onerror = () => resolve('');
    });
  } catch {
    return '';
  }
}

async function storeToken(key: string, t: string) {
  try {
    const d = await db();
    const s = d.transaction('kv', 'readwrite').objectStore('kv');
    if (t) s.put(t, `${key}:token`);
    else s.delete(`${key}:token`);
  } catch {
    /* ignore */
  }
}

// ---------------------------------------------------------------------------------------------
// Buttons (the same schema as the native TUI/GUI)
// ---------------------------------------------------------------------------------------------

const ARG = /\$\w+/g;

/** `$name` placeholders in a button's map values: [key, placeholder]. */
export function buttonArgs(b: Button): [string, string][] {
  const out: [string, string][] = [];
  if (typeof b.cmd === 'object' && b.cmd) {
    for (const [k, v] of Object.entries(b.cmd)) {
      if (typeof v === 'string') for (const m of v.matchAll(ARG)) out.push([k, m[0]]);
    }
  }
  return out;
}

function jsonToCbor(v: unknown): CborValue {
  if (Array.isArray(v)) return v.map(jsonToCbor);
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, jsonToCbor(x)]));
  return v as CborValue;
}

/** The bytes a button sends on port 0: a text line, or CBOR with the placeholders replaced (a value
 * that is a single placeholder becomes a number when the answer parses as one). */
export function buttonPayload(b: Button, answers: Record<string, string> = {}): Uint8Array {
  if (typeof b.cmd === 'string') return new TextEncoder().encode(`${b.cmd}\r`);
  const m: Record<string, unknown> = { ...b.cmd };
  for (const [k, v] of Object.entries(b.cmd)) {
    if (typeof v !== 'string') continue;
    const args = [...v.matchAll(ARG)].map((x) => x[0]);
    if (!args.length) continue;
    const val = (a: string) => answers[`${k} ${a}`] ?? '';
    if (args.length === 1 && v === args[0]) {
      const a = val(args[0]).trim();
      m[k] = a !== '' && /^-?\d+$/.test(a) ? parseInt(a, 10) : a !== '' && !Number.isNaN(Number(a)) ? Number(a) : val(args[0]);
    } else {
      let t = v;
      for (const a of args) t = t.replace(a, val(a));
      m[k] = t;
    }
  }
  return encode(jsonToCbor(m));
}

/** A stable pastel colour for a log source. */
export function autoColour(source: string, dark: boolean): string {
  let h = 0;
  for (const c of source) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return dark ? `hsl(${h % 360} 35% 22%)` : `hsl(${h % 360} 70% 93%)`;
}
