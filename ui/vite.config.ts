import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The port is tauri.conf.json's devUrl. Everything is built into the exe, so nothing is fetched from elsewhere.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5183, strictPort: true, fs: { allow: ['..'] } },
  build: { target: 'es2022', chunkSizeWarningLimit: 2000 },
});
