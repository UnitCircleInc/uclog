// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * The ucLog VS Code extension (plan 03). The logger is the web UI in a webview; devices are reached
 * through `uclog-hub`: a private hub on 127.0.0.1 for a local device, or a hub found over mDNS or
 * typed in. State lives only in `globalStorageUri`, `globalState` and SecretStorage.
 */

import * as vscode from 'vscode';
import { promises as fsp } from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { DEMO, deviceLabel, DevicesTree, pickDevice } from './devices.ts';
import { allowedUrl, hostFetch } from './fetch.ts';
import { findLocalLogdata, FsError, ImageHashes, LogdataStore, NodeFs, sharedCacheDir } from './files.ts';
import { deviceProblem, listDevices, parseHubAddress, PrivateHub, resolveHub, type Device } from './hub.ts';
import type { ConnSummary, FromWebview, WebviewInit } from './messages.ts';
import { LoggerPanel, type Target } from './panel.ts';
import { DeviceTerminal } from './term.ts';

const DEFAULT_LOGDATA_URL = 'https://fw.unitcircle.ca';
const TOKEN_KEY = 'uclog.logdataToken';
const UI_STATE = 'uclog.ui';
const EXCLUDE = '{**/node_modules/**,**/.git/**}';

interface UiState {
  history?: string[];
  colours?: Record<string, string>;
  recentHubs?: string[];
  lastHub?: string;
  layout?: string;
}

class App implements vscode.Disposable {
  private out: vscode.LogOutputChannel;
  private hubs = new Map<string, { hub: Promise<PrivateHub>; device: Device; port?: number; stopping?: Promise<void> }>();
  private panel: LoggerPanel | null = null;
  private tree = new DevicesTree();
  private status: vscode.StatusBarItem;
  private store: LogdataStore;
  private hashes = new ImageHashes();
  private fs: NodeFs | null = null;
  private filesRoot: string | null = null;
  private device: Device | null = null;
  private conn: ConnSummary = { link: 'idle', liveness: null };
  private pty: DeviceTerminal | null = null;
  private terminal: vscode.Terminal | null = null;
  private bleDevices: Device[] = [];
  private disposables: vscode.Disposable[] = [];

  constructor(private ctx: vscode.ExtensionContext) {
    this.out = vscode.window.createOutputChannel('ucLog', { log: true });
    this.store = new LogdataStore(sharedCacheDir(), [vscode.Uri.joinPath(ctx.globalStorageUri, 'logdata').fsPath]);
    this.status = vscode.window.createStatusBarItem('uclog.status', vscode.StatusBarAlignment.Left, 50);
    this.status.name = 'ucLog';
    this.updateStatus();
    this.status.show();
    this.updateFiles();
    const reg = (id: string, f: (...a: never[]) => unknown) => this.disposables.push(vscode.commands.registerCommand(id, f));
    reg('uclog.open', () => this.ensurePanel());
    reg('uclog.connect', () => this.connectPick());
    reg('uclog.connectDevice', (d: Device) => this.connectTo(d));
    reg('uclog.stopHub', (d?: Device) => this.stopHubCommand(d));
    reg('uclog.disconnect', () => this.panel?.disconnect());
    reg('uclog.loadLogdata', () => this.loadLogdata());
    reg('uclog.flashFirmware', () => this.flashFirmware());
    reg('uclog.openTerminal', () => this.openTerminal());
    reg('uclog.setToken', () => this.setToken());
    reg('uclog.clearCache', () => this.clearCache());
    reg('uclog.resetAll', () => this.resetAll());
    reg('uclog.showOutput', () => this.out.show());
    reg('uclog.refreshDevices', () => this.refresh());
    reg('uclog.scanBle', () => this.scanBle());
    reg('uclog.addHub', () => this.addHub());
    this.disposables.push(
      this.out,
      this.status,
      vscode.window.createTreeView('uclog.devices', { treeDataProvider: this.tree, showCollapseAll: false }),
      vscode.workspace.onDidChangeConfiguration((e) => {
        if (!e.affectsConfiguration('uclog')) return;
        if (e.affectsConfiguration('uclog.semihost.root')) this.updateFiles();
        this.panel?.post({ t: 'settings', settings: this.settings() });
      }),
      vscode.workspace.onDidChangeWorkspaceFolders(() => this.updateFiles()),
    );
    void this.refresh();
  }

