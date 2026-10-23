// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** The logdata cache in IndexedDB (plan 02 §4 resolver), shared by every page of the origin. */

import type { LogdataCache } from '@uclog/core';

const DB = 'uclog';
const STORE = 'logdata';

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore(STORE);
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

export class IdbCache implements LogdataCache {
  private db: Promise<IDBDatabase> | null = null;

  private tx(mode: IDBTransactionMode) {
    this.db ??= open();
    return this.db.then((db) => db.transaction(STORE, mode).objectStore(STORE));
  }

  async get(h: string): Promise<Uint8Array | undefined> {
    const s = await this.tx('readonly');
    return new Promise((resolve, reject) => {
      const r = s.get(h);
      r.onsuccess = () => resolve(r.result ? new Uint8Array(r.result as ArrayBuffer) : undefined);
      r.onerror = () => reject(r.error);
    });
  }

  async put(h: string, d: Uint8Array): Promise<void> {
    const s = await this.tx('readwrite');
    return new Promise((resolve, reject) => {
      const r = s.put(d.slice().buffer, h);
      r.onsuccess = () => resolve();
      r.onerror = () => reject(r.error);
    });
  }
}
