import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  envPrefix: ['VITE_', 'TAURI_'],
  build: { target: 'es2022', minify: 'esbuild', sourcemap: true },
  resolve: {
    conditions: mode === 'test' ? ['browser'] : []
  },
  test: {
    environment: 'jsdom'
  }
}));