  private cfg() {
    return vscode.workspace.getConfiguration('uclog');
  }

  // -------------------------------------------------------------------------------------------
  // Hubs
  // -------------------------------------------------------------------------------------------

  private hubBin(): Promise<string> {
    return resolveHub(this.ctx.extensionPath, this.cfg().get<string>('hubPath', ''));
  }

  /** The private hub for a local device: started once, shared by reconnects. */
  private async ensureHub(device: Device): Promise<PrivateHub> {
    const uri = device.uri;
    let e = this.hubs.get(uri);
    if (e?.stopping) await e.stopping;
    e = this.hubs.get(uri);
    if (e) {
      const h = await e.hub.catch(() => null);
      if (h && !h.exited) return h;
      this.hubs.delete(uri);
    }
    const bin = await this.hubBin();
    let lastProblem = '';
    const log = (l: string) => {
      this.out.info(`[${uri}] ${l}`);
      // The hub keeps retrying a device it cannot open: say why in the logger.
      const p = deviceProblem(l);
      if (p && p !== lastProblem && this.device?.uri === uri) this.panel?.post({ t: 'note', text: `hub: ${p}` });
      lastProblem = p ?? (/device (alive|open)/.test(l) ? '' : lastProblem);
    };
    this.out.info(`starting ${bin} for ${uri}`);
    const hub = PrivateHub.start(bin, uri, {
      baud: this.cfg().get<number>('baud', 1_000_000),
      usbFilters: this.cfg().get<string[]>('usbFilters', []),
      log,
    });
    const entry: { hub: Promise<PrivateHub>; device: Device; port?: number } = { hub, device };
    this.hubs.set(uri, entry);
    try {
      const h = await hub;
      entry.port = h.port;
      this.out.info(`hub for ${uri} listening on 127.0.0.1:${h.port}`);
      this.hubsChanged();
      h.onExit((code) => {
        this.out.info(`hub for ${uri} exited (${code})`);
        if (this.hubs.get(uri)?.hub === hub) {
          this.hubs.delete(uri);
          this.hubsChanged();
        }
      });
      return h;
    } catch (err) {
      this.hubs.delete(uri);
      this.out.error(`hub for ${uri}: ${(err as Error).message}`);
      throw err;
    }
  }

  private releaseHub(uri: string) {
    if (this.cfg().get<boolean>('keepHubRunning', true)) return;
    void this.stopHub(uri);
  }

  private stopHub(uri: string): Promise<void> {
    const e = this.hubs.get(uri);
    if (!e) return Promise.resolve();
    e.stopping ??= e.hub.then((h) => h.stop()).catch(() => {}).finally(() => {
      if (this.hubs.get(uri) === e) this.hubs.delete(uri);
      this.hubsChanged();
    });
    return e.stopping;
  }

  /** `uclog.hubsRunning` (the Stop Hub command) and the Devices view. */
  private hubsChanged() {
    const n = [...this.hubs.values()].filter((e) => !e.stopping && e.device.kind !== 'loopback').length;
    void vscode.commands.executeCommand('setContext', 'uclog.hubsRunning', n > 0);
    void this.refresh();
  }

  /** Stop a kept private hub, freeing its device for other hosts (a BLE device starts advertising
   * again). From the Devices view (`d`), or the Command Palette (pick one). */
  private async stopHubCommand(d?: Device) {
    let uri = d?.uri;
    if (!uri) {
      const running = [...this.hubs.entries()].filter(([, e]) => !e.stopping && e.device.kind !== 'loopback');
      if (!running.length) return void vscode.window.showInformationMessage('ucLog: no hubs running.');
      const pick = running.length === 1 ? { uri: running[0][0] } : await vscode.window.showQuickPick(
        running.map(([u, e]) => ({ label: deviceLabel(e.device), description: u, uri: u })),
        { title: 'ucLog: Stop Hub', placeHolder: 'The device is freed for other hosts' },
      );
      if (!pick) return;
      uri = pick.uri;
    }
    if (this.device?.uri === uri && this.conn.link !== 'idle') this.panel?.disconnect();
    await this.stopHub(uri);
    this.out.info(`hub for ${uri} stopped`);
  }

  private async stopHubs() {
    const all = [...this.hubs.values()];
    this.hubs.clear();
    await Promise.all(all.map((e) => e.hub.then((h) => h.stop()).catch(() => {})));
  }

