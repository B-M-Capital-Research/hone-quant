import { defineConfig, devices } from "@playwright/test";

/**
 * Smoke tests against a running hone-quant (demo data recommended):
 *   HONE_QUANT_E2E_URL=http://127.0.0.1:8090 HONE_QUANT_E2E_PASSWORD=… bun run test:e2e
 */
export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  fullyParallel: false,
  reporter: [["list"]],
  use: {
    baseURL: process.env.HONE_QUANT_E2E_URL ?? "http://127.0.0.1:8090",
    viewport: { width: 1440, height: 900 },
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } } }],
});
