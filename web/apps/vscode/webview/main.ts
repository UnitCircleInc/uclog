// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * The logger webview (plan 03 §2–3): `<uc-log>` with the workers inlined, bytes to and from the
 * extension host's hub connection, and the host's pickers, settings and services.
 */

import TransportWorker from '@uclog/workers/transport.worker?worker&inline';
import DecodeWorker from '@uclog/workers/decode.worker?worker&inline';
import { UcLog, type Settings } from '@uclog/ui';
import type { MainLink } from '@uclog/transports-web';
import type { FromWebview, LinkState, ToWebview, WebviewInit } from '../src/messages.ts';

declare function acquireVsCodeApi(): { postMessage(m: unknown): void };
const vscode = acquireVsCodeApi();
const post = (m: FromWebview) => vscode.postMessage(m);
const init = JSON.parse(document.getElementById('uclog-init')!.textContent!) as WebviewInit;

// ---------------------------------------------------------------------------------------------
// Calls answered by the host
// ---------------------------------------------------------------------------------------------

let nextCall = 1;
const calls = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();

function call<T>(op: string, ...args: unknown[]): Promise<T> {
  const id = nextCall++;
  return new Promise<T>((resolve, reject) => {
    calls.set(id, { resolve: resolve as (v: unknown) => void, reject });
    post({ t: 'call', id, op, args });
  });
}

/** Byte arrays may arrive as plain objects or ArrayBuffers, depending on the VS Code version. */
function bytes(v: unknown): Uint8Array {
  if (v instanceof Uint8Array) return v;
  if (v instanceof ArrayBuffer) return new Uint8Array(v);
  if (ArrayBuffer.isView(v)) return new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
  if (v && typeof v === 'object') return Uint8Array.from(Object.values(v as Record<string, number>));
  return new Uint8Array(0);
}

// ---------------------------------------------------------------------------------------------
// The link: the extension host's TCP connection to a hub
// ---------------------------------------------------------------------------------------------

class ExtLink implements MainLink {
  readonly kind = 'hub' as const;
  onClose: MainLink['onClose'] = () => {};
  onApproval?: () => void;
  private queue: Uint8Array[] = [];
  private sink: ((b: Uint8Array) => void) | null = null;
  private opening: { resolve: () => void; reject: (e: Error) => void } | null = null;
  private isOpen = false;
  private closed = false;

  constructor(
    readonly id: number,
    readonly description: string,
  ) {
    links.set(id, this);
  }

  // Data that arrived before the client took the link over is delivered after it has sent its
  // `connect` to the transport worker (a microtask later).
  set onData(f: (b: Uint8Array) => void) {
    this.sink = f;
    queueMicrotask(() => {
      const q = this.queue;
      this.queue = [];
      q.forEach((b) => this.sink?.(b));
    });
  }
  get onData() {
    return this.sink ?? (() => {});
  }

  open(): Promise<void> {
    return new Promise((resolve, reject) => {
      this.opening = { resolve, reject };
      post({ t: 'open', id: this.id });
    });
  }

  state(s: LinkState, detail?: string) {
    if (s === 'approval') return this.onApproval?.();
    if (s === 'connecting') return;
    if (s === 'open') {
      this.isOpen = true;
      this.opening?.resolve();
      this.opening = null;
      return;
    }
    // busy, timeout, error, closed
    if (this.opening) {
      this.opening.reject(Object.assign(new Error(detail ?? s), { state: s }));
      this.opening = null;
    } else if (this.isOpen && !this.closed) {
      this.onClose(s === 'closed' ? 'lost' : 'error', detail);
    }
    links.delete(this.id);
  }

  rx(b: Uint8Array) {
    if (this.sink && this.queue.length === 0) this.sink(b);
    else this.queue.push(b);
  }

  async write(b: Uint8Array) {
    if (!this.closed) post({ t: 'tx', id: this.id, data: b });
  }

  async close() {
    if (this.closed) return;
    this.closed = true;
    links.delete(this.id);
    post({ t: 'close', id: this.id });
  }
}

const links = new Map<number, ExtLink>();

// ---------------------------------------------------------------------------------------------
// The element
// ---------------------------------------------------------------------------------------------

UcLog.workers = {
  transport: () => new TransportWorker({ name: 'uclog-transport' }),
  decode: () => new DecodeWorker({ name: 'uclog-decode' }),
};