  private privatePorts(): Set<number> {
    return new Set([...this.hubs.values()].flatMap((e) => (e.port ? [e.port] : [])));
  }

  private target(d: Device): Target {
    if (d.kind === 'hub') {
      return {
        description: d.name && d.name !== d.uri ? `hub ${d.name}` : `hub ${d.uri.replace(/^tcp:\/\//, '')}`,
        resolve: async () => parseHubAddress(d.uri),
      };
    }
    return {
      description: d.kind === 'loopback' ? 'simulated device' : `${d.name || d.uri}`,
      resolve: async (progress) => {
        progress('starting uclog-hub…');
        const h = await this.ensureHub(d);
        return { host: '127.0.0.1', port: h.port };
      },
      released: () => {
        this.releaseHub(d.uri);
        void this.refresh(); // a kept hub shows as "hub running"
      },
    };
  }

  // -------------------------------------------------------------------------------------------
  // Devices
  // -------------------------------------------------------------------------------------------

  private async list(): Promise<Device[]> {
    const bin = await this.hubBin();
    const { devices, notes } = await listDevices(bin, { usbFilters: this.cfg().get<string[]>('usbFilters', []), hubs: 1 });
    notes.forEach((n) => this.out.warn(n));
    // Our private hubs are in this machine's registry: list the device, not its hub.
    const mine = this.privatePorts();
    const out = devices.filter((d) => {
      if (d.kind !== 'hub') return true;
      try {
        const a = parseHubAddress(d.uri);
        return !(['localhost', '127.0.0.1', '::1'].includes(a.host) && mine.has(a.port));
      } catch {
        return true;
      }
    });
    // A device whose private hub is still running (keepHubRunning) is listed even when the list
    // can't see it: a BLE device connected to the hub stops advertising. Connecting reuses the hub.
    for (const [uri, e] of this.hubs) {
      if (e.stopping || e.device.kind === 'loopback') continue;
      const i = out.findIndex((d) => d.uri === uri);
      const kept = { ...(i >= 0 ? out[i] : e.device), busy: false, hubRunning: true };
      if (i >= 0) out[i] = kept;
      else out.push(kept);
    }
    return out;
  }

  private async refresh() {
    try {
      const d = await this.list();
      this.tree.set([...d, ...this.bleDevices.filter((b) => !d.some((x) => x.uri === b.uri)), DEMO]);
    } catch (e) {
      this.out.error((e as Error).message);
      this.tree.set([DEMO]);
    }
  }

  private async scanBle(): Promise<Device[]> {
    const bin = await this.hubBin();
    return vscode.window.withProgress({ location: { viewId: 'uclog.devices' }, title: 'Scanning for Bluetooth devices' }, async () => {
      const { devices, notes } = await listDevices(bin, { bleScan: 4 });
      notes.forEach((n) => this.out.warn(n));
      this.bleDevices = devices.filter((d) => d.kind === 'ble');
      if (notes.length && !this.bleDevices.length) void vscode.window.showWarningMessage(`ucLog: ${notes.join('; ')}`);
      await this.refresh();
      return this.bleDevices;
    });
  }

  private async addHub() {
    const s = await vscode.window.showInputBox({
      title: 'ucLog: Hub address',
      prompt: 'host:port of a uclog hub (TCP)',
      placeHolder: 'lab-pc.local:9000',
      validateInput: (v) => {
        try {
          parseHubAddress(v);
          return null;
        } catch (e) {
          return (e as Error).message;
        }
      },
    });
    if (!s) return;
    const a = parseHubAddress(s);
    const uri = `tcp://${a.host.includes(':') ? `[${a.host}]` : a.host}:${a.port}`;
    const st = this.uiState();
    st.recentHubs = [uri, ...(st.recentHubs ?? []).filter((u) => u !== uri)].slice(0, 10);
    st.lastHub = uri;
    await this.ctx.globalState.update(UI_STATE, st);
    await this.connectTo({ kind: 'hub', uri, name: uri, serial: '', description: 'entered', busy: null });
  }

  private async connectPick() {
    const r = await pickDevice(
      () => this.list(),
      async () => (await listDevices(await this.hubBin(), { bleScan: 4 })).devices.filter((d) => d.kind === 'ble'),
      this.uiState().recentHubs ?? [],
    );
    if (r === 'address') return this.addHub();
    if (r) await this.connectTo(r);
  }

