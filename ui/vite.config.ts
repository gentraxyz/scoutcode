import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The port is tauri.conf.json's devUrl. Everything is built into the exe, so nothing is fetched from elsewhere.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5183, strictPort: true, fs: { allow: ['..'] } },
  build: {
    target: 'es2022',
    chunkSizeWarningLimit: 2000,
    rollupOptions: {
      output: {
        // Each translation in a chunk of its own, read only when it's the picked language. Grouped, they'd share a
        // chunk with the bundler's runtime and load at startup.
        manualChunks(id: string) {
          const m = /[\\/]src[\\/]i18n[\\/]([a-z]+)\.ts$/.exec(id);
          return m && m[1] !== 'en' ? `lang-${m[1]}` : undefined;
        },
      },
    },
  },
});
