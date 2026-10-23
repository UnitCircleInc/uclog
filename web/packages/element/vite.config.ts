import { defineConfig } from 'vite';

// `vite build`: ESM (uPlot in a lazy chunk). `--mode iife`: one classic script.
// `--mode workers`: the two workers as files, for `worker-base`.
export default defineConfig(({ mode }) => {
  if (mode === 'workers') {
    return {
      build: {
        outDir: 'dist/workers',
        emptyOutDir: true,
        lib: {
          entry: {
            'uclog-transport-worker': '../workers/src/transport.worker.ts',
            'uclog-decode-worker': '../workers/src/decode.worker.ts',
          },
          formats: ['es'],
        },
      },
    };
  }
  const iife = mode === 'iife';
  return {
    worker: { format: 'es' },
    build: {
      emptyOutDir: !iife,
      sourcemap: true,
      lib: {
        entry: 'src/index.ts',
        name: 'UcLog',
        formats: [iife ? 'iife' : 'es'],
        fileName: () => (iife ? 'uclog-element.iife.js' : 'uclog-element.js'),
      },
      rollupOptions: iife ? { output: { inlineDynamicImports: true } } : {},
    },
  };
});
