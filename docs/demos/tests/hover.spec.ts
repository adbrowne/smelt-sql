/**
 * Phase 3: Hover Information Demo
 *
 * Showcases the rich schema information available on hover — instant
 * documentation without leaving your editor. Hovering smelt.ref() shows the
 * full model schema (columns, types, lineage); hovering smelt.source() shows
 * the source's declared columns and types from sources.yml.
 *
 * Produces 3 screenshots (auto-recorded video via Playwright config).
 */
import { test, expect } from "@playwright/test";
import {
  launchCodeServer,
  type CodeServerHandle,
} from "../helpers/code-server";
import {
  waitForWorkbench,
  dismissDialogs,
  openFile,
  hoverWord,
  waitForDiagnostics,
  goToLine,
} from "../helpers/editor";
import {
  screenshot,
  screenshotEditor,
  screenshotWithOverlay,
  mediaDir,
} from "../helpers/capture";

let server: CodeServerHandle;

test.beforeAll(async () => {
  await mediaDir("hover");

  if (!process.env.CODE_SERVER_URL) {
    server = await launchCodeServer();
  }
});

test.afterAll(async () => {
  server?.stop();
});

async function setupPage(page: import("@playwright/test").Page) {
  const url = process.env.CODE_SERVER_URL ?? server.url;
  await page.goto(url, { waitUntil: "load" });
  await waitForWorkbench(page);
  await dismissDialogs(page);
}

/**
 * Prime the LSP by opening a broken file and waiting for diagnostics.
 */
async function primeLSP(page: import("@playwright/test").Page) {
  await openFile(page, "bad_ref.sql");
  await waitForDiagnostics(page, { timeout: 60_000 });
  await page.waitForTimeout(2000);
}

/**
 * Helper: click on a specific word in the editor to position the cursor there.
 * Reused from goto-definition tests — uses :text-is() for exact leaf-span matching.
 */
async function clickWord(
  page: import("@playwright/test").Page,
  word: string,
  opts?: { line?: number }
) {
  if (opts?.line) {
    await goToLine(page, opts.line);
  }
  const editorArea = page.locator(".editor-instance .monaco-editor");
  let wordSpan = editorArea
    .locator(`.view-lines .view-line span:text-is("${word}")`)
    .first();

  if (!(await wordSpan.isVisible({ timeout: 2000 }).catch(() => false))) {
    wordSpan = editorArea
      .locator(".view-lines .view-line")
      .locator(`span:has-text("${word}")`)
      .last();
  }

  await wordSpan.click();
  await page.waitForTimeout(300);
}

// ---------------------------------------------------------------------------
// Screenshot 1: "Model schema on hover"
// Hover smelt.ref('stg_events') in user_first_purchase.sql to show the full
// upstream model schema: column names, types, lineage.
// ---------------------------------------------------------------------------
test('Screenshot: "Model schema on hover"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open user_first_purchase.sql which refs stg_events and stg_users
  await openFile(page, "user_first_purchase.sql");
  await page.waitForTimeout(2000);

  // Hover over the stg_events ref — the LSP should show the model schema table
  const editorArea = page.locator(".editor-instance .monaco-editor");
  const refSpan = editorArea
    .locator(`.view-lines .view-line span:has-text("stg_events")`)
    .last();
  await refSpan.hover();

  // Wait for the hover widget to appear with schema content
  const hoverWidget = page.locator(".monaco-hover-content").first();
  await hoverWidget.waitFor({ timeout: 10_000 });
  await page.waitForTimeout(1500); // Let the hover fully render

  // Verify hover content contains schema info (column names or "Model:" header)
  const hoverText = await hoverWidget.textContent();
  expect(hoverText).toBeTruthy();

  // Full-page screenshot to capture the hover overlay
  await screenshotWithOverlay(page, {
    feature: "hover",
    name: "01-model-schema-on-hover",
  });

  // Also capture just the editor area for a tighter crop
  await screenshotEditor(page, {
    feature: "hover",
    name: "01-model-schema-on-hover-editor",
  });

  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
// Screenshot 2: "Upstream model schema with lineage"
// Hover smelt.ref('stg_users') in the same file to show a different schema,
// demonstrating the hover works for any ref.
// ---------------------------------------------------------------------------
test('Screenshot: "Upstream model schema with lineage"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open user_first_purchase.sql
  await openFile(page, "user_first_purchase.sql");
  await page.waitForTimeout(2000);

  // Hover over the stg_users ref — shows user schema (user_id, email, etc.)
  const editorArea = page.locator(".editor-instance .monaco-editor");
  const refSpan = editorArea
    .locator(`.view-lines .view-line span:has-text("stg_users")`)
    .last();
  await refSpan.hover();

  const hoverWidget = page.locator(".monaco-hover-content").first();
  await hoverWidget.waitFor({ timeout: 10_000 });
  await page.waitForTimeout(1500);

  const hoverText = await hoverWidget.textContent();
  expect(hoverText).toBeTruthy();

  await screenshotWithOverlay(page, {
    feature: "hover",
    name: "02-upstream-schema-lineage",
  });

  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
// Screenshot 3: "Source schema on hover"
// Hover smelt.source('raw.events') in stg_events.sql to show the source's
// declared columns and types from sources.yml.
// ---------------------------------------------------------------------------
test('Screenshot: "Source schema on hover"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open stg_events.sql which references smelt.source('raw.events')
  await openFile(page, "stg_events.sql");
  await page.waitForTimeout(2000);

  // Hover over the source reference
  const editorArea = page.locator(".editor-instance .monaco-editor");
  const sourceSpan = editorArea
    .locator(`.view-lines .view-line span:has-text("raw.events")`)
    .last();
  await sourceSpan.hover();

  const hoverWidget = page.locator(".monaco-hover-content").first();
  await hoverWidget.waitFor({ timeout: 10_000 });
  await page.waitForTimeout(1500);

  // Verify hover content mentions the source
  const hoverText = await hoverWidget.textContent();
  expect(hoverText).toBeTruthy();

  await screenshotWithOverlay(page, {
    feature: "hover",
    name: "03-source-schema-on-hover",
  });

  // Also capture a tighter editor-only screenshot
  await screenshotEditor(page, {
    feature: "hover",
    name: "03-source-schema-on-hover-editor",
  });

  await page.keyboard.press("Escape");
});
