import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { defineConfig, type Plugin } from 'vite';

// `vite build`: the web app (uclog.unitcircle.ca/app/), with sw.js. `vite build --mode single`:
// the one-file app, dist-single/uclog-app.html (plan 08 §3.1).

// The service worker: every built file precached under a cache named for this build, so the app
// loads with no network after one visit; a new build replaces it on the next load (or at once,
// when the page asks: the "Reload" in main.ts).
function serviceWorker(): Plugin {
  return {
    name: 'uclog-service-worker',
    apply: 'build',
    enforce: 'post', // after index.html is in the bundle
    generateBundle(_, bundle) {
      const files = Object.keys(bundle).filter((f) => !f.endsWith('.map'));
      const hash = createHash('sha256');
      for (const f of files.sort()) {
        const o = bundle[f];
        hash.update(f).update(o.type === 'chunk' ? o.code : o.source);
      }
      const version = hash.digest('hex').slice(0, 16);
      this.emitFile({
        type: 'asset',
        fileName: 'sw.js',
        source: `// ucLog web app service worker (apps/demo/vite.config.ts): build ${version}.
const CACHE = 'uclog-app-${version}';
const FILES = ${JSON.stringify(['./', ...files])};
self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(FILES)));
});
self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys().then((ks) => Promise.all(ks.filter((k) => k.startsWith('uclog-app-') && k !== CACHE).map((k) => caches.delete(k)))));
});
self.addEventListener('message', (e) => {
  if (e.data === 'skip-waiting') self.skipWaiting();
});
self.addEventListener('fetch', (e) => {
  const r = e.request;
  if (r.method !== 'GET' || new URL(r.url).origin !== location.origin) return;
  e.respondWith(caches.open(CACHE).then((c) => c.match(r, { ignoreSearch: true }).then((hit) => hit || fetch(r))));
});
`,
      });
    },
  };
}

// The one-file app: the IIFE bundle and the page's styles in one HTML file.
function singleFile(): Plugin {
  return {
    name: 'uclog-single-file',
    apply: 'build',
    generateBundle(_, bundle) {
      const chunk = bundle['uclog-app.js'];
      if (chunk?.type !== 'chunk') throw new Error('uclog-app.js not built');
      const js = chunk.code.replace(/<\/(script)/gi, '<\\/$1');
      const page = readFileSync(join(import.meta.dirname, 'index.html'), 'utf8')
        .replace(/<!-- Shown by the built app[\s\S]*?<\/nav>\s*/, '')
        .replace(/<script type="module" src="\/src\/main.ts"><\/script>/, () => `<script>${js}</script>`);
      this.emitFile({ type: 'asset', fileName: 'uclog-app.html', source: page });
      delete bundle['uclog-app.js'];
    },
  };
}

// Both builds use @uclog/element as published (packages/element/dist: build it first). `vite`
// (development) uses its source instead, so changes to the packages show at once.
const fromSource = { alias: { '@uclog/element': join(import.meta.dirname, '../../packages/element/src/index.ts') } };

export default defineConfig(({ command, mode }) =>
  mode === 'single'
    ? {
        build: {
          outDir: 'dist-single',
          emptyOutDir: true,
          lib: { entry: 'src/single.ts', name: 'UcLogApp', formats: ['iife'], fileName: () => 'uclog-app.js' },
          rollupOptions: { output: { codeSplitting: false } },
        },
        plugins: [singleFile()],
      }
    : {
        base: './', // relative: the app works under /app/ (or anywhere)
        server: { port: 5173, fs: { allow: ['../../..'] } },
        resolve: command === 'serve' ? fromSource : {},
        build: { sourcemap: true },
        plugins: [serviceWorker()],
      },
);
