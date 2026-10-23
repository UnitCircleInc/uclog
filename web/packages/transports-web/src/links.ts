// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Device links that live on the main thread (Web Serial, WebHID, Web Bluetooth; plan 02 §3, §5).
 * They only move bytes: received chunks go to `onData` untouched (the transport worker does the
 * rest), and `write` sends what the worker produced.
 */

import { BLE_SERVICE, HID_IN, HID_OUT, HID_VENDOR, hidReports, SERIAL_FILTERS, stlinkSplit } from '@uclog/core';

export type CloseReason = 'closed' | 'lost' | 'error';

export interface MainLink {
  readonly kind: 'serial' | 'hid' | 'ble' | 'hub';
  readonly description: string;
  onData: (b: Uint8Array) => void;
  onClose: (reason: CloseReason, detail?: string) => void;
  /** A hub asking its operator before accepting us (set by the client before `open`). */
  onApproval?: () => void;
  /** Rejects with an error whose `state` may be `busy` or `timeout` (hubs). */
  open(): Promise<void>;
  write(b: Uint8Array): Promise<void>;
  close(): Promise<void>;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const hex4 = (n?: number) => (n === undefined ? '????' : n.toString(16).toUpperCase().padStart(4, '0'));

export function serialDescription(p: SerialPort): string {
  const i = p.getInfo();
  if (i.bluetoothServiceClassId) return `Bluetooth serial ${i.bluetoothServiceClassId}`;
  const family = i.usbVendorId === 0x0483 ? 'STLINK' : i.usbVendorId === 0x0403 ? 'FTDI' : i.usbVendorId === 0x2fe3 ? 'Zephyr CDC' : 'serial';
  return `${family} ${hex4(i.usbVendorId)}:${hex4(i.usbProductId)}`;
}

/** Web Serial: 1 000 000 baud 8N1; STLINK split writes (§4.1). */
export class SerialLink implements MainLink {
  readonly kind = 'serial';
  onData: (b: Uint8Array) => void = () => {};
  onClose: (reason: CloseReason, detail?: string) => void = () => {};
  private reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
  private writer: WritableStreamDefaultWriter<Uint8Array> | null = null;
  private closing = false;
  private stlink: boolean;

  constructor(
    readonly port: SerialPort,
    readonly baud = 1_000_000,
  ) {
    this.stlink = port.getInfo().usbVendorId === 0x0483;
  }

  get description() {
    return serialDescription(this.port);
  }

  async open() {
    await this.port.open({ baudRate: this.baud, dataBits: 8, stopBits: 1, parity: 'none', flowControl: 'none', bufferSize: 65536 });
    this.writer = this.port.writable!.getWriter();
    void this.readLoop();
  }

  private async readLoop() {
    let reason: CloseReason = 'closed';
    let detail: string | undefined;
    try {
      while (this.port.readable && !this.closing) {
        this.reader = this.port.readable.getReader();
        try {
          for (;;) {
            const { value, done } = await this.reader.read();
            if (done) break;
            if (value?.length) this.onData(value);
          }
        } catch (e) {
          // Parity/framing/break errors are recoverable: get a new reader. A lost device is not.
          const name = (e as DOMException).name;
          if (name === 'NetworkError' || name === 'NotFoundError') {
            reason = 'lost';
            detail = (e as Error).message;
            break;
          }
        } finally {
          this.reader.releaseLock();
          this.reader = null;
        }
      }
    } finally {
      if (!this.closing) {
        await this.close();
        this.onClose(reason === 'closed' ? 'lost' : reason, detail);
      }
    }
  }

  async write(b: Uint8Array) {
    if (!this.writer) throw new Error('not open');
    const parts = this.stlink ? stlinkSplit(b) : [b];
    for (let i = 0; i < parts.length; i++) {
      if (i) await sleep(10);
      await this.writer.write(parts[i]);
    }
  }

  async close() {
    this.closing = true;
    try {
      await this.reader?.cancel();
    } catch {
      /* already gone */
    }
    try {
      this.writer?.releaseLock();
    } catch {
      /* ignore */
    }
    this.writer = null;
    try {
      await this.port.close();
    } catch {
      /* already closed or lost */
    }
  }
}

/** WebHID: input report 0xA1, output report 0xA2 with 63 zero-padded data bytes (§4.3). */
export class HidLink implements MainLink {
  readonly kind = 'hid';
  onData: (b: Uint8Array) => void = () => {};
  onClose: (reason: CloseReason, detail?: string) => void = () => {};
  private onReport = (e: HIDInputReportEvent) => {
    if (e.reportId === HID_IN) this.onData(new Uint8Array(e.data.buffer, e.data.byteOffset, e.data.byteLength).slice());
  };
  private onDisconnect = (e: HIDConnectionEvent) => {
    if (e.device === this.device) {
      void this.close();
      this.onClose('lost');
    }
  };

