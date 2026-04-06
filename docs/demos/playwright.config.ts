import { defineConfig } from "@playwright/test";

/**
 * Playwright configuration for smelt LSP demos.
 *
 * Expects code-server to be running on CODE_SERVER_URL (default http://localhost:18080).
 * Launch it with: bash scripts/start-code-server.sh
 */
export default defineConfig({
  testDir: "./tests",
  timeout: 120_000, // 2 minutes per test — LSP init can be slow
  retries: 0,
  workers: 1, // Serial — we share one code-server instance
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL: process.env.CODE_SERVER_URL ?? "http://localhost:18080",
    viewport: { width: 1280, height: 720 },
    video: "on",
    screenshot: "on",
    trace: "retain-on-failure",
    // Give code-server pages extra time to load
    navigationTimeout: 30_000,
    actionTimeout: 15_000,
  },
  outputDir: "./test-results",
});