UcLog.host = {
  services: {
    cache: { get: (h) => call<Uint8Array | undefined>('cache-get', h).then((v) => (v ? bytes(v) : undefined)), put: (h, d) => call('cache-put', h, d) },
    fetch: (url, headers) => call<{ status: number; headers: Record<string, string>; body: unknown }>('fetch', url, headers).then((r) => ({ ...r, body: bytes(r.body) })),
    sibling: (h) => call<{ name: string; data: unknown } | undefined>('sibling', h).then((r) => (r ? { name: r.name, data: bytes(r.data) } : undefined)),
    fs: {
      open: (p, f) => call('fs-open', p, f),
      read: (fd, n) => call('fs-read', fd, n).then(bytes),
      write: (fd, d) => call('fs-write', fd, d),
      close: (fd) => call('fs-close', fd),
      stat: (fd) => call('fs-stat', fd),
      mkdir: (p) => call('fs-mkdir', p),
    },
  },
  showConnect: () => post({ t: 'show-connect' }),
  pickFile: (kind) => call<{ name: string; data: unknown } | null>('pick-file', kind).then((r) => (r ? { name: r.name, data: bytes(r.data) } : null)),
  openSource: (file, line) => post({ t: 'open-source', file, line }),
  save: (name, data) => post({ t: 'save', name, data }),
  openSettings: () => post({ t: 'open-settings' }),
  settings: {
    load: () => init.settings,
    save: (s: Settings) => post({ t: 'settings-save', settings: s }),
  },
  files: init.files,
  filesHint: 'Set with the uclog.semihost.root setting.',
};

const el = document.createElement('uc-log');
el.setAttribute('storage-key', 'vscode');
el.setAttribute('panels', 'log,terminal,commands,plots,device,fw-update');

// Theme bridge: VS Code's body class picks light or dark; colours come from --vscode-* (page CSS).
const theme = () => el.setAttribute('theme', document.body.classList.contains('vscode-light') || document.body.classList.contains('vscode-high-contrast-light') ? 'light' : 'dark');
theme();
new MutationObserver(theme).observe(document.body, { attributes: true, attributeFilter: ['class'] });
document.body.append(el);

let terminalForward = init.terminalForward;

window.addEventListener('message', (e: MessageEvent<ToWebview>) => {
  const m = e.data;
  switch (m.t) {
    case 'connect':
      void el.connect({ kind: 'external', link: new ExtLink(m.id, m.description) }).catch(() => {});
      return;
    case 'disconnect':
      void el.disconnect();
      return;
    case 'link':
      return links.get(m.id)?.state(m.state, m.detail);
    case 'rx':
      return links.get(m.id)?.rx(bytes(m.data));
    case 'settings':
      return el.settings?.replace(m.settings);
    case 'files':
      return el.client?.setHostFiles(m.files);
    case 'load-logdata':
      void el.loadLogdata({ name: m.name, data: bytes(m.data) }).then(
        (r) => el.dispatchEvent(new CustomEvent('uc-note', { detail: `logdata ${m.name} loaded for target ${r.target}` })),
        (err: Error) => el.dispatchEvent(new CustomEvent('uc-note', { detail: `${m.name}: ${err.message}` })),
      );
      return;
    case 'fw':
      void el.fwUpdate(m.name, bytes(m.data));
      return;
    case 'terminal-forward':
      terminalForward = m.on;
      return;
    case 'send':
      return el.send(m.port, bytes(m.data));
    case 'note':
      el.dispatchEvent(new CustomEvent('uc-note', { detail: m.text }));
      return;
    case 'reply': {
      const c = calls.get(m.id);
      calls.delete(m.id);
      if (!c) return;
      if (m.ok) c.resolve(m.value);
      else c.reject(Object.assign(new Error(m.error ?? m.errno ?? 'error'), m.errno ? { errno: m.errno } : {}));
    }
  }
});

void el.ready().then(() => {
  const c = el.client;
  c.on('connection', (s) => post({ t: 'state', conn: { link: s.link, description: s.description, detail: s.detail, liveness: s.liveness } }));
  c.on('terminal', (d) => terminalForward && post({ t: 'terminal', data: d }));
  post({ t: 'ready' });
});
