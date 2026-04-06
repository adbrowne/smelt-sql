/**
 * Phase 2: Go-to-Definition Demo
 *
 * Showcases how Ctrl+Click / F12 navigation works across the model dependency
 * graph — tracing a revenue metric from mart model back to raw source.
 * Produces 2 screenshots + 1 video (auto-recorded by Playwright).
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
  goToDefinition,
  getActiveTabName,
  goToLine,
} from "../helpers/editor";
import {
  screenshot,
  screenshotEditor,
  mediaDir,
} from "../helpers/capture";

let server: CodeServerHandle;

test.beforeAll(async () => {
  await mediaDir("goto-definition");

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
 * Helper: click on a specific word in the editor to position the cursor there.
 * Uses :text-is() to target leaf spans (exact text match) rather than
 * :has-text() which matches parent containers and produces wrong coordinates.
 */
async function clickWord(
  page: import("@playwright/test").Page,
  word: string,
  opts?: { line?: number }
) {
  if (opts?.line) {
    await goToLine(page, opts.line);
  }
  // Use :text-is() for exact leaf-span matching. Falls back to :has-text()
  // for text that spans multiple tokens (e.g., 'raw.events' in a string).
  const editorArea = page.locator(".editor-instance .monaco-editor");
  let wordSpan = editorArea
    .locator(`.view-lines .view-line span:text-is("${word}")`)
    .first();

  // If exact match not found, fall back to :has-text() on the deepest span
  if (!(await wordSpan.isVisible({ timeout: 2000 }).catch(() => false))) {
    wordSpan = editorArea
      .locator(".view-lines .view-line")
      .locator(`span:has-text("${word}")`)
      .last(); // last = deepest/most-specific match
  }

  await wordSpan.click();
  await page.waitForTimeout(300);
}


/**
 * Helper: wait for a specific file tab to become active.
 */
async function waitForActiveTab(
  page: import("@playwright/test").Page,
  expectedName: string,
  timeout = 10_000
) {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const tabName = await getActiveTabName(page);
    if (tabName && tabName.includes(expectedName)) return;
    await page.waitForTimeout(300);
  }
  // Final check with assertion
  const tabName = await getActiveTabName(page);
  expect(tabName).toContain(expectedName);
}

// ---------------------------------------------------------------------------
// Ensure LSP is initialized before running go-to-definition tests.
// We prime it by opening a broken file and waiting for diagnostics.
// ---------------------------------------------------------------------------
async function primeLSP(page: import("@playwright/test").Page) {
  await openFile(page, "bad_ref.sql");
  await waitForDiagnostics(page, { timeout: 60_000 });
  // Give the LSP a moment to fully index the workspace
  await page.waitForTimeout(2000);
}

// ---------------------------------------------------------------------------
// Video: "Trace a column through the pipeline"
// Start in daily_revenue.sql, jump through stg_events to raw source.
// (Playwright auto-records video for the full test)
// ---------------------------------------------------------------------------
test('Video: "Trace a column through the pipeline"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Step 1: Open the daily_revenue model — our starting point for tracing
  await openFile(page, "daily_revenue.sql");
  await page.waitForTimeout(1500);

  await screenshotEditor(page, {
    feature: "goto-definition",
    name: "01-start-daily-revenue",
  });

  // Step 2: Click on stg_events inside smelt.ref('stg_events') and F12
  await clickWord(page, "stg_events");
  await page.waitForTimeout(500);
  await goToDefinition(page);

  // Verify we jumped to stg_events.sql
  await waitForActiveTab(page, "stg_events.sql");
  await page.waitForTimeout(1000);

  await screenshotEditor(page, {
    feature: "goto-definition",
    name: "02-jumped-to-stg-events",
  });

  // Step 3: Jump from stg_events to the raw.events source definition
  await clickWord(page, "raw.events");
  await page.waitForTimeout(500);
  await goToDefinition(page);

  await waitForActiveTab(page, "sources.yml");
  await page.waitForTimeout(1000);

  await screenshotEditor(page, {
    feature: "goto-definition",
    name: "03-jumped-to-sources-yml",
  });
});

// ---------------------------------------------------------------------------
// Screenshot: "Jump to CTE definition"
// In user_first_purchase.sql, click on the CTE name 'purchases' in the
// FROM clause and go to its definition in the WITH clause.
// ---------------------------------------------------------------------------
test('Screenshot: "Jump to CTE definition"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open user_first_purchase.sql which has a 'purchases' CTE
  await openFile(page, "user_first_purchase.sql");
  await page.waitForTimeout(1500);

  // The CTE 'purchases' is referenced in the FROM clause on line 17:
  // FROM purchases AS p
  // Click on 'purchases' in the FROM clause
  await clickWord(page, "purchases", { line: 17 });
  await page.waitForTimeout(500);

  // Go to definition — should jump to the WITH clause CTE definition
  await goToDefinition(page);
  await page.waitForTimeout(1000);

  // Take a screenshot showing the cursor now at the CTE definition
  await screenshotEditor(page, {
    feature: "goto-definition",
    name: "05-cte-definition-jump",
  });

  // Also take a full-page screenshot for context
  await screenshot(page, {
    feature: "goto-definition",
    name: "05-cte-definition-jump-full",
  });
});

// ---------------------------------------------------------------------------
// Screenshot: "Jump to source definition"
// Show the cursor landing in sources.yml with the events table visible.
// ---------------------------------------------------------------------------
test('Screenshot: "Jump to source definition"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open stg_events.sql which references smelt.source('raw.events')
  await openFile(page, "stg_events.sql");
  await page.waitForTimeout(1500);

  // Take a "before" screenshot
  await screenshotEditor(page, {
    feature: "goto-definition",
    name: "06-source-before-jump",
  });

  // Click on the source reference and go to definition
  await clickWord(page, "raw.events");
  await page.waitForTimeout(500);
  await goToDefinition(page);

  // Verify we landed in sources.yml
  await waitForActiveTab(page, "sources.yml");
  await page.waitForTimeout(1000);

  // Screenshot showing sources.yml with the events table definition visible
  await screenshotEditor(page, {
    feature: "goto-definition",
    name: "06-source-definition-landing",
  });

  // Full-page version showing the tab bar and file explorer for context
  await screenshot(page, {
    feature: "goto-definition",
    name: "06-source-definition-landing-full",
  });
});