  constructor(readonly device: HIDDevice) {}

  get description() {
    return `HID ${this.device.productName || ''} ${hex4(this.device.vendorId)}:${hex4(this.device.productId)}`.replace(/\s+/g, ' ');
  }

  async open() {
    if (!this.device.opened) await this.device.open();
    this.device.addEventListener('inputreport', this.onReport);
    navigator.hid.addEventListener('disconnect', this.onDisconnect);
  }

  async write(b: Uint8Array) {
    for (const r of hidReports(b)) await this.device.sendReport(HID_OUT, r as BufferSource);
  }

  async close() {
    this.device.removeEventListener('inputreport', this.onReport);
    navigator.hid.removeEventListener('disconnect', this.onDisconnect);
    try {
      if (this.device.opened) await this.device.close();
    } catch {
      /* ignore */
    }
  }
}

/** Web Bluetooth: notifications in, whole-frame writes of up to 512 bytes out (§4.2, D18). */
export class BleLink implements MainLink {
  readonly kind = 'ble';
  onData: (b: Uint8Array) => void = () => {};
  onClose: (reason: CloseReason, detail?: string) => void = () => {};
  private ch: BluetoothRemoteGATTCharacteristic | null = null;
  private closing = false;
  private onNotify = (e: Event) => {
    const v = (e.target as BluetoothRemoteGATTCharacteristic).value!;
    this.onData(new Uint8Array(v.buffer, v.byteOffset, v.byteLength).slice());
  };
  private onGone = () => {
    if (!this.closing) this.onClose('lost');
  };

  constructor(readonly device: BluetoothDevice) {}

  get description() {
    return `BLE ${this.device.name || this.device.id}`;
  }

  async open() {
    this.device.addEventListener('gattserverdisconnected', this.onGone);
    const server = await this.device.gatt!.connect();
    const svc = await server.getPrimaryService(BLE_SERVICE);
    this.ch = await svc.getCharacteristic(BLE_SERVICE);
    this.ch.addEventListener('characteristicvaluechanged', this.onNotify);
    await this.ch.startNotifications();
  }

  async write(b: Uint8Array) {
    if (!this.ch) throw new Error('not open');
    for (let i = 0; i < b.length; i += 512) await this.ch.writeValueWithResponse(b.subarray(i, i + 512) as BufferSource);
  }

  async close() {
    this.closing = true;
    this.ch?.removeEventListener('characteristicvaluechanged', this.onNotify);
    this.device.removeEventListener('gattserverdisconnected', this.onGone);
    try {
      this.device.gatt?.disconnect();
    } catch {
      /* ignore */
    }
  }
}

/** Browser support for each link kind. */
export function support() {
  const n = typeof navigator === 'undefined' ? ({} as Navigator) : navigator;
  return { serial: 'serial' in n, hid: 'hid' in n, ble: 'bluetooth' in n, ws: typeof WebSocket !== 'undefined' };
}

/** Ports the user granted earlier (no chooser needed). */
export async function grantedDevices(): Promise<{ serial: SerialPort[]; hid: HIDDevice[] }> {
  const s = support();
  const serial = s.serial ? await navigator.serial.getPorts() : [];
  const hid = s.hid ? (await navigator.hid.getDevices()).filter((d) => d.vendorId === HID_VENDOR) : [];
  return { serial, hid };
}

/** The Chrome chooser for each kind. */
export async function requestSerial(filters = SERIAL_FILTERS): Promise<SerialPort> {
  return navigator.serial.requestPort({ filters });
}

export async function requestHid(): Promise<HIDDevice | undefined> {
  const d = await navigator.hid.requestDevice({ filters: [{ vendorId: HID_VENDOR }] });
  return d[0];
}

export async function requestBle(): Promise<BluetoothDevice> {
  return navigator.bluetooth.requestDevice({ filters: [{ services: [BLE_SERVICE] }] });
}
