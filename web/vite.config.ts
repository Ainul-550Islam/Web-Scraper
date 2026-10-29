import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// The harvest admin API does not emit CORS headers (deliberate — it is a
// same-origin surface). In development we bridge that with a dev-proxy so the
// browser origin stays authoritative; in production the console must be served
// from the same origin as the admin address (see docs/web-client.md).
const BACKEND = process.env.HARVEST_API ?? "http://127.0.0.1:9678";

const HOSTS = process.env.VITE_ALLOWED_HOST
  ? process.env.VITE_ALLOWED_HOST.split(",").map((host) => host.trim()).filter(Boolean)
  : [".e2b.app"];

export default defineConfig({
  plugins: [react()],
  // `vite preview` serves the built dist the same way a static host must:
  // same origin for app and /api (the admin API is CORS-silent on purpose).
  preview: {
    host: "0.0.0.0",
    allowedHosts: HOSTS,
    proxy: {
      // Trailing slash on purpose: the console has a real route at `/api-keys`,
      // and a bare `/api` prefix would swallow it into the backend rewrite.
      "/api/": {
        target: BACKEND,
        changeOrigin: false,
        rewrite: (p) => p.replace(/^\/api/, ""),
      },
    },
  },
  server: {
    host: "0.0.0.0",
    // sandbox/preview hosts (e.g. *.e2b.app) must be accepted by the dev
    // server's host check; kept scoped to the proxy-style preview domain.
    allowedHosts: HOSTS,
    proxy: {
      "/api": {
        target: BACKEND,
        changeOrigin: false,
        rewrite: (p) => p.replace(/^\/api/, ""),
      },
    },
  },
  build: {
    target: "es2022",
    rollupOptions: {
      output: {
        manualChunks: {
          vendor: ["react", "react-dom"],
        },
        // Route chunks keep their page name even though every page is now a
        // directory barrel (features/<area>/<Page>/index.tsx): the emitted
        // asset list stays a human-readable route map, and the smoke test
        // asserts one chunk per route by these names.
        chunkFileNames: (info) => {
          const m = /\/src\/features\/[^/]+\/([^/]+)\/index\.tsx?$/.exec(
            info.facadeModuleId ?? "",
          );
          return "assets/" + (m ? m[1] : "[name]") + "-[hash].js";
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["src/test/setup.ts"],
    css: false,
    restoreMocks: true,
    maxWorkers: 2,
  },
});
