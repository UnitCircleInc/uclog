// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * The main-thread side (plan 02 §3): starts the transport and decode workers, connects them with
 * a MessageChannel, owns the Serial/HID/BLE device objects (forwarding raw bytes only), and turns
 * worker messages into events for the UI.
 */

import { decode as decodeCbor, encode, type CborValue, type FilterSpec, type FwProgress, type Liveness, type ResolverConfig, type Row } from '@uclog/core';
import {
  BleLink,
  HidLink,
  requestBle,
  requestHid,
  requestSerial,
  SerialLink,
  type MainLink,
} from '@uclog/transports-web';
import type { DecodeState, DecodeToMain, LinkInit, LinkState, MainToDecode, MainToTransport, PlotData, TransportToMain } from './protocol.ts';

export interface WorkerFactories {
  transport(): Worker;
  decode(): Worker;
}

/** Workers served from a directory (the `worker-base` attribute; no blob: URLs needed). */
export function workersAt(base: string): WorkerFactories {
  const b = base.replace(/\/+$/, '');
  return {
    transport: () => new Worker(`${b}/uclog-transport-worker.js`, { type: 'module', name: 'uclog-transport' }),
    decode: () => new Worker(`${b}/uclog-decode-worker.js`, { type: 'module', name: 'uclog-decode' }),
  };
}

/** What `connect()` takes: a URI (`ws://host:9000`, `loopback:rate=5`, `serial`, `hid`, `ble`;
 * the last three open the browser's chooser) or a device object. */
export type LinkSpec =
  | string
  | { kind: 'serial'; port: SerialPort; baud?: number }
  | { kind: 'hid'; device: HIDDevice }
  | { kind: 'ble'; device: BluetoothDevice };

export interface ConnectionState {
  link: LinkState | 'idle';
  detail?: string;
  description?: string;
  /** The spec as a display string. */
  uri?: string;
  liveness: Liveness | null;
  stdinWanted: boolean;
  rx: number;
  tx: number;
}

export interface ClientEvents {
  connection: ConnectionState;
  terminal: Uint8Array;
  command: { data: Uint8Array; value?: CborValue; text?: string };
  port: { port: number; data: Uint8Array };
  console: { fid: number; data: Uint8Array };
  note: string;
  fw: FwProgress;
  'fw-done': { port?: number; error?: string };
  state: DecodeState;
  'device-info': { info: unknown; fields: [string, string][]; appHash: string | null };
  logdata: { appHash: string; source: { kind: string; name: string } | null; targets: number[] };
  logs: Row[];
}

const te = new TextEncoder();

type DistOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;

export class Client extends EventTarget {
  private transport: Worker;
  private decodeW: Worker;
  private mainLink: MainLink | null = null;
  private writeChain: Promise<void> = Promise.resolve();
  private nextId = 1;
  private waiting = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  private fs: FileSystemDirectoryHandle | null = null;
  private lastSpec: LinkSpec | null = null;
  /** Reopen a serial/HID device when it comes back (unplug, reset). */
  autoReconnect = true;
  conn: ConnectionState = { link: 'idle', liveness: null, stdinWanted: false, rx: 0, tx: 0 };
  decodeState: DecodeState | null = null;
  deviceInfo: ClientEvents['device-info'] | null = null;
  logdata: ClientEvents['logdata'] | null = null;

  constructor(workers: WorkerFactories) {
    super();
    this.transport = workers.transport();
    this.decodeW = workers.decode();
    const ch = new MessageChannel();
    this.transport.postMessage({ t: 'init', decoder: ch.port1 } satisfies MainToTransport, [ch.port1]);
    this.decodeW.postMessage({ t: 'init', transport: ch.port2 } satisfies MainToDecode, [ch.port2]);
    this.transport.onmessage = (e: MessageEvent<TransportToMain>) => this.onTransport(e.data);
    this.decodeW.onmessage = (e: MessageEvent<DecodeToMain>) => this.onDecode(e.data);
    const err = (w: string) => (e: ErrorEvent) => this.emit('note', `${w} worker: ${e.message}`);
    this.transport.onerror = err('transport');
    this.decodeW.onerror = err('decode');
    if (typeof navigator !== 'undefined' && 'serial' in navigator) navigator.serial.addEventListener('connect', this.onSerialConnect);
    if (typeof navigator !== 'undefined' && 'hid' in navigator) navigator.hid.addEventListener('connect', this.onHidConnect);
  }

  dispose() {
    void this.disconnect();
    if ('serial' in navigator) navigator.serial.removeEventListener('connect', this.onSerialConnect);
    if ('hid' in navigator) navigator.hid.removeEventListener('connect', this.onHidConnect);
    this.transport.terminate();
    this.decodeW.terminate();
  }

  on<K extends keyof ClientEvents>(type: K, f: (v: ClientEvents[K]) => void): () => void {
    const h = (e: Event) => f((e as CustomEvent<ClientEvents[K]>).detail);
    this.addEventListener(type, h);
    return () => this.removeEventListener(type, h);
  }

