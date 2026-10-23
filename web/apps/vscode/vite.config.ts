import { defineConfig } from 'vite';

// `vite build --mode host`: the extension host bundle (Node, CommonJS, `vscode` external).
// `vite build`: the webview (ES module, workers inlined, uPlot in a lazy chunk).
export default defineConfig(({ mode }) => {
  if (mode === 'host') {
    return {
      build: {
        ssr: 'src/extension.ts',
        outDir: 'dist',
        emptyOutDir: false,
        target: 'node20',
        sourcemap: true,
        rollupOptions: {
          external: ['vscode'],
          output: { format: 'cjs', entryFileNames: 'extension.cjs' },
        },
      },
      ssr: { noExternal: true },
    };
  }
  return {
    worker: { format: 'es' },
    build: {
      outDir: 'dist/webview',
      emptyOutDir: true,
      sourcemap: true,
      lib: { entry: 'webview/main.ts', formats: ['es'], fileName: () => 'webview.js' },
      rollupOptions: { output: { chunkFileNames: '[name]-[hash].js' } },
    },
  };
});