  private async connectTo(d: Device) {
    if (d.busy && d.kind === 'hub') {
      const go = await vscode.window.showWarningMessage(`${d.name || d.uri} is in use by another client. Try anyway?`, 'Connect');
      if (!go) return;
    }
    const p = this.ensurePanel();
    this.device = d;
    this.tree.setConnected(null);
    await p.connect(this.target(d));
  }

  // -------------------------------------------------------------------------------------------
  // The panel
  // -------------------------------------------------------------------------------------------

  private ensurePanel(): LoggerPanel {
    if (this.panel) {
      this.panel.reveal();
      return this.panel;
    }
    const p = new LoggerPanel(this.ctx, {
      init: (): WebviewInit => ({ settings: this.settings(), files: this.filesRoot, terminalForward: !!this.pty }),
      call: (op, args) => this.call(op, args),
      onMessage: (m) => this.onMessage(m),
      onState: (c) => this.onState(c),
      onDispose: () => {
        if (this.panel === p) this.panel = null;
        this.onState({ link: 'idle', liveness: null });
      },
    });
    this.panel = p;
    // A private hub kept running: show what it buffered.
    const d = this.device;
    if (d && d.kind !== 'hub' && this.hubs.has(d.uri)) void p.connect(this.target(d));
    return p;
  }

  private onState(c: ConnSummary) {
    const was = this.conn.link;
    this.conn = c;
    const open = c.link === 'open';
    void vscode.commands.executeCommand('setContext', 'uclog.connected', open || c.link === 'connecting' || c.link === 'approval');
    this.tree.setConnected(open && this.device ? this.device.uri : null);
    this.updateStatus();
    if (open && was !== 'open' && this.cfg().get<string>('terminal.mode') === 'pseudoterminal' && !this.terminal) this.openTerminal(false);
  }

  private updateStatus() {
    const c = this.conn;
    const name = this.device ? this.device.name || this.device.uri : '';
    const s = this.status;
    s.backgroundColor = undefined;
    s.command = 'uclog.open';
    switch (c.link) {
      case 'open':
        if (c.liveness === 'not-responding') {
          s.text = `$(warning) ucLog: ${name} — device not responding`;
          s.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
        } else s.text = `$(pass-filled) ucLog: ${name}`;
        break;
      case 'connecting':
        s.text = `$(sync~spin) ucLog: connecting ${name}…`;
        break;
      case 'approval':
        s.text = `$(watch) ucLog: waiting for hub approval…`;
        break;
      case 'busy':
        s.text = `$(error) ucLog: hub busy`;
        s.backgroundColor = new vscode.ThemeColor('statusBarItem.errorBackground');
        s.command = 'uclog.connect';
        break;
      case 'error':
      case 'timeout':
        s.text = `$(error) ucLog: ${c.detail ?? 'connection failed'}`;
        s.backgroundColor = new vscode.ThemeColor('statusBarItem.errorBackground');
        s.command = 'uclog.connect';
        break;
      default:
        s.text = '$(plug) ucLog';
        s.command = this.panel ? 'uclog.connect' : 'uclog.open';
    }
    s.tooltip = c.link === 'idle' ? 'ucLog: open the logger or connect' : `ucLog: ${c.description ?? name}${c.detail ? ` (${c.detail})` : ''}`;
  }

  private onMessage(m: FromWebview) {
    switch (m.t) {
      case 'show-connect':
        return void this.connectPick();
      case 'open-settings':
        return void vscode.commands.executeCommand('workbench.action.openSettings', '@ext:unitcircle.uclog');
      case 'token-refused':
        return void this.tokenRefused();
      case 'open-source':
        return void this.openSource(m.file, m.line);
      case 'save':
        return void this.save(m.name, toBytes(m.data));
      case 'settings-save':
        return void this.saveSettings(m.settings as Record<string, unknown>);
      case 'terminal':
        return this.pty?.output(toBytes(m.data));
    }
  }

  // -------------------------------------------------------------------------------------------
  // Services for the webview
  // -------------------------------------------------------------------------------------------

