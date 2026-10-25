// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * `<uc-log-view>`: the virtualised log table (plan 02 §6). Rows live in the decode worker; this
 * element asks for the window it shows. Filters: level, source, line, text search; double-click a
 * row to show only its source, right-click for more. Error records are always shown.
 */

import { html, LitElement, css, nothing } from 'lit';
import { LEVELS, NO_FILTER, plainLine, type FilterSpec, type Row } from '@uclog/core';
import type { Client, DecodeState } from '@uclog/workers';
import { base } from './styles.ts';
import { autoColour, type SettingsStore } from './settings.ts';
import type { UcHost } from './host.ts';
import { download, stamp } from './util.ts';

const ROW_H = 20;
const OVERSCAN = 20;

export class UcLogView extends LitElement {
  static properties = {
    client: { attribute: false },
    settings: { attribute: false },
    host: { attribute: false },
    dark: { type: Boolean },
    rows: { state: true },
    total: { state: true },
    filter: { state: true },
    filterError: { state: true },
    follow: { state: true },
    selected: { state: true },
    menu: { state: true },
    discard: { state: true },
  };
  declare client: Client;
  declare settings: SettingsStore;
  declare host: UcHost | null;
  declare dark: boolean;
  declare rows: Row[];
  declare total: number;
  declare filter: FilterSpec;
  declare filterError: string;
  declare follow: boolean;
  declare selected: Set<number>;
  declare menu: { x: number; y: number; row: Row } | null;
  declare discard: boolean;

  private first = 0;
  private inFlight = false;
  private again = false;
  private off: (() => void)[] = [];
  private searchTimer: ReturnType<typeof setTimeout> | undefined;
  private lastClicked: number | null = null;

  constructor() {
    super();
    this.dark = false;
    this.host = null;
    this.rows = [];
    this.total = 0;
    this.filter = { ...NO_FILTER };
    this.filterError = '';
    this.follow = true;
    this.selected = new Set();
    this.menu = null;
    this.discard = false;
  }

  static styles = [
    base,
    css`
      :host {
        display: flex;
        flex-direction: column;
        min-height: 0;
        height: 100%;
      }
      .bar {
        padding: 6px 8px;
        border-bottom: 1px solid var(--uc-border);
      }
      .bar input[type='search'] {
        width: 16em;
      }
      .bar input.small {
        width: 7em;
      }
      .scroll {
        flex: 1;
        overflow: auto;
        position: relative;
        outline: none;
      }
      .spacer {
        position: relative;
      }
      .r {
        position: absolute;
        left: 0;
        right: 0;
        height: ${ROW_H}px;
        line-height: ${ROW_H}px;
        display: grid;
        grid-template-columns: 6.5em 4.2em minmax(8em, 16em) 1fr;
        gap: 8px;
        padding: 0 8px;
        white-space: pre;
        overflow: hidden;
        cursor: default;
        font-family: var(--uc-mono, ui-monospace, Menlo, Consolas, monospace);
        font-size: 12px;
      }
      .r > span {
        overflow: hidden;
        text-overflow: ellipsis;
      }
      .r > .src {
        cursor: pointer;
      }
      .r > .src:hover {
        text-decoration: underline;
        color: var(--uc-accent);
      }
      .r.sel {
        outline: 1px solid var(--uc-accent);
        outline-offset: -1px;
        background: var(--uc-selection) !important;
      }
      .lvl-0 .lvl {
        color: var(--uc-muted);
      }
      .lvl-2 .lvl {
        color: var(--uc-warn);
      }
      .lvl-3 .lvl,
      .lvl-4 .lvl,
      .lvl-5 .lvl {
        color: var(--uc-error);
        font-weight: 600;
      }
      .r.error {
        color: var(--uc-error);
      }
      .ts {
        color: var(--uc-muted);
        text-align: right;
      }
      .empty {
        padding: 24px;
        text-align: center;
      }
      .menu {
        position: fixed;
        z-index: 10;
        background: var(--uc-panel);
        border: 1px solid var(--uc-border);
        border-radius: 8px;
        box-shadow: 0 6px 24px rgb(0 0 0 / 0.25);
        padding: 4px;
        display: flex;
        flex-direction: column;
        min-width: 14em;
      }
      .menu button {
        border: none;
        background: none;
        text-align: left;
        border-radius: 4px;
      }
      .menu label {
        padding: 3px 10px;
        display: flex;
        gap: 8px;
        align-items: center;
      }
      .menu input[type='color'] {
        padding: 0;
        width: 2.5em;
        min-height: 20px;
      }
      .follow {
        margin-left: auto;
      }
    `,
  ];

  connectedCallback() {
    super.connectedCallback();
    this.off.push(this.client.on('state', (s) => this.onState(s)));
    this.addEventListener('keydown', this.onKey);
    document.addEventListener('pointerdown', this.closeMenu, true);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.off.forEach((f) => f());
    this.off = [];
    document.removeEventListener('pointerdown', this.closeMenu, true);
  }

  private closeMenu = (e: Event) => {
    if (this.menu && !e.composedPath().some((n) => (n as HTMLElement).classList?.contains('menu'))) this.menu = null;
  };

