import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { fileURLToPath } from "node:url";

const backend = process.env.HONE_QUANT_BACKEND_URL ?? "http://127.0.0.1:8090";
// URL prefix the UI is built for ("" or e.g. "/quant"); must match the server's
// HONE_QUANT_BASE_PATH, which refuses to start with a UI built for another prefix.
const basePath = (process.env.HONE_QUANT_BASE_PATH ?? "").trim().replace(/\/+$/, "");
if (basePath && !/^(\/[A-Za-z0-9._-]+)+$/.test(basePath)) {
  throw new Error(`HONE_QUANT_BASE_PATH must look like /quant (got ${JSON.stringify(basePath)})`);
}

export default defineConfig({
  base: `${basePath}/`,
  plugins: [solid()],
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  server: {
    host: "127.0.0.1",
    port: Number(process.env.HONE_QUANT_WEB_PORT ?? "5173"),
    proxy: { [`${basePath}/api`]: { target: backend, changeOrigin: false } },
  },
  build: {
    target: "es2022",
    sourcemap: false,
    chunkSizeWarningLimit: 900,
    rollupOptions: {
      output: {
        manualChunks: { echarts: ["echarts/core", "echarts/charts", "echarts/components", "echarts/renderers"] },
      },
    },
  },
});