  private async call(op: string, a: unknown[]): Promise<unknown> {
    const fs = () => {
      if (!this.fs) throw new FsError('EACCES');
      return this.fs;
    };
    switch (op) {
      case 'cache-get':
        return this.store.get(a[0] as string);
      case 'cache-put':
        return this.store.put(a[0] as string, toBytes(a[1]));
      case 'fetch': {
        const url = a[0] as string;
        const service = this.cfg().get<string>('logdata.url') || DEFAULT_LOGDATA_URL;
        if (!allowedUrl(url, service)) throw new Error(`not the logdata service: ${url}`);
        return hostFetch(url, a[1] as Record<string, string>, (await this.ctx.secrets.get(TOKEN_KEY)) || undefined);
      }
      case 'sibling': {
        const r = await findLocalLogdata(a[0] as string, await this.workspaceFiles(), this.hashes);
        return r && { ...r, name: vscode.workspace.asRelativePath(r.name) };
      }
      case 'fs-open':
        return fs().open(a[0] as string[], a[1] as never);
      case 'fs-read':
        return fs().read(a[0] as number, a[1] as number);
      case 'fs-write':
        return fs().write(a[0] as number, toBytes(a[1]));
      case 'fs-close':
        return fs().close(a[0] as number);
      case 'fs-stat':
        return fs().stat(a[0] as number);
      case 'fs-mkdir':
        return fs().mkdir(a[0] as string[]);
      case 'pick-file':
        return a[0] === 'firmware' ? this.pickFirmware() : this.pickLogdata();
    }
    throw new Error(`unknown call ${op}`);
  }

  private async workspaceFiles() {
    if (!vscode.workspace.workspaceFolders?.length) return { logdata: [], images: [] };
    const [logdata, images] = await Promise.all([
      vscode.workspace.findFiles('**/*.logdata', EXCLUDE, 500),
      vscode.workspace.findFiles('**/*.signed.bin', EXCLUDE, 100),
    ]);
    return { logdata: logdata.map((u) => u.fsPath), images: images.map((u) => u.fsPath) };
  }

  private async pickLogdata(): Promise<{ name: string; data: Uint8Array } | null> {
    const r = await vscode.window.showOpenDialog({
      title: 'ucLog: Load logdata',
      canSelectMany: false,
      defaultUri: vscode.workspace.workspaceFolders?.[0]?.uri,
      filters: { 'Logdata or ELF': ['logdata', 'elf'], 'All files': ['*'] },
    });
    if (!r?.[0]) return null;
    return { name: vscode.workspace.asRelativePath(r[0]), data: new Uint8Array(await fsp.readFile(r[0].fsPath)) };
  }

  /** Signed images in the workspace (newest first), or Browse… */
  private async pickFirmware(): Promise<{ name: string; data: Uint8Array } | null> {
    const found = vscode.workspace.workspaceFolders?.length ? await vscode.workspace.findFiles('**/*.signed.bin', EXCLUDE, 50) : [];
    const withTime = await Promise.all(found.map(async (u) => ({ u, t: (await fsp.stat(u.fsPath).catch(() => null))?.mtimeMs ?? 0 })));
    withTime.sort((x, y) => y.t - x.t);
    type It = vscode.QuickPickItem & { uri?: vscode.Uri };
    const items: It[] = withTime.map(({ u, t }) => ({
      label: `$(file-binary) ${vscode.workspace.asRelativePath(u)}`,
      description: t ? new Date(t).toLocaleString() : '',
      uri: u,
    }));
    items.push({ label: '$(folder-opened) Browse…' });
    let uri: vscode.Uri | undefined;
    if (items.length > 1) {
      const it = await vscode.window.showQuickPick(items, { title: 'ucLog: Firmware image', placeHolder: 'A signed image (.bin)' });
      if (!it) return null;
      uri = it.uri;
    }
    if (!uri) {
      const r = await vscode.window.showOpenDialog({
        title: 'ucLog: Firmware image',
        canSelectMany: false,
        defaultUri: vscode.workspace.workspaceFolders?.[0]?.uri,
        filters: { 'Signed image': ['bin'], 'All files': ['*'] },
      });
      uri = r?.[0];
    }
    if (!uri) return null;
    return { name: vscode.workspace.asRelativePath(uri), data: new Uint8Array(await fsp.readFile(uri.fsPath)) };
  }

  private async loadLogdata() {
    const f = await this.pickLogdata();
    if (!f) return;
    const p = this.ensurePanel();
    await p.whenReady();
    p.post({ t: 'load-logdata', name: f.name, data: f.data });
  }

