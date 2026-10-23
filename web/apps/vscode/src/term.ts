// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Port 61 in a VS Code terminal (plan 03 §4, `uclog.terminal.mode: pseudoterminal`). Line mode:
 * the line being typed is shown locally and replaced by the device's echo when sent.
 */

import * as vscode from 'vscode';

export class DeviceTerminal implements vscode.Pseudoterminal {
  private writeEmitter = new vscode.EventEmitter<string>();
  private closeEmitter = new vscode.EventEmitter<number | void>();
  readonly onDidWrite = this.writeEmitter.event;
  readonly onDidClose = this.closeEmitter.event;
  private line = '';
  private decoder = new TextDecoder();
  private history: string[] = [];
  private hist = -1;
  private lastCr = false;

  constructor(
    private send: (line: string) => void,
    private opened: (on: boolean) => void,
  ) {}

  open() {
    this.writeEmitter.fire('\x1b[2mucLog device terminal (port 61). Lines are sent with Enter.\x1b[0m\r\n');
    this.opened(true);
  }

  close() {
    this.opened(false);
  }

  /** Device output: bare LF becomes CRLF. */
  output(data: Uint8Array) {
    let s = this.decoder.decode(data, { stream: true });
    s = s.replace(/\r?\n/g, (m, off: number) => (m === '\n' && off === 0 && this.lastCr ? '\n' : '\r\n'));
    this.lastCr = s.endsWith('\r');
    if (!s) return;
    // Keep the line being typed below the output.
    const typed = this.line;
    this.writeEmitter.fire(typed ? `\r\x1b[K${s}${typed}` : s);
  }

  handleInput(data: string) {
    if (data.startsWith('\x1b')) {
      // Up/down: history; other keys with escape sequences are ignored.
      if ((data !== '\x1b[A' && data !== '\x1b[B') || !this.history.length) return;
      if (data === '\x1b[A') this.hist = this.hist < 0 ? this.history.length - 1 : Math.max(0, this.hist - 1);
      else this.hist = this.hist < 0 || this.hist + 1 >= this.history.length ? -1 : this.hist + 1;
      this.line = this.hist < 0 ? '' : this.history[this.hist];
      this.writeEmitter.fire(`\r\x1b[K${this.line}`);
      return;
    }
    for (const ch of data) {
      if (ch === '\r') {
        const l = this.line;
        this.line = '';
        this.hist = -1;
        this.writeEmitter.fire('\r\x1b[K');
        if (l.trim()) this.history = [...this.history.filter((h) => h !== l), l].slice(-50);
        this.send(l);
      } else if (ch === '\x7f' || ch === '\b') {
        if (this.line) {
          this.line = this.line.slice(0, -1);
          this.writeEmitter.fire('\b \b');
        }
      } else if (ch === '\x03') {
        this.line = '';
        this.writeEmitter.fire('^C\r\n');
      } else if (ch >= ' ') {
        this.line += ch;
        this.writeEmitter.fire(ch);
      }
    }
  }

  dispose() {
    this.closeEmitter.fire();
  }
}
