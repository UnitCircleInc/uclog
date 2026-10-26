// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * The bundled `uclog-hub` (plan 03 §2, D19): choosing the binary for this OS and CPU, listing
 * devices, and private hubs on 127.0.0.1 for local devices. No `vscode` import (unit tested).
 */

import { spawn, execFile, type ChildProcess } from 'node:child_process';
import { promises as fsp, constants as fsc } from 'node:fs';
import * as path from 'node:path';

/** Targets in the VSIX (`bin/<platform>-<arch>/`). */
export const TARGETS = ['linux-x64', 'linux-arm64', 'linux-arm', 'darwin-arm64', 'darwin-x64', 'win32-x64', 'win32-arm64'];

export function bundledHubPath(extRoot: string, platform = process.platform, arch = process.arch): string {
  const exe = platform === 'win32' ? 'uclog-hub.exe' : 'uclog-hub';
  return path.join(extRoot, 'bin', `${platform}-${arch}`, exe);
}

/** The hub to run: `uclog.hubPath`, else the bundled one. Sets the exec bit (VSIX unzip drops it). */
export async function resolveHub(extRoot: string, override: string, platform = process.platform, arch = process.arch): Promise<string> {
  const p = override.trim() || bundledHubPath(extRoot, platform, arch);
  try {
    await fsp.access(p, fsc.F_OK);
  } catch {
    throw new Error(
      override.trim()
        ? `uclog.hubPath: ${p} does not exist`
        : `no bundled uclog-hub for ${platform}-${arch}; set uclog.hubPath to a uclog-hub executable`,
    );
  }
  if (platform !== 'win32') {
    try {
      await fsp.access(p, fsc.X_OK);
    } catch {
      await fsp.chmod(p, 0o755);
    }
  }
  return p;
}

export interface Device {
  kind: 'serial' | 'hid' | 'ble' | 'udp' | 'hub' | string;
  uri: string;
  name: string;
  serial: string;
  description: string;
  busy: boolean | null;
  /** This window's private hub for the device is running (kept after a disconnect). */
  hubRunning?: boolean;
}

export interface ListOptions {
  usbFilters?: string[];
  /** Seconds of BLE scanning (omit: none). */
  bleScan?: number;
  /** Seconds of mDNS browsing; 0 lists only this machine's hubs (omit: no hubs). */
  hubs?: number;
}

function filterArgs(f: string[] | undefined): string[] {
  return (f ?? []).flatMap((x) => ['--usb-filter', x]);
}

/** `uclog-hub list --json`. Problems the hub reports (BLE off, mDNS) come back as notes. */
export function listDevices(hub: string, o: ListOptions = {}): Promise<{ devices: Device[]; notes: string[] }> {
  const args = ['list', '--json', ...filterArgs(o.usbFilters)];
  if (o.bleScan !== undefined) args.push('--ble-scan', String(o.bleScan));
  if (o.hubs !== undefined) args.push('--hubs', String(o.hubs));
  const timeout = 15_000 + 1000 * ((o.bleScan ?? 0) + (o.hubs ?? 0));
  return new Promise((resolve, reject) => {
    execFile(hub, args, { timeout, windowsHide: true }, (err, stdout, stderr) => {
      if (err) return reject(new Error(`uclog-hub list: ${stderr.trim() || err.message}`));
      try {
        const devices = JSON.parse(stdout) as Device[];
        const notes = stderr
          .split('\n')
          .map((l) => l.trim())
          .filter(Boolean);
        resolve({ devices, notes });
      } catch (e) {
        reject(new Error(`uclog-hub list: bad output (${(e as Error).message})`));
      }
    });
  });
}