  private get scroller(): HTMLElement | null {
    return this.renderRoot.querySelector('.scroll');
  }

  /** `DecodeState.version` of the rows last fetched. */
  private version = -1;

  private onState(s: DecodeState) {
    // Once the store is full the count stays put while rows scroll through it: compare the version.
    if (s.visible !== this.total || s.version !== this.version) {
      this.total = s.visible;
      this.version = s.version;
      if (this.follow) this.updateComplete.then(() => this.toBottom());
      this.fetch();
    }
  }

  /** Where the view last put scrollTop when following. */
  private autoTop = 0;

  private toBottom() {
    const el = this.scroller;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    this.autoTop = el.scrollTop;
  }

  /** Ask the decode worker for the rows in view (one request in flight). */
  private fetch() {
    if (this.inFlight) {
      this.again = true;
      return;
    }
    const el = this.scroller;
    const h = el?.clientHeight ?? 600;
    const top = el?.scrollTop ?? 0;
    const first = Math.max(0, Math.floor(top / ROW_H) - OVERSCAN);
    const count = Math.ceil(h / ROW_H) + OVERSCAN * 2;
    this.inFlight = true;
    void this.client
      .window(first, count)
      .then((w) => {
        this.first = w.start;
        this.rows = w.rows;
        this.total = w.total;
      })
      .finally(() => {
        this.inFlight = false;
        if (this.again) {
          this.again = false;
          this.fetch();
        }
      });
  }

  private onScroll() {
    const el = this.scroller!;
    const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - ROW_H;
    // New rows grow the table between our own scroll and its event, so "not at the bottom" alone
    // does not mean the user scrolled: only moving up from where we put it does.
    if (this.follow && !atBottom && el.scrollTop < this.autoTop - 2) this.follow = false;
    else if (!this.follow && atBottom) this.follow = true;
    this.fetch();
  }

  async applyFilter(f: FilterSpec = this.filter) {
    this.filter = f;
    try {
      await this.client.setFilter(f, this.discard);
      this.filterError = '';
      this.selected = new Set();
      this.fetch();
    } catch (e) {
      this.filterError = (e as Error).message;
    }
  }

  private setSearch(v: string) {
    clearTimeout(this.searchTimer);
    this.searchTimer = setTimeout(() => void this.applyFilter({ ...this.filter, search: v }), 200);
  }

