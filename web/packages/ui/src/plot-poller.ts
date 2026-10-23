// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

import type { Client, PlotData } from '@uclog/workers';
import { download, safeName, stamp } from './util.ts';

/** Polls the decode worker for one plot's data when its version changes (one request at a time,
 * at most once per animation frame). */
export class PlotPoller {
  private version = -1;
  private inFlight = false;
  private queued = false;
  private off: () => void;
  force = false;

  constructor(
    private client: Client,
    private name: () => string,
    private request: () => Promise<PlotData>,
    private apply: (d: PlotData) => void,
  ) {
    this.off = client.on('state', (s) => {
      const p = s.plots.find((x) => x.name === this.name());
      if (p && p.version !== this.version) this.kick();
    });
  }

  kick(force = false) {
    if (force) this.force = true;
    if (this.inFlight) {
      this.queued = true;
      return;
    }
    this.inFlight = true;
    requestAnimationFrame(() => {
      this.request()
        .then((d) => {
          this.version = d.version;
          this.force = false;
          this.apply(d);
        })
        .catch(() => {})
        .finally(() => {
          this.inFlight = false;
          if (this.queued) {
            this.queued = false;
            this.kick();
          }
        });
    });
  }

  dispose() {
    this.off();
  }
}

export async function exportPlot(client: Client, name: string) {
  download(`uclog-${stamp()}-plot-${safeName(name)}.csv`, await client.exportPlot(name), 'text/csv');
}

