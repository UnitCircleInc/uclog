// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** The Devices view and the Connect… quick pick (plan 03 §4): USB, BLE (on demand), hubs. */

import * as vscode from 'vscode';
import type { Device } from './hub.ts';

const ICONS: Record<string, string> = { serial: 'plug', hid: 'circuit-board', ble: 'bluetooth', udp: 'broadcast', hub: 'server', loopback: 'beaker' };
const KIND: Record<string, string> = { serial: 'Serial', hid: 'HID', ble: 'BLE', udp: 'UDP', hub: 'Hub', loopback: 'Demo' };

/** The simulated device, run by a private hub (`uclog-hub -d loopback:`). */
export const DEMO: Device = {
  kind: 'loopback',
  uri: 'loopback:rate=5',
  name: 'Simulated device',
  serial: '',
  description: 'logs, terminal and plots without hardware',
  busy: null,
};

export function deviceLabel(d: Device): string {
  return d.name || d.uri;
}

export function deviceDetail(d: Device): string {
  return [KIND[d.kind] ?? d.kind, d.serial, d.hubRunning ? 'hub running' : d.description].filter(Boolean).join(' · ');
}

export class DevicesTree implements vscode.TreeDataProvider<Device> {
  private changed = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changed.event;
  devices: Device[] = [];
  /** The device or hub the logger is connected to (uri). */
  connected: string | null = null;
  loading = false;

  set(devices: Device[]) {
    this.devices = devices;
    this.changed.fire();
  }

  setConnected(uri: string | null) {
    this.connected = uri;
    this.changed.fire();
  }

  getTreeItem(d: Device): vscode.TreeItem {
    const item = new vscode.TreeItem(deviceLabel(d), vscode.TreeItemCollapsibleState.None);
    const on = this.connected === d.uri;
    const state = on ? 'connected' : d.busy ? 'busy' : d.hubRunning ? 'hub running' : '';
    item.description = [KIND[d.kind] ?? d.kind, d.serial, state].filter(Boolean).join(' · ');
    const why = d.busy ? '\n\nIn use by another client.' : d.hubRunning ? '\n\nThe hub is still connected and buffering logs (`uclog.keepHubRunning`).' : '';
    item.tooltip = new vscode.MarkdownString(`**${deviceLabel(d)}**\n\n\`${d.uri}\`\n\n${d.description}${why}`);
    item.iconPath = new vscode.ThemeIcon(on ? 'pass-filled' : (ICONS[d.kind] ?? 'device-desktop'));
    item.contextValue = d.hubRunning ? 'device-hub' : 'device';
    item.command = { command: 'uclog.connectDevice', title: 'Connect', arguments: [d] };
    return item;
  }

  getChildren(d?: Device): Device[] {
    return d ? [] : this.devices;
  }
}

type PickItem = vscode.QuickPickItem & { device?: Device; action?: 'address' | 'ble' };

/** Devices, hubs, "Scan for Bluetooth…", "Enter hub address…" and the demo device. */
export async function pickDevice(
  list: () => Promise<Device[]>,
  scanBle: () => Promise<Device[]>,
  recentHubs: string[],
): Promise<Device | 'address' | undefined> {
  const qp = vscode.window.createQuickPick<PickItem>();
  qp.title = 'ucLog: Connect';
  qp.placeholder = 'Choose a device or hub';
  qp.matchOnDescription = true;
  qp.matchOnDetail = true;
  let devices: Device[] = [];
  const build = () => {
    const items: PickItem[] = devices.map((d) => ({
      label: `$(${ICONS[d.kind] ?? 'device-desktop'}) ${deviceLabel(d)}`,
      description: [KIND[d.kind] ?? d.kind, d.serial, d.busy ? 'busy' : d.hubRunning ? 'hub running' : ''].filter(Boolean).join(' · '),
      detail: `${d.uri} — ${d.hubRunning ? 'reconnects through the running hub, with the logs it kept' : d.description}`.replace(/ — $/, ''),
      device: d,
    }));
    const recent = recentHubs.filter((u) => !devices.some((d) => d.uri === u));
    if (recent.length) {
      items.push({ label: 'Recent hubs', kind: vscode.QuickPickItemKind.Separator });
      for (const u of recent) items.push({ label: `$(server) ${u}`, device: { kind: 'hub', uri: u, name: u, serial: '', description: 'recent', busy: null } });
    }
    items.push({ label: '', kind: vscode.QuickPickItemKind.Separator });
    items.push({ label: '$(bluetooth) Scan for Bluetooth devices…', action: 'ble', alwaysShow: true });
    items.push({ label: '$(edit) Enter hub address…', action: 'address', alwaysShow: true });
    items.push({ label: `$(beaker) ${DEMO.name}`, description: DEMO.description, device: DEMO, alwaysShow: true });
    qp.items = items;
  };
  build();
  qp.busy = true;
  void list().then(
    (d) => {
      devices = d;
      qp.busy = false;
      build();
    },
    (e: Error) => {
      qp.busy = false;
      qp.placeholder = e.message;
    },
  );
  return new Promise((resolve) => {
    qp.onDidAccept(async () => {
      const it = qp.selectedItems[0];
      if (!it) return;
      if (it.action === 'ble') {
        qp.busy = true;
        qp.placeholder = 'Scanning for Bluetooth devices…';
        try {
          const found = await scanBle();
          // A device held by a running hub isn't advertising, so the scan can't see it: keep it.
          const held = devices.filter((d) => d.kind === 'ble' && d.hubRunning);
          devices = [...devices.filter((d) => d.kind !== 'ble'), ...held, ...found.filter((f) => !held.some((h) => h.uri === f.uri))];
          qp.placeholder = found.length ? 'Choose a device or hub' : 'No Bluetooth devices found';
        } catch (e) {
          qp.placeholder = (e as Error).message;
        }
        qp.busy = false;
        build();
        return;
      }
      resolve(it.action === 'address' ? 'address' : it.device);
      qp.hide();
    });
    qp.onDidHide(() => {
      resolve(undefined);
      qp.dispose();
    });
    qp.show();
  });
}