  private async flashFirmware() {
    const f = await this.pickFirmware();
    if (!f) return;
    const p = this.ensurePanel();
    await p.whenReady();
    p.post({ t: 'fw', name: f.name, data: f.data });
  }

  /** `file:line` from a log record: a workspace file with that name (or path suffix). */
  private async openSource(file: string, line: string) {
    const n = Math.max(1, parseInt(line, 10) || 1);
    let uri: vscode.Uri | undefined;
    if (path.isAbsolute(file) && (await fsp.stat(file).catch(() => null))) uri = vscode.Uri.file(file);
    if (!uri) {
      const base = path.basename(file.replace(/\\/g, '/'));
      const found = await vscode.workspace.findFiles(`**/${base}`, EXCLUDE, 50);
      const suffix = file.replace(/\\/g, '/').replace(/^\.?\//, '');
      const best = found.filter((u) => u.fsPath.replace(/\\/g, '/').endsWith(`/${suffix}`));
      const cands = best.length ? best : found;
      if (cands.length === 1) uri = cands[0];
      else if (cands.length > 1) {
        const it = await vscode.window.showQuickPick(
          cands.map((u) => ({ label: vscode.workspace.asRelativePath(u), uri: u })),
          { title: `ucLog: ${file}:${n}` },
        );
        uri = it?.uri;
        if (!uri) return;
      }
    }
    if (!uri) {
      void vscode.window.showWarningMessage(`ucLog: ${file} is not in the workspace.`);
      return;
    }
    const doc = await vscode.workspace.openTextDocument(uri);
    const pos = new vscode.Position(Math.min(n - 1, doc.lineCount - 1), 0);
    // Not over the logger: the first group, or the second when the logger is in the first.
    const col = this.panel?.panel.viewColumn === vscode.ViewColumn.One ? vscode.ViewColumn.Two : vscode.ViewColumn.One;
    await vscode.window.showTextDocument(doc, { selection: new vscode.Range(pos, pos), preview: true, viewColumn: col });
  }

  private async save(name: string, data: Uint8Array) {
    const folder = vscode.workspace.workspaceFolders?.[0]?.uri;
    const uri = await vscode.window.showSaveDialog({
      title: 'ucLog: Export',
      defaultUri: folder ? vscode.Uri.joinPath(folder, name) : vscode.Uri.file(path.join(os.homedir(), name)),
    });
    if (!uri) return;
    await vscode.workspace.fs.writeFile(uri, data);
    this.out.info(`saved ${uri.fsPath}`);
  }

  // -------------------------------------------------------------------------------------------
  // Settings (VS Code settings + globalState for what the UI remembers)
  // -------------------------------------------------------------------------------------------

  private uiState(): UiState {
    return { ...(this.ctx.globalState.get<UiState>(UI_STATE) ?? {}) };
  }

  private settings() {
    const c = this.cfg();
    const st = this.uiState();
    return {
      logdata: { domain: c.get<string>('logdata.domain', ''), url: c.get<string>('logdata.url', '') },
      maxLines: c.get<number>('maxLogLines', 100_000),
      terminal: {
        buttons: c.get('terminal.buttons', [{ label: 'Help', cmd: 'help' }]),
        history: st.history ?? [],
        onConnect: c.get<string[]>('terminal.onConnect', []),
      },
      commands: { buttons: c.get('commands.buttons', []) },
      colours: st.colours ?? {},
      recentHubs: st.recentHubs ?? [],
      lastHub: st.lastHub ?? '',
      layout: st.layout === 'grid' ? 'grid' : 'tabs',
    };
  }

  private async saveSettings(s: Record<string, unknown>) {
    const t = s.terminal as { history?: string[] } | undefined;
    const st: UiState = {
      ...this.uiState(),
      history: t?.history,
      colours: s.colours as Record<string, string>,
      layout: s.layout as string,
    };
    await this.ctx.globalState.update(UI_STATE, st);
  }

  private updateFiles() {
    const raw = this.cfg().get<string>('semihost.root', '${workspaceFolder}/uclog-files').trim();
    const ws = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    let root: string | null = null;
    if (raw) {
      root = raw.replace(/\$\{workspaceFolder\}/g, ws ?? '\0');
      if (root.includes('\0')) root = null;
      else if (!path.isAbsolute(root)) root = ws ? path.join(ws, root) : null;
    }
    if (root === this.filesRoot) return;
    void this.fs?.closeAll();
    this.filesRoot = root;
    this.fs = root ? new NodeFs(root) : null;
    this.panel?.post({ t: 'files', files: root });
  }

  // -------------------------------------------------------------------------------------------
  // Terminal, token, data
  // -------------------------------------------------------------------------------------------

  private openTerminal(show = true) {
    if (this.terminal) {
      if (show) this.terminal.show();
      return;
    }
    const pty = new DeviceTerminal(
      (line) => this.panel?.post({ t: 'send', port: 61, data: new TextEncoder().encode(`${line}\r`) }),
      (on) => this.panel?.post({ t: 'terminal-forward', on }),
    );
    this.pty = pty;
    this.terminal = vscode.window.createTerminal({ name: 'ucLog device', pty, iconPath: new vscode.ThemeIcon('terminal') });
    const sub = vscode.window.onDidCloseTerminal((t) => {
      if (t !== this.terminal) return;
      this.terminal = null;
      this.pty = null;
      this.panel?.post({ t: 'terminal-forward', on: false });
      sub.dispose();
    });
    this.terminal.show(!show);
  }

  /** The service refused the stored token, but read without it worked. */
  private async tokenRefused() {
    if (!(await this.ctx.secrets.get(TOKEN_KEY))) return;
    const remove = 'Remove Token';
    const a = await vscode.window.showWarningMessage(
      'ucLog: the logdata service refused the stored token, but this account allows reading without one. Remove the stored token?',
      remove,
      'Keep',
    );
    if (a !== remove) return;
    await this.ctx.secrets.delete(TOKEN_KEY);
    void vscode.window.showInformationMessage('ucLog: logdata token removed.');
  }

  private async setToken() {
    const t = await vscode.window.showInputBox({
      title: 'ucLog: Logdata token',
      prompt: 'Bearer token for the logdata service (empty: remove the stored token)',
      password: true,
      ignoreFocusOut: true,
    });
    if (t === undefined) return;
    if (t.trim()) await this.ctx.secrets.store(TOKEN_KEY, t.trim());
    else await this.ctx.secrets.delete(TOKEN_KEY);
    void vscode.window.showInformationMessage(t.trim() ? 'ucLog: logdata token stored.' : 'ucLog: logdata token removed.');
  }

  private async clearCache() {
    const ok = await vscode.window.showWarningMessage(
      `Remove the ${await this.store.count()} logdata files in ${this.store.dir}? uclog, the Python library and the device builds share this folder.`,
      { modal: true },
      'Remove',
    );
    if (ok !== 'Remove') return;
    const n = await this.store.clear();
    void vscode.window.showInformationMessage(`ucLog: removed ${n} cached logdata file${n === 1 ? '' : 's'}.`);
  }

  private async resetAll() {
    const ok = await vscode.window.showWarningMessage(
      `Reset all ucLog data? This removes the token and remembered UI state (history, colours, hubs). Settings and the shared logdata cache (${this.store.dir}) are kept.`,
      { modal: true },
      'Reset',
    );
    if (ok !== 'Reset') return;
    await fsp.rm(this.ctx.globalStorageUri.fsPath, { recursive: true, force: true });
    await this.ctx.secrets.delete(TOKEN_KEY);
    for (const k of this.ctx.globalState.keys()) await this.ctx.globalState.update(k, undefined);
    this.panel?.post({ t: 'settings', settings: this.settings() });
    void vscode.window.showInformationMessage('ucLog: all data reset.');
  }

  async dispose() {
    this.panel?.panel.dispose();
    this.terminal?.dispose();
    this.disposables.forEach((d) => d.dispose());
    await this.stopHubs();
  }
}

function toBytes(v: unknown): Uint8Array {
  if (v instanceof Uint8Array) return v;
  if (v instanceof ArrayBuffer) return new Uint8Array(v);
  if (ArrayBuffer.isView(v)) return new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
  if (v && typeof v === 'object') return Uint8Array.from(Object.values(v as Record<string, number>));
  return new Uint8Array(0);
}

let app: App | null = null;

export function activate(ctx: vscode.ExtensionContext) {
  app = new App(ctx);
}

export async function deactivate() {
  await app?.dispose();
  app = null;
}
