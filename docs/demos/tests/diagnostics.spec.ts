/**
 * Phase 1: Real-Time Diagnostics Demo
 *
 * Showcases how smelt catches errors as you type — the most immediately
 * compelling LSP feature. Produces 3 screenshots + 1 video.
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
  waitForLSPReady,
  typeText,
} from "../helpers/editor";
import {
  screenshot,
  screenshotEditor,
  screenshotWithOverlay,
  mediaDir,
} from "../helpers/capture";

let server: CodeServerHandle;

test.beforeAll(async () => {
  // Ensure media output directory exists
  await mediaDir("diagnostics");

  if (!process.env.CODE_SERVER_URL) {
    server = await launchCodeServer();
  }
});

test.afterAll(async () => {
  server?.stop();
});

/**
 * Helper: navigate to code-server and wait for the workbench to be ready.
 */
async function setupPage(page: import("@playwright/test").Page) {
  const url = process.env.CODE_SERVER_URL ?? server.url;
  await page.goto(url, { waitUntil: "load" });
  await waitForWorkbench(page);
  await dismissDialogs(page);
}

// ---------------------------------------------------------------------------
// Screenshot 1: "Clean pipeline" — healthy model with no diagnostics
// ---------------------------------------------------------------------------
test('Screenshot: "Clean pipeline"', async ({ page }) => {
  await setupPage(page);

  // Prime LSP by opening a broken file first so we know it's running
  await openFile(page, "bad_ref.sql");
  await waitForDiagnostics(page, { timeout: 60_000 });

  // Now open a clean staging model (verified clean in smoke tests)
  await openFile(page, "stg_users.sql");
  // Give LSP time to process the new file
  await page.waitForTimeout(4000);

  // Verify no errors on the clean file
  const errors = await page.locator(".squiggly-error").count();
  expect(errors).toBe(0);

  await screenshotEditor(page, {
    feature: "diagnostics",
    name: "01-clean-pipeline",
  });
});

// ---------------------------------------------------------------------------
// Video: "Typo caught instantly" — bad_ref.sql shows error + hover tooltip
// (Playwright records the entire test as a video automatically)
// ---------------------------------------------------------------------------
test('Video: "Typo caught instantly"', async ({ page }) => {
  await setupPage(page);

  // Open the broken file with a typo: smelt.ref('stg_uusers')
  await openFile(page, "bad_ref.sql");

  // Wait for the red squiggly to appear
  await waitForDiagnostics(page, { timeout: 60_000 });

  // Verify error exists
  const errorsBefore = await page.locator(".squiggly-error").count();
  expect(errorsBefore).toBeGreaterThan(0);

  // Take screenshot showing the error squiggle
  await screenshotEditor(page, {
    feature: "diagnostics",
    name: "02-typo-error-visible",
  });

  // Hover over the erroneous ref string to show the tooltip
  await hoverWord(page, "stg_uusers");
  // Wait for hover content to fully render
  await page.waitForTimeout(2000);

  // Capture the hover tooltip (full page to include overlays)
  await screenshotWithOverlay(page, {
    feature: "diagnostics",
    name: "02-typo-hover-tooltip",
  });

  // Dismiss hover
  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
// Screenshot 3: "Type mismatch across models"
// ---------------------------------------------------------------------------
test('Screenshot: "Type mismatch across models"', async ({ page }) => {
  await setupPage(page);

  // Open the type mismatch file: WHERE user_id = 'abc'
  await openFile(page, "type_mismatch.sql");

  // Wait for diagnostics
  await waitForDiagnostics(page, { timeout: 60_000 });

  // Verify diagnostics are present (could be error or warning)
  const errors = await page.locator(".squiggly-error").count();
  const warnings = await page.locator(".squiggly-warning").count();
  expect(errors + warnings).toBeGreaterThan(0);

  // Take a screenshot showing the diagnostic
  await screenshotEditor(page, {
    feature: "diagnostics",
    name: "03-type-mismatch",
  });

  // Try hovering on the WHERE clause to trigger diagnostic tooltip.
  // This is best-effort — the main screenshot with squiggles is the key visual.
  try {
    await hoverWord(page, "abc");
    await page.waitForTimeout(2000);
    await screenshotWithOverlay(page, {
      feature: "diagnostics",
      name: "03-type-mismatch-hover",
    });
  } catch {
    // Hover on string literals can be flaky — try the column name instead
    try {
      await page.keyboard.press("Escape");
      await hoverWord(page, "user_id", { line: 6 });
      await page.waitForTimeout(2000);
      await screenshotWithOverlay(page, {
        feature: "diagnostics",
        name: "03-type-mismatch-hover",
      });
    } catch {
      // If hover fails entirely, the squiggly screenshot is still valuable
    }
  }
});

// ---------------------------------------------------------------------------
// Screenshot 4: "Undeclared column"
// ---------------------------------------------------------------------------
test('Screenshot: "Undeclared column"', async ({ page }) => {
  await setupPage(page);

  // Open the missing column file
  await openFile(page, "missing_column.sql");

  // Wait for diagnostics
  await waitForDiagnostics(page, { timeout: 60_000 });

  // Verify diagnostics exist
  const errors = await page.locator(".squiggly-error").count();
  const warnings = await page.locator(".squiggly-warning").count();
  expect(errors + warnings).toBeGreaterThan(0);

  // Screenshot showing the undeclared column error
  await screenshotEditor(page, {
    feature: "diagnostics",
    name: "04-undeclared-column",
  });

  // Hover on the offending column name
  try {
    await hoverWord(page, "nonexistent_col");
    await page.waitForTimeout(1500);
    await screenshotWithOverlay(page, {
      feature: "diagnostics",
      name: "04-undeclared-column-hover",
    });
  } catch {
    // Column text might not be hoverable as a single word; take editor screenshot as fallback
    await screenshotEditor(page, {
      feature: "diagnostics",
      name: "04-undeclared-column-detail",
    });
  }
});
