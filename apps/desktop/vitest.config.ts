import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import path from 'path';

export default defineConfig({
  plugins: [react()],
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: './src/__tests__/setup.ts',
    css: true,
    // Unit tests live under src/ and use *.test.*; Playwright e2e specs (e2e/*.spec.ts)
    // must not be picked up by vitest.
    include: ['src/**/*.test.{ts,tsx}'],
  },
  resolve: {
    alias: [
      { find: '@tauri-apps/api/core', replacement: path.resolve(__dirname, '__mocks__/tauri.ts') },
      { find: '@tauri-apps/api/window', replacement: path.resolve(__dirname, '__mocks__/tauri.ts') },
    ],
  },
});
