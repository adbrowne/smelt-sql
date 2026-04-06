/**
 * Phase 5: Find References Demo
 *
 * Showcases instant impact analysis — before changing a model, see every
 * downstream consumer. Works for model refs and CTEs within a file.
 *
 * Produces 2 screenshots.
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
  waitForDiagnostics,
  goToLine,
  findReferences,
} from "../helpers/editor";
import {
  screenshotEditor,
  screenshotWithOverlay,
  mediaDir,
} from "../helpers/capture";

let server: CodeServerHandle;

test.beforeAll(async () => {
  await mediaDir("references");

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

/**
 * Wait for the references peek widget to appear and populate with results.
 */
async function waitForReferencesWidget(
  page: import("@playwright/test").Page,
  timeout = 15_000
): Promise<void> {
  // The peek widget appears as a zone widget in the editor
  await page
    .locator(".reference-zone-widget, .references-view, .zone-widget")
    .first()
    .waitFor({ timeout });

  // Wait for reference entries to populate in the tree
  await page
    .locator(
      ".reference-zone-widget .monaco-list-row, " +
        ".references-view .monaco-list-row, " +
        ".zone-widget .monaco-list-row"
    )
    .first()
    .waitFor({ timeout: 10_000 });

  // Let the widget fully render
  await page.waitForTimeout(1000);
}

// ---------------------------------------------------------------------------
// Screenshot 1: "Find all consumers of a model"
// Open daily_revenue.sql, find references on 'stg_events' inside the ref call.
// The peek widget shows all downstream consumers of stg_events.
// Caption: "Before changing a model, see every downstream consumer —
//           instant impact analysis."
// ---------------------------------------------------------------------------
test('Screenshot: "Find all consumers of a model"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open daily_revenue.sql which has smelt.ref('stg_events')
  await openFile(page, "daily_revenue.sql");
  await page.waitForTimeout(2000);

  // Click on 'stg_events' inside the ref call (line 4: FROM smelt.ref('stg_events'))
  await clickWord(page, "stg_events", { line: 4 });
  await page.waitForTimeout(500);

  // Trigger Find References (Shift+F12)
  await page.keyboard.press("Shift+F12");
  await page.waitForTimeout(1000);

  // Wait for the references widget to appear and populate
  try {
    await waitForReferencesWidget(page);
  } catch {
    // Retry: sometimes the LSP needs more time
    await page.keyboard.press("Escape");
    await page.waitForTimeout(2000);
    await clickWord(page, "stg_events", { line: 4 });
    await page.waitForTimeout(500);
    await page.keyboard.press("Shift+F12");
    await waitForReferencesWidget(page, 20_000);
  }

  // Verify we have reference results
  const refRows = page.locator(
    ".reference-zone-widget .monaco-list-row, " +
      ".references-view .monaco-list-row, " +
      ".zone-widget .monaco-list-row"
  );
  const count = await refRows.count();
  expect(count).toBeGreaterThan(0);

  // Screenshot: full page showing the peek widget with references
  await screenshotWithOverlay(page, {
    feature: "references",
    name: "01-find-model-consumers",
  });

  // Editor-only crop
  await screenshotEditor(page, {
    feature: "references",
    name: "01-find-model-consumers-editor",
  });

  // Dismiss the peek widget
  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
// Screenshot 2: "Find CTE references within a file"
// Open user_first_purchase.sql, find references on the 'purchases' CTE name.
// The peek widget shows the definition and usage within the file.
// Caption: "Works for CTEs too — see every use of a subquery within
//           complex models."
// ---------------------------------------------------------------------------
test('Screenshot: "Find CTE references within a file"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open user_first_purchase.sql which has a 'purchases' CTE
  await openFile(page, "user_first_purchase.sql");
  await page.waitForTimeout(2000);

  // Click on 'purchases' in the CTE definition (line 2: WITH purchases AS ()
  await clickWord(page, "purchases", { line: 2 });
  await page.waitForTimeout(500);

  // Trigger Find References (Shift+F12)
  await page.keyboard.press("Shift+F12");
  await page.waitForTimeout(1000);

  // Wait for the references widget to appear and populate
  try {
    await waitForReferencesWidget(page);
  } catch {
    // Retry with more time
    await page.keyboard.press("Escape");
    await page.waitForTimeout(2000);
    await clickWord(page, "purchases", { line: 2 });
    await page.waitForTimeout(500);
    await page.keyboard.press("Shift+F12");
    await waitForReferencesWidget(page, 20_000);
  }

  // Verify we have reference results
  const refRows = page.locator(
    ".reference-zone-widget .monaco-list-row, " +
      ".references-view .monaco-list-row, " +
      ".zone-widget .monaco-list-row"
  );
  const count = await refRows.count();
  expect(count).toBeGreaterThan(0);

  // Screenshot: full page showing the peek widget with CTE references
  await screenshotWithOverlay(page, {
    feature: "references",
    name: "02-find-cte-references",
  });

  // Editor-only crop
  await screenshotEditor(page, {
    feature: "references",
    name: "02-find-cte-references-editor",
  });

  // Dismiss the peek widget
  await page.keyboard.press("Escape");
});