/** `tcp://host:port` → host and port (hubs from the list or typed in). */
export function parseHubAddress(s: string): { host: string; port: number } {
  let t = s.trim().replace(/^tcp:\/\//i, '');
  if (/^(wss?|https?|udp):\/\//i.test(t)) throw new Error(`use a tcp:// address or host:port (${s})`);
  t = t.replace(/\/+$/, '');
  const m = /^\[([^\]]+)\](?::(\d+))?$/.exec(t) ?? /^([^:]+)(?::(\d+))?$/.exec(t);
  if (!m || !m[1]) throw new Error(`not a hub address: ${s}`);
  const port = m[2] ? Number(m[2]) : 9000;
  if (!(port > 0 && port < 65536)) throw new Error(`bad port in ${s}`);
  return { host: m[1], port };
}

export interface HubOptions {
  baud?: number;
  usbFilters?: string[];
  log?: (line: string) => void;
  /** Wait this long for the ready line. */
  timeoutMs?: number;
}

/** A hub for one local device, listening on 127.0.0.1 only (loopback: no approval, D14). It exits
 * when the extension host does (`--exit-on-stdin-eof`). */
export class PrivateHub {
  private exitWaiters: ((code: number | null) => void)[] = [];
  exited = false;
  exitCode: number | null = null;
  private tail: string[] = [];

  private constructor(
    readonly device: string,
    private child: ChildProcess,
    readonly port: number,
  ) {}

  static start(hub: string, device: string, o: HubOptions = {}): Promise<PrivateHub> {
    const args = [
      '-d',
      device,
      '--bind',
      '127.0.0.1',
      '--port',
      '0',
      '--no-mdns',
      '--json-ready',
      '--exit-on-stdin-eof',
      '--baud',
      String(o.baud ?? 1_000_000),
      ...filterArgs(o.usbFilters),
    ];
    const child = spawn(hub, args, { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, env: { ...process.env, NO_COLOR: '1' } });
    const tail: string[] = [];
    const log = (l: string) => {
      tail.push(l);
      if (tail.length > 20) tail.shift();
      o.log?.(l);
    };
    lines(child.stderr!, (l) => log(stripAnsi(l)));
    return new Promise((resolve, reject) => {
      let done = false;
      const timer = setTimeout(() => fail(new Error(`uclog-hub did not start within ${(o.timeoutMs ?? 15_000) / 1000} s`)), o.timeoutMs ?? 15_000);
      const fail = (e: Error) => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        child.kill();
        reject(e);
      };
      child.on('error', (e) => fail(new Error(`uclog-hub: ${e.message}`)));
      child.on('exit', (code) => {
        // The hub's last stderr lines say why (device busy, not found, …).
        const why = tail.filter((l) => !/^\s*$/.test(l)).slice(-3).join('; ');
        fail(new Error(why || `uclog-hub exited (${code})`));
      });
      lines(child.stdout!, (l) => {
        if (done) return o.log?.(l);
        let port: number;
        try {
          port = (JSON.parse(l) as { port: number }).port;
        } catch {
          return o.log?.(l);
        }
        done = true;
        clearTimeout(timer);
        const h = new PrivateHub(device, child, port);
        h.tail = tail;
        child.removeAllListeners('exit');
        child.on('exit', (code) => {
          h.exited = true;
          h.exitCode = code;
          h.exitWaiters.forEach((w) => w(code));
        });
        resolve(h);
      });
    });
  }

  /** The hub's last stderr lines. */
  get lastLines(): string[] {
    return [...this.tail];
  }

  onExit(f: (code: number | null) => void) {
    if (this.exited) f(this.exitCode);
    else this.exitWaiters.push(f);
  }

  /** Close stdin (the hub exits), then kill it if it lingers. */
  stop(): Promise<void> {
    if (this.exited) return Promise.resolve();
    return new Promise((resolve) => {
      const t = setTimeout(() => this.child.kill(), 3000);
      this.onExit(() => {
        clearTimeout(t);
        resolve();
      });
      this.child.stdin?.end();
    });
  }
}

export function stripAnsi(s: string): string {
  // eslint-disable-next-line no-control-regex
  return s.replace(/\x1b\[[0-9;]*m/g, '');
}

/** A hub log line about the device (`device: serial: No such file… (retrying)`), for the UI. */
export function deviceProblem(line: string): string | null {
  const m = /\b(?:WARN|ERROR)\b.*?\bdevice: (.*)$/.exec(line);
  return m ? m[1].trim() : null;
}

function lines(stream: NodeJS.ReadableStream, f: (line: string) => void) {
  let buf = '';
  stream.setEncoding?.('utf8');
  stream.on('data', (chunk: string) => {
    buf += chunk;
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      f(buf.slice(0, i).replace(/\r$/, ''));
      buf = buf.slice(i + 1);
    }
  });
  stream.on('end', () => buf && f(buf));
}
