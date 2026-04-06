/**
 * Phase 7: Code Actions / Quick Fixes Demo
 *
 * Showcases intelligent quick fixes — create missing models, CAST type
 * mismatches, add missing sources and columns. The LSP doesn't just find
 * problems, it offers one-click solutions.
 *
 * Produces 1 animated gif + up to 3 screenshots.
 */
import { test, expect } from "@playwright/test";
import { execSync } from "node:child_process";
import * as path from "node:path";
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
} from "../helpers/editor";
import {
  screenshotEditor,
  screenshotWithOverlay,
  mediaDir,
  VideoTimer,
  saveVideo,
  getEditorBounds,
} from "../helpers/capture";

const REPO_ROOT = path.resolve(__dirname, "..", "..", "..");

let server: CodeServerHandle;

test.beforeAll(async () => {
  await mediaDir("code-actions");

  if (!process.env.CODE_SERVER_URL) {
    server = await launchCodeServer();
  }
});

test.afterAll(async () => {
  // Clean up any files created by "create model" actions
  try {
    execSync("git checkout examples/demo_workspace/", { cwd: REPO_ROOT });
    execSync("git clean -fd examples/demo_workspace/", { cwd: REPO_ROOT });
  } catch {
    // Best effort
  }
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
 * The files are small enough that all lines are visible without scrolling.
 */
async function clickWord(
  page: import("@playwright/test").Page,
  word: string
) {
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
 * Wait for the code action widget (lightbulb dropdown) to appear.
 * Tries multiple selectors since code-server versions vary.
 */
async function waitForActionWidget(
  page: import("@playwright/test").Page,
  timeout = 10_000
): Promise<void> {
  // The action widget can appear with various class combinations
  const widgetLocator = page.locator(
    ".action-widget, " +
      ".editor-widget.action-widget, " +
      ".context-view .monaco-list, " +
      ".context-view .action-bar"
  ).first();
  await widgetLocator.waitFor({ timeout });

  // Wait for at least one visible item in the widget
  const itemLocator = page.locator(
    ".action-widget .monaco-list-row, " +
      ".action-widget .action-item, " +
      ".context-view .monaco-list-row, " +
      ".context-view .action-item"
  ).first();
  await itemLocator.waitFor({ timeout: 5000 });

  // Let the widget fully render
  await page.waitForTimeout(500);
}

/**
 * Ensure the editor has focus by dismissing overlays and clicking the editor.
 */
async function focusEditor(
  page: import("@playwright/test").Page
): Promise<void> {
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);

  const editor = page.locator(".editor-instance .monaco-editor .view-lines");
  if (await editor.isVisible().catch(() => false)) {
    await editor.click();
    await page.waitForTimeout(300);
  }
}

/**
 * Trigger code actions and wait for the widget, trying multiple approaches.
 * Returns true if the action widget appeared.
 */
async function triggerCodeActionsOnWord(
  page: import("@playwright/test").Page,
  word: string
): Promise<boolean> {
  await focusEditor(page);
  await clickWord(page, word);
  await page.waitForTimeout(500);

  // First try: Ctrl+. (works in code-server when editor has focus)
  await page.keyboard.press("Control+.");
  await page.waitForTimeout(500);
  try {
    await waitForActionWidget(page, 8_000);
    return true;
  } catch {
    // Ctrl+. may have been intercepted
  }

  // Second try: command palette "Quick Fix"
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  await focusEditor(page);
  await clickWord(page, word);
  await page.waitForTimeout(500);

  await page.keyboard.press("Control+Shift+p");
  const input = page.locator(".quick-input-widget input.input");
  await input.waitFor({ timeout: 5000 });
  await input.fill("Quick Fix");
  await page.waitForTimeout(500);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(500);

  try {
    await waitForActionWidget(page, 10_000);
    return true;
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------
// Screenshot 1: "Create model quick fix"
// Open bad_ref.sql which has smelt.ref('stg_uusers'). The LSP shows a red
// squiggly. Quick Fix shows "Create model 'stg_uusers'" action.
// Caption: "Reference a model before it exists — smelt scaffolds it for you."
// ---------------------------------------------------------------------------
test('Screenshot: "Create model quick fix"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open bad_ref.sql
  await openFile(page, "bad_ref.sql");
  await page.waitForTimeout(2000);
  await waitForDiagnostics(page, { timeout: 30_000 });
  await page.waitForTimeout(1000);

  const appeared = await triggerCodeActionsOnWord(page, "stg_uusers");
  if (!appeared) {
    test.skip(true, "Code action widget did not appear for undefined ref");
    return;
  }

  // Screenshot showing the lightbulb menu with "Create model" option
  await screenshotWithOverlay(page, {
    feature: "code-actions",
    name: "01-create-model-quickfix",
  });

  await screenshotEditor(page, {
    feature: "code-actions",
    name: "01-create-model-quickfix-editor",
  });

  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
// Gif: "Create a model from a reference"
// Animated version showing the full flow: error visible, code action triggered,
// lightbulb menu with "Create model" option.
// Caption: "Reference a model before it exists — smelt scaffolds it for you."
// ---------------------------------------------------------------------------
test('Video: "Create a model from a reference"', async ({ page }) => {
  const timer = new VideoTimer();
  await setupPage(page);
  await primeLSP(page);

  // Open bad_ref.sql
  await openFile(page, "bad_ref.sql");
  await page.waitForTimeout(2000);

  // Wait for the red squiggly on the undefined ref
  await waitForDiagnostics(page, { timeout: 30_000 });
  await page.waitForTimeout(1000);

  // Ensure editor has focus before demo starts
  await focusEditor(page);

  // --- Demo starts ---
  timer.markDemoStart();

  // Pause to show the error squiggly
  await page.waitForTimeout(2000);

  // Trigger code actions using the same reliable approach as the screenshot test
  const appeared = await triggerCodeActionsOnWord(page, "stg_uusers");

  if (appeared) {
    // Pause so viewer can see the lightbulb menu with "Create model" option
    await page.waitForTimeout(3000);
  } else {
    // If still no widget, just pause to show the error state
    await page.waitForTimeout(2000);
  }

  timer.markDemoEnd();

  // Capture the gif
  const crop = await getEditorBounds(page);
  const viewport = page.viewportSize();
  await page.keyboard.press("Escape");
  await page.close();
  await saveVideo(page, {
    feature: "code-actions",
    name: "create-model-from-ref",
    timer,
    crop: crop ?? undefined,
    viewportSize: viewport ?? undefined,
  });
});

// ---------------------------------------------------------------------------
// Screenshot 2: "Fix type mismatch with CAST"
// Open type_mismatch.sql which compares user_id (INTEGER) to 'abc' (VARCHAR).
// Quick Fix offers a "CAST as INTEGER" fix.
// Caption: "Type mismatches come with a one-click CAST fix."
// ---------------------------------------------------------------------------
test('Screenshot: "Fix type mismatch with CAST"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open the type mismatch file
  await openFile(page, "type_mismatch.sql");
  await page.waitForTimeout(3000);

  // Wait for diagnostics — the type mismatch may show as warning or error
  try {
    await waitForDiagnostics(page, { timeout: 45_000 });
  } catch {
    test.skip(true, "Type mismatch diagnostic did not appear");
    return;
  }
  await page.waitForTimeout(1000);

  // The diagnostic is on the WHERE clause comparison; try clicking on 'abc'
  // which is the string literal being compared to INTEGER user_id
  let appeared = await triggerCodeActionsOnWord(page, "abc");

  // If no code action on 'abc', try the whole WHERE expression
  if (!appeared) {
    await page.keyboard.press("Escape");
    await page.waitForTimeout(300);
    appeared = await triggerCodeActionsOnWord(page, "user_id");
  }

  if (!appeared) {
    test.skip(true, "CAST code action did not appear for type mismatch");
    return;
  }

  // Screenshot showing the CAST fix in the lightbulb menu
  await screenshotWithOverlay(page, {
    feature: "code-actions",
    name: "02-cast-type-mismatch",
  });

  await screenshotEditor(page, {
    feature: "code-actions",
    name: "02-cast-type-mismatch-editor",
  });

  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
// Screenshot 3: "Quick fixes for undeclared columns"
// Open missing_column.sql which references nonexistent_col on stg_users.
// Quick Fix shows available actions for the diagnostic.
// Caption: "Even column declarations can be auto-added to your source definitions."
// ---------------------------------------------------------------------------
test('Screenshot: "Quick fixes for undeclared columns"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open missing_column.sql
  await openFile(page, "missing_column.sql");
  await page.waitForTimeout(2000);

  // Wait for diagnostics on the undeclared column
  try {
    await waitForDiagnostics(page, { timeout: 30_000 });
  } catch {
    test.skip(true, "Missing column diagnostic did not appear");
    return;
  }
  await page.waitForTimeout(1000);

  const appeared = await triggerCodeActionsOnWord(page, "nonexistent_col");
  if (!appeared) {
    test.skip(true, "Code action did not appear for undeclared column");
    return;
  }

  // Screenshot showing the code actions for the undeclared column
  await screenshotWithOverlay(page, {
    feature: "code-actions",
    name: "03-undeclared-column-quickfix",
  });

  await screenshotEditor(page, {
    feature: "code-actions",
    name: "03-undeclared-column-quickfix-editor",
  });

  await page.keyboard.press("Escape");
});
