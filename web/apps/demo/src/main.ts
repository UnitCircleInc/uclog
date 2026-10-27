// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

// The web app (uclog.unitcircle.ca/app/, and `vite` for development): <uc-log> with its workers as
// files. Built for the site, it also works offline (sw.js, written by vite.config.ts) and offers
// the one-file app (uclog-app.html, `vite build --mode single`).

import { UcLog } from '@uclog/ui';
import { defaultWorkers } from '@uclog/workers/default-workers';
import sampleUrl from '../../../../spec/vectors/sample.logdata?url';
import { startPage } from './page.ts';

UcLog.workers = defaultWorkers;
const el = startPage('uclog-demo', sampleUrl);

// The built app on http(s): the service worker (precached app, updates on the next load) and a
// link to the one-file app. Not in development (no sw.js) or from a file.
if (import.meta.env.PROD && location.protocol !== 'file:') {
  const bar = document.getElementById('app-bar');
  if (bar) bar.hidden = false;
  if ('serviceWorker' in navigator) {
    void navigator.serviceWorker.register('sw.js').then((reg) => {
      // A newer version installed while this page runs: offer to switch to it now.
      const offer = (w: ServiceWorker | null) => {
        if (!w || !navigator.serviceWorker.controller) return; // the first install: nothing to say
        const update = document.getElementById('app-update');
        if (!update) return;
        update.hidden = false;
        update.querySelector('button')!.onclick = () => w.postMessage('skip-waiting');
        el.dispatchEvent(new CustomEvent('uc-note', { detail: 'A new version of the app is ready: reload to use it' }));
      };
      offer(reg.waiting);
      reg.addEventListener('updatefound', () => {
        const w = reg.installing;
        w?.addEventListener('statechange', () => w.state === 'installed' && offer(w));
      });
      navigator.serviceWorker.addEventListener('controllerchange', () => location.reload());
    });
  }
}