  private onKey = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === 'c' && this.selected.size && !(e.composedPath()[0] instanceof HTMLInputElement)) {
      e.preventDefault();
      void this.copySelected();
    }
    if ((e.metaKey || e.ctrlKey) && e.key === 'f') {
      e.preventDefault();
      (this.renderRoot.querySelector('input[type=search]') as HTMLInputElement)?.focus();
    }
  };

  private async copySelected() {
    const csvRows = this.rows.filter((r) => this.selected.has(r.seq));
    const text = csvRows.map(plainLine).join('\n');
    await navigator.clipboard.writeText(text).catch(() => {});
  }

  private rowClick(e: MouseEvent, r: Row) {
    const s = new Set(e.metaKey || e.ctrlKey ? this.selected : []);
    if (e.shiftKey && this.lastClicked !== null) {
      const [a, b] = [Math.min(this.lastClicked, r.seq), Math.max(this.lastClicked, r.seq)];
      for (const x of this.rows) if (x.seq >= a && x.seq <= b) s.add(x.seq);
    } else if (s.has(r.seq)) s.delete(r.seq);
    else s.add(r.seq);
    this.lastClicked = r.seq;
    this.selected = s;
  }

  private colour(file: string) {
    return this.settings.s.colours[file] ?? 'transparent';
  }

  private async exportCsv(onlySelected: boolean) {
    const csv = await this.client.exportLogs(onlySelected ? [...this.selected] : null);
    download(`uclog-${stamp()}-logs.csv`, csv, 'text/csv');
  }

  render() {
    const f = this.filter;
    const st = this.client.decodeState;
    return html`
      <div class="bar row" part="toolbar">
        <select
          aria-label="Minimum level"
          @change=${(e: Event) => this.applyFilter({ ...f, minLevel: Number((e.target as HTMLSelectElement).value) })}
        >
          ${LEVELS.map((l, i) => html`<option value=${i} ?selected=${f.minLevel === i}>≥ ${l}</option>`)}
        </select>
        <input
          type="search"
          placeholder="Search (regex)"
          aria-label="Search"
          .value=${f.search}
          @input=${(e: Event) => this.setSearch((e.target as HTMLInputElement).value)}
        />
        <input
          class="small"
          placeholder="Source"
          aria-label="Source filter"
          .value=${f.source}
          @change=${(e: Event) => this.applyFilter({ ...f, source: (e.target as HTMLInputElement).value.trim() })}
        />
        <input
          class="small"
          placeholder="Line"
          aria-label="Line filter"
          .value=${f.line}
          @change=${(e: Event) => this.applyFilter({ ...f, line: (e.target as HTMLInputElement).value.trim() })}
        />
        ${f.minLevel || f.search || f.source || f.line
          ? html`<button @click=${() => this.applyFilter({ ...NO_FILTER })}>Clear filter</button>`
          : nothing}
        <label title="Do not store rows that the filter hides">
          <input
            type="checkbox"
            .checked=${this.discard}
            @change=${(e: Event) => {
              this.discard = (e.target as HTMLInputElement).checked;
              void this.applyFilter();
            }}
          />
          Discard filtered
        </label>
        <button @click=${() => this.client.clearLogs()}>Clear</button>
        <button @click=${() => this.exportCsv(false)}>Export CSV</button>
        ${this.selected.size ? html`<button @click=${() => this.copySelected()}>Copy ${this.selected.size}</button>` : nothing}
        <span class="follow muted">
          ${st ? `${st.visible.toLocaleString()} shown · ${st.received.toLocaleString()} received` : ''}
          ${st?.discarded ? ` · ${st.discarded.toLocaleString()} discarded` : ''}
        </span>
        ${this.follow ? nothing : html`<button @click=${() => ((this.follow = true), this.toBottom())}>↓ Follow</button>`}
        ${this.filterError ? html`<span class="err">${this.filterError}</span>` : nothing}
      </div>
      <div class="scroll" tabindex="0" part="table" @scroll=${this.onScroll}>
        ${this.total === 0
          ? html`<div class="empty muted">No logs yet.</div>`
          : html`<div class="spacer" style="height:${this.total * ROW_H}px">
              ${this.rows.map((r, i) => {
                const top = (this.first + i) * ROW_H;
                const src = r.line ? `${r.file}:${r.line}` : r.file;
                return html`<div
                  class="r lvl-${r.lvl} ${r.error ? 'error' : ''} ${this.selected.has(r.seq) ? 'sel' : ''}"
                  style="top:${top}px;background:${this.colour(r.file)}"
                  title=${r.error ? `${r.error}: ${r.text}` : r.text}
                  @click=${(e: MouseEvent) => this.rowClick(e, r)}
                  @dblclick=${() => this.applyFilter({ ...f, source: r.file })}
                  @contextmenu=${(e: MouseEvent) => {
                    e.preventDefault();
                    this.menu = { x: e.clientX, y: e.clientY, row: r };
                  }}
                >
                  <span class="ts">${r.ts.toFixed(3)}</span><span class="lvl">${r.level}</span>${this.host?.openSource && r.line
                    ? html`<span
                        class="src"
                        title="Open ${src}"
                        @click=${(e: MouseEvent) => {
                          e.stopPropagation();
                          this.host!.openSource!(r.file, r.line);
                        }}
                        >${src}</span
                      >`
                    : html`<span>${src}</span>`}<span>${r.text}</span>
                </div>`;
              })}
            </div>`}
      </div>
      ${this.menu ? this.renderMenu(this.menu) : nothing}
    `;
  }

  private renderMenu(m: { x: number; y: number; row: Row }) {
    const r = m.row;
    const done = () => (this.menu = null);
    return html`<div class="menu" style="left:${m.x}px;top:${m.y}px" role="menu">
      ${this.host?.openSource && r.line
        ? html`<button @click=${() => (done(), this.host!.openSource!(r.file, r.line))}>Open ${r.file}:${r.line}</button>`
        : nothing}
      <button @click=${() => (done(), this.applyFilter({ ...this.filter, source: r.file }))}>Show only ${r.file}</button>
      ${r.line
        ? html`<button @click=${() => (done(), this.applyFilter({ ...this.filter, source: r.file, line: r.line }))}>
            Show only ${r.file}:${r.line}
          </button>`
        : nothing}
      <button @click=${() => (done(), this.applyFilter({ ...this.filter, minLevel: r.lvl }))}>Hide below ${r.level}</button>
      <button @click=${() => (done(), this.applyFilter({ ...NO_FILTER }))}>Clear filter</button>
      <button
        @click=${() => {
          done();
          void navigator.clipboard.writeText(plainLine(r)).catch(() => {});
        }}
      >
        Copy row
      </button>
      <label
        >Colour for ${r.file}
        <input
          type="color"
          .value=${toHex(this.settings.s.colours[r.file] ?? autoColour(r.file, this.dark))}
          @change=${(e: Event) => {
            this.settings.s.colours[r.file] = (e.target as HTMLInputElement).value;
            this.settings.save();
            this.requestUpdate();
          }}
      /></label>
      ${this.settings.s.colours[r.file]
        ? html`<button
            @click=${() => {
              delete this.settings.s.colours[r.file];
              this.settings.save();
              done();
            }}
          >
            Reset colour
          </button>`
        : nothing}
    </div>`;
  }
}

/** Any CSS colour → #rrggbb (for <input type=color>). */
function toHex(c: string): string {
  if (/^#[0-9a-f]{6}$/i.test(c)) return c;
  const el = document.createElement('canvas').getContext('2d')!;
  el.fillStyle = c;
  const v = el.fillStyle;
  return /^#[0-9a-f]{6}$/i.test(v) ? v : '#ffffff';
}

customElements.define('uc-log-view', UcLogView);
