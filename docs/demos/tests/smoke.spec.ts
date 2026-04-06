/**
 * Smoke test: verify code-server launches with the smelt extension and the
 * demo workspace loads with working LSP diagnostics.
 */
import { test, expect } from "@playwright/test";
import {
  launchCodeServer,
  type CodeServerHandle,
} from "../helpers/code-server.js";
import {
  dismissDialogs,
  waitForWorkbench,
  openFile,
  waitForDiagnostics,
} from "../helpers/editor.js";
import { screenshot } from "../helpers/capture.js";

let server: CodeServerHandle;

test.beforeAll(async () => {
  // Use an already-running code-server if CODE_SERVER_URL is set,
  // otherwise launch one ourselves.
  if (!process.env.CODE_SERVER_URL) {
    server = await launchCodeServer();
  }
});

test.afterAll(async () => {
  server?.stop();
});

test("code-server loads the demo workspace", async ({ page }) => {
  const url = process.env.CODE_SERVER_URL ?? server.url;
  await page.goto(url, { waitUntil: "load" });
  await waitForWorkbench(page);
  await dismissDialogs(page);

  // Verify the workbench loaded
  await expect(page.locator(".monaco-workbench")).toBeVisible();

  await screenshot(page, {
    feature: "smoke",
    name: "workbench-loaded",
  });
});

test("smelt extension activates and produces diagnostics", async ({
  page,
}) => {
  const url = process.env.CODE_SERVER_URL ?? server.url;
  await page.goto(url, { waitUntil: "load" });
  await waitForWorkbench(page);
  await dismissDialogs(page);

  // Open a file with a known error — bad_ref.sql has a typo in smelt.ref()
  await openFile(page, "bad_ref.sql");

  // Wait for the LSP to produce diagnostics (squiggly underlines)
  await waitForDiagnostics(page, { timeout: 45_000 });

  // Verify at least one error decoration is present
  const errorCount = await page.locator(".squiggly-error").count();
  expect(errorCount).toBeGreaterThan(0);

  await screenshot(page, {
    feature: "smoke",
    name: "diagnostics-active",
  });
});

test("clean model has no errors", async ({ page }) => {
  const url = process.env.CODE_SERVER_URL ?? server.url;
  await page.goto(url, { waitUntil: "load" });
  await waitForWorkbench(page);
  await dismissDialogs(page);

  // Open a model that should be clean
  await openFile(page, "stg_users.sql");

  // Give the LSP time to process
  await page.waitForTimeout(5000);

  // Should have no error squigglies
  const errorCount = await page.locator(".squiggly-error").count();
  expect(errorCount).toBe(0);

  await screenshot(page, {
    feature: "smoke",
    name: "clean-model",
  });
});
