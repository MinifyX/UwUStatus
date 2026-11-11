/// <reference types="vitest/config" />
import tailwindcss from '@tailwindcss/vite';
import { bootScript } from '@uwusuite/design';
import react from '@vitejs/plugin-react';
import { defineConfig, type Plugin } from 'vite';

// `pnpm dev` talks to a UwUStatus server on this machine: the web app and the API share one origin,
// as they do when the server serves the build. Start the server with
// UWUSTATUS_PUBLIC_URL=http://localhost:5173 so its Host/Origin checks match.
const server = process.env.UWUSTATUS_DEV_SERVER ?? 'http://127.0.0.1:8080';

/**
 * Theme, contrast and motion go onto <html> before the first paint, so a dark window never
 * flashes white (@uwusuite/design's bootScript). As a file, /boot.js, not inline, so a strict
 * CSP (script-src 'self') keeps working.
 */
function boot(): Plugin {
  const source = bootScript('uwustatus.appearance');
  return {
    name: 'uwustatus-boot',
    configureServer(dev) {
      dev.middlewares.use('/boot.js', (_request, response) => {
        response.setHeader('content-type', 'text/javascript');
        response.end(source);
      });
    },
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'boot.js', source });
    },
  };
}

export default defineConfig({
  plugins: [react(), tailwindcss(), boot()],
  server: {
    port: 5173,
    // Host and Origin stay `localhost:5173`, which the server's Host/Origin guard accepts.
    proxy: Object.fromEntries(
      ['/api', '/healthz'].map((path) => [path, { target: server, changeOrigin: false }]),
    ),
  },
  build: {
    target: 'es2022',
    // The server embeds every file; no source maps.
    sourcemap: false,
    assetsInlineLimit: 0,
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
