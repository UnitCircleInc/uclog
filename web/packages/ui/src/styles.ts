// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/** Shared styles. Colours are `--uc-*` custom properties, set by `<uc-log>` per theme. */

import { css } from 'lit';

export const base = css`
  :host {
    font-family: var(--uc-font, system-ui, -apple-system, 'Segoe UI', sans-serif);
    font-size: var(--uc-font-size, 13px);
    color: var(--uc-fg);
  }
  button,
  select,
  input,
  textarea {
    font: inherit;
    color: inherit;
  }
  button {
    background: var(--uc-button-bg);
    border: 1px solid var(--uc-border);
    border-radius: 6px;
    padding: 3px 10px;
    cursor: pointer;
    min-height: 26px;
  }
  button:hover:not(:disabled) {
    background: var(--uc-button-hover);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button.primary {
    background: var(--uc-accent);
    border-color: var(--uc-accent);
    color: var(--uc-accent-fg);
  }
  input,
  select,
  textarea {
    background: var(--uc-input-bg);
    border: 1px solid var(--uc-border);
    border-radius: 6px;
    padding: 3px 6px;
    min-height: 26px;
    box-sizing: border-box;
  }
  input:focus,
  select:focus,
  textarea:focus,
  button:focus-visible {
    outline: 2px solid var(--uc-accent);
    outline-offset: -1px;
  }
  label:has(> input[type='checkbox']) {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  input[type='checkbox'] {
    min-height: 0;
    margin: 0;
  }
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .muted {
    color: var(--uc-muted);
  }
  .err {
    color: var(--uc-error);
  }
  .ok {
    color: var(--uc-ok);
  }
  .mono {
    font-family: var(--uc-mono, ui-monospace, 'SF Mono', Menlo, Consolas, monospace);
  }
  h3 {
    font-size: 1em;
    margin: 12px 0 6px;
  }
`;