  private emit<K extends keyof ClientEvents>(type: K, v: ClientEvents[K]) {
    this.dispatchEvent(new CustomEvent(type, { detail: v }));
  }

  private setConn(c: Partial<ConnectionState>) {
    this.conn = { ...this.conn, ...c };
    this.emit('connection', this.conn);
  }

  private toT(m: MainToTransport, transfer: Transferable[] = []) {
    this.transport.postMessage(m, transfer);
  }

  private toD(m: MainToDecode, transfer: Transferable[] = []) {
    this.decodeW.postMessage(m, transfer);
  }

  private request<T>(m: DistOmit<Extract<MainToDecode, { id: number }>, 'id'>, transfer: Transferable[] = []): Promise<T> {
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.waiting.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.toD({ ...m, id } as MainToDecode, transfer);
    });
  }

  private onTransport(m: TransportToMain) {
    switch (m.t) {
      case 'link': {
        const ended = m.state === 'closed' || m.state === 'busy' || m.state === 'error' || m.state === 'timeout';
        this.setConn({
          link: m.state,
          detail: m.detail,
          description: m.description ?? this.conn.description,
          liveness: ended ? null : this.conn.liveness,
          stdinWanted: ended ? false : this.conn.stdinWanted,
        });
        return;
      }
      case 'liveness':
        return this.setConn({ liveness: m.state });
      case 'port':
        if (m.port === 61) return this.emit('terminal', m.data);
        if (m.port === 0) return this.emit('command', commandReply(m.data));
        return this.emit('port', { port: m.port, data: m.data });
      case 'console':
        return this.emit('console', { fid: m.fid, data: m.data });
      case 'stdin-wanted':
        return this.setConn({ stdinWanted: true });
      case 'note':
        return this.emit('note', m.text);
      case 'tx': {
        const link = this.mainLink;
        if (!link) return;
        this.writeChain = this.writeChain
          .then(() => link.write(m.data))
          .catch((e) => this.emit('note', `write: ${(e as Error).message}`));
        return;
      }
      case 'fw':
        return this.emit('fw', m.progress);
      case 'fw-done':
        return this.emit('fw-done', { port: m.port, error: m.error });
      case 'stats':
        if (m.rx !== this.conn.rx || m.tx !== this.conn.tx) this.setConn({ rx: m.rx, tx: m.tx });
        return;
    }
  }

  private onDecode(m: DecodeToMain) {
    switch (m.t) {
      case 'state':
        this.decodeState = m.state;
        return this.emit('state', m.state);
      case 'device-info':
        this.deviceInfo = { info: m.info, fields: m.fields, appHash: m.appHash };
        return this.emit('device-info', this.deviceInfo);
      case 'logdata':
        this.logdata = { appHash: m.appHash, source: m.source, targets: m.targets };
        return this.emit('logdata', this.logdata);
      case 'note':
        return this.emit('note', m.text);
      case 'logs':
        return this.emit('logs', m.rows);
      case 'reply': {
        const w = this.waiting.get(m.id);
        this.waiting.delete(m.id);
        if (!w) return;
        if (m.ok) w.resolve(m.value);
        else w.reject(new Error(m.error));
      }
    }
  }

  // -------------------------------------------------------------------------------------------
  // Connecting
  // -------------------------------------------------------------------------------------------

  /** Connect. `serial`, `hid` and `ble` open the browser's chooser, so call them from a click. */
  async connect(spec: LinkSpec): Promise<void> {
    await this.disconnect();
    this.deviceInfo = null;
    this.logdata = null;
    let s = spec;
    if (s === 'serial') s = { kind: 'serial', port: await requestSerial() };
    else if (s === 'hid') {
      const d = await requestHid();
      if (!d) throw new Error('no HID device chosen');
      s = { kind: 'hid', device: d };
    } else if (s === 'ble') s = { kind: 'ble', device: await requestBle() };
    this.lastSpec = s;
    if (typeof s === 'string') {
      const init = parseUri(s);
      this.setConn({ link: 'connecting', uri: s, detail: undefined, description: undefined, liveness: null, rx: 0, tx: 0, stdinWanted: false });
      this.toT({ t: 'connect', link: init, fs: this.fs });
      return;
    }
    const link: MainLink =
      s.kind === 'serial' ? new SerialLink(s.port, s.baud) : s.kind === 'hid' ? new HidLink(s.device) : new BleLink(s.device);
    this.setConn({ link: 'connecting', uri: link.kind, description: link.description, detail: undefined, liveness: null, rx: 0, tx: 0, stdinWanted: false });
    try {
      await link.open();
    } catch (e) {
      this.setConn({ link: 'error', detail: `${link.description}: ${(e as Error).message}` });
      throw e;
    }
    this.mainLink = link;
    link.onData = (b) => this.toT({ t: 'rx', data: b }, [b.buffer]);
    link.onClose = (reason, detail) => {
      if (this.mainLink !== link) return;
      this.mainLink = null;
      this.toT({ t: 'link-closed', reason: detail ? `${reason}: ${detail}` : reason === 'lost' ? 'device lost' : reason });
    };
    this.toT({ t: 'connect', link: { kind: 'forward', description: link.description }, fs: this.fs });
  }

  async disconnect() {
    const l = this.mainLink;
    this.mainLink = null;
    this.toT({ t: 'disconnect' });
    if (l) await l.close();
    if (this.conn.link !== 'idle') this.setConn({ link: 'closed', detail: 'disconnected', liveness: null, stdinWanted: false });
  }

  /** Reconnect the last device or URI. */
  reconnect() {
    if (this.lastSpec) return this.connect(this.lastSpec);
  }

  private lostAndBack(same: boolean) {
    const s = this.lastSpec;
    if (!same || !s || !this.autoReconnect || this.mainLink) return;
    if (this.conn.link === 'closed' && this.conn.detail !== 'disconnected') {
      setTimeout(() => void this.connect(s).catch(() => {}), 500);
    }
  }

  private onSerialConnect = (e: Event) => {
    const s = this.lastSpec;
    this.lostAndBack(typeof s === 'object' && s?.kind === 'serial' && e.target === s.port);
  };

  private onHidConnect = (e: HIDConnectionEvent) => {
    const s = this.lastSpec;
    this.lostAndBack(typeof s === 'object' && s?.kind === 'hid' && e.device === s.device);
  };

  // -------------------------------------------------------------------------------------------
  // Device I/O
  // -------------------------------------------------------------------------------------------

  send(port: number, data: Uint8Array | string) {
    const b = typeof data === 'string' ? te.encode(data) : data;
    this.toT({ t: 'send', port, data: b });
  }

  /** Port 0: a text line (sent with `\r`) or a CBOR map (README §5.3). */
  command(cmd: string | Record<string, CborValue>) {
    this.send(0, typeof cmd === 'string' ? te.encode(cmd.endsWith('\r') ? cmd : cmd + '\r') : encode(cmd));
  }

  stdin(line: string) {
    this.toT({ t: 'stdin', line: te.encode(line) });
    this.setConn({ stdinWanted: false });
  }

  fwUpdate(image: Uint8Array) {
    const c = image.slice();
    this.toT({ t: 'fw-start', image: c }, [c.buffer]);
  }

  cancelFw() {
    this.toT({ t: 'fw-cancel' });
  }

  /** The folder for semihost file ops (null: file ops fail with EACCES). */
  setFilesDir(h: FileSystemDirectoryHandle | null) {
    this.fs = h;
    this.toT({ t: 'set-fs', fs: h });
  }

  get filesDir() {
    return this.fs;
  }

  // -------------------------------------------------------------------------------------------
  // Decoding and views
  // -------------------------------------------------------------------------------------------

  configure(resolver: ResolverConfig, maxLines: number, emitLogs = false) {
    this.toD({ t: 'config', resolver, maxLines, emitLogs });
  }

  async loadLogdata(name: string, data: Uint8Array): Promise<{ target: number }> {
    return this.request({ t: 'load-logdata', name, data });
  }

  setFilter(spec: FilterSpec, discardFiltered = false): Promise<void> {
    return this.request({ t: 'filter', spec, discardFiltered });
  }

  window(start: number, count: number): Promise<{ start: number; rows: Row[]; total: number }> {
    return this.request({ t: 'window', start, count });
  }

  clearLogs() {
    this.toD({ t: 'clear' });
  }

  clearPlots() {
    this.toD({ t: 'clear-plots' });
  }

  plot(name: string, x0: number | null, x1: number | null, width: number): Promise<PlotData> {
    return this.request({ t: 'plot', name, x0, x1, width });
  }

  exportLogs(only: number[] | null = null): Promise<string> {
    return this.request({ t: 'export-logs', only });
  }

  exportPlot(name: string): Promise<string> {
    return this.request({ t: 'export-plot', name });
  }
}

/** `ws://`, `wss://`, `loopback:` URIs. */
export function parseUri(s: string): LinkInit {
  if (/^wss?:\/\//i.test(s)) {
    const u = new URL(s);
    if (!u.port) u.port = '9000';
    return { kind: 'ws', url: u.toString().replace(/\/$/, '') };
  }
  if (s.startsWith('loopback:') || s === 'loopback') return { kind: 'loopback', opts: s.slice('loopback:'.length) };
  throw new Error(`unsupported address ${JSON.stringify(s)} (ws://host:port, wss://…, loopback:)`);
}

function commandReply(data: Uint8Array): ClientEvents['command'] {
  // CBOR maps start with major type 5; text replies are UTF-8.
  if (data.length && data[0] >> 5 === 5) {
    try {
      return { data, value: decodeCbor(data) };
    } catch {
      /* fall through */
    }
  }
  return { data, text: new TextDecoder().decode(data) };
}
