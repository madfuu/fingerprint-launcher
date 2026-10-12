/// <reference types="vitest/config" />
import { fileURLToPath, URL } from 'node:url';

import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// Set by the Tauri CLI when it drives `vite` (dev / build).
const tauriPlatform = process.env.TAURI_ENV_PLATFORM;
const tauriDebug = !!process.env.TAURI_ENV_DEBUG;
const devHost = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  // Keep Rust compiler output visible in the terminal.
  clearScreen: false,
  // Expose TAURI_ENV_* to the frontend alongside VITE_*.
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  server: {
    port: 5173,
    // Tauri's devUrl points at a fixed port; fail instead of silently moving.
    strictPort: true,
    host: devHost || false,
    hmr: devHost ? { protocol: 'ws', host: devHost, port: 5174 } : undefined,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    // WebView2 (Chromium) on Windows; WebKit on Linux / macOS.
    target: tauriPlatform === 'windows' ? 'chrome105' : 'safari13',
    minify: tauriDebug ? false : 'esbuild',
    sourcemap: tauriDebug,
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
