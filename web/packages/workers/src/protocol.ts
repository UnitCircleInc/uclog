// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** Messages between the main thread and the two workers (plan 02 §3). */

import type { FilterSpec, FwProgress, Liveness, ResolverConfig, Row, SourceKind, ToDecoder } from '@uclog/core';

/** How the transport worker reaches the device. */
export type LinkInit =
  | { kind: 'ws'; url: string }
  | { kind: 'loopback'; opts: string }
  /** Serial/HID/BLE: the main thread owns the device and forwards bytes both ways. */
  | { kind: 'forward'; description: string };

export type LinkState = 'connecting' | 'approval' | 'open' | 'busy' | 'closed' | 'error' | 'timeout';

export type MainToTransport =
  | { t: 'init'; decoder: MessagePort }
  | { t: 'connect'; link: LinkInit; fs: FileSystemDirectoryHandle | null }
  | { t: 'rx'; data: Uint8Array }
  | { t: 'link-closed'; reason: string }
  | { t: 'send'; port: number; data: Uint8Array }
  | { t: 'stdin'; line: Uint8Array }
  | { t: 'disconnect' }
  | { t: 'fw-start'; image: Uint8Array }
  | { t: 'fw-cancel' }
  | { t: 'set-fs'; fs: FileSystemDirectoryHandle | null };

export type TransportToMain =
  | { t: 'link'; state: LinkState; detail?: string; description?: string }
  | { t: 'liveness'; state: Liveness }
  | { t: 'port'; port: number; data: Uint8Array }
  | { t: 'console'; fid: number; data: Uint8Array }
  | { t: 'stdin-wanted' }
  | { t: 'note'; text: string }
  | { t: 'tx'; data: Uint8Array }
  | { t: 'fw'; progress: FwProgress }
  | { t: 'fw-done'; port?: number; error?: string }
  | { t: 'stats'; rx: number; tx: number; dropped: number };

/** Transport → decoder over a MessageChannel; acknowledged for back-pressure. */
export type TransportToDecoder = { t: 'batch'; id: number; items: ToDecoder[] };
export type DecoderToTransport = { t: 'ack'; id: number };

export interface PlotInfo {
  name: string;
  kind: 'line' | 'heatmap' | 'spectrogram';
  version: number;
}

export interface DecodeState {
  received: number;
  stored: number;
  visible: number;
  discarded: number;
  errorCounts: Record<string, number>;
  plots: PlotInfo[];
  plotFrames: number;
  plotOrphans: number;
  plotErrors: number;
  filter: string;
}

export interface LineSeriesData {
  name: string;
  props: string;
  axis: number;
  x: Float64Array;
  y: Float64Array;
}

export type PlotData =
  | {
      kind: 'line';
      name: string;
      version: number;
      xlabel: string;
      axes: { label: string; limits: [number, number] | null }[];
      series: LineSeriesData[];
      /** Full x range of the data. */
      xmin: number;
      xmax: number;
    }
  | {
      kind: 'grid';
      name: string;
      version: number;
      spectrogram: boolean;
      rows: number;
      cols: number;
      xlabel: string;
      ylabel: string;
      values: Float64Array;
      /** Spectrogram: x of the first and last column shown. */
      x0: number;
      x1: number;
    };

export type MainToDecode =
  | { t: 'init'; transport: MessagePort }
  | { t: 'config'; resolver: ResolverConfig; maxLines: number; emitLogs: boolean }
  | { t: 'load-logdata'; id: number; name: string; data: Uint8Array }
  | { t: 'filter'; id: number; spec: FilterSpec; discardFiltered: boolean }
  | { t: 'window'; id: number; start: number; count: number }
  | { t: 'clear' }
  | { t: 'plot'; id: number; name: string; x0: number | null; x1: number | null; width: number }
  | { t: 'clear-plots' }
  | { t: 'export-logs'; id: number; only: number[] | null }
  | { t: 'export-plot'; id: number; name: string };

export type DecodeToMain =
  | { t: 'state'; state: DecodeState }
  | { t: 'device-info'; info: unknown; fields: [string, string][]; appHash: string | null }
  | { t: 'logdata'; appHash: string; source: { kind: SourceKind; name: string } | null; targets: number[] }
  | { t: 'note'; text: string }
  | { t: 'logs'; rows: Row[] }
  | { t: 'reply'; id: number; ok: boolean; error?: string; value?: unknown };
