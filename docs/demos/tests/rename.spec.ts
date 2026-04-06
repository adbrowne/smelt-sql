/**
 * Phase 6: Rename Refactoring Demo
 *
 * Showcases project-wide rename — press F2 on a model name and every
 * ref('...') call across the project updates simultaneously.
 *
 * Produces 1 animated gif + 1 screenshot.
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
} from "../helpers/editor";
import {
  screenshotEditor,
  screenshotWithOverlay,
  mediaDir,
  VideoTimer,
  saveVideo,
  getEditorBounds,
} from "../helpers/capture";

let server: CodeServerHandle;

test.beforeAll(async () => {
  await mediaDir("rename");

  if (!process.env.CODE_SERVER_URL) {
    server = await launchCodeServer();
  }
});

test.afterAll(async () => {
  // Workspace is a disposable copy — no cleanup needed
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
 * Wait for the rename input widget to appear in the editor.
 */
async function waitForRenameInput(
  page: import("@playwright/test").Page,
  timeout = 10_000
): Promise<import("@playwright/test").Locator> {
  const renameInput = page.locator(
    ".rename-input input, .rename-box input, .monaco-editor .rename-input input"
  );
  await renameInput.first().waitFor({ timeout });
  await page.waitForTimeout(300);
  return renameInput.first();
}

// ---------------------------------------------------------------------------
// Screenshot: "Rename preview"
// Show the rename input dialog before pressing Enter.
// Caption: "See every change before it happens — no surprises."
// ---------------------------------------------------------------------------
test('Screenshot: "Rename preview"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open daily_revenue.sql which has smelt.ref('stg_events')
  await openFile(page, "daily_revenue.sql");
  await page.waitForTimeout(2000);

  // Click on 'stg_events' inside the ref call (line 6)
  await clickWord(page, "stg_events", { line: 6 });
  await page.waitForTimeout(500);

  // Trigger rename (F2)
  await page.keyboard.press("F2");
  await page.waitForTimeout(1000);

  // Wait for the rename input widget
  let renameInput: import("@playwright/test").Locator;
  try {
    renameInput = await waitForRenameInput(page);
  } catch {
    // Retry: click the word again and press F2
    await clickWord(page, "stg_events", { line: 6 });
    await page.waitForTimeout(500);
    await page.keyboard.press("F2");
    renameInput = await waitForRenameInput(page, 15_000);
  }

  // Clear the current text and type the new name (but do NOT press Enter)
  await renameInput.selectText();
  await page.keyboard.type("stg_activity_events", { delay: 40 });
  await page.waitForTimeout(1000);

  // Screenshot showing the rename input with the new name typed
  await screenshotWithOverlay(page, {
    feature: "rename",
    name: "01-rename-preview",
  });

  await screenshotEditor(page, {
    feature: "rename",
    name: "01-rename-preview-editor",
  });

  // Dismiss without applying — press Escape
  await page.keyboard.press("Escape");
  await page.waitForTimeout(500);
});

// ---------------------------------------------------------------------------
// Gif: "Rename a model across the project"
// Open daily_revenue.sql, position cursor on stg_events in the ref call,
// press F2, type new name, press Enter — all ref() calls update.
// Caption: "One keystroke renames the model and updates every reference
//           across the project."
// ---------------------------------------------------------------------------
test('Video: "Rename a model across the project"', async ({ page }) => {
  const timer = new VideoTimer();
  await setupPage(page);
  await primeLSP(page);

  // Open daily_revenue.sql which has smelt.ref('stg_events')
  await openFile(page, "daily_revenue.sql");
  await page.waitForTimeout(2000);

  // --- Demo starts ---
  timer.markDemoStart();

  // Click on 'stg_events' inside the ref call (line 6)
  await clickWord(page, "stg_events", { line: 6 });
  await page.waitForTimeout(2000);

  // Trigger rename (F2)
  await page.keyboard.press("F2");
  await page.waitForTimeout(1500);

  // Wait for the rename input widget
  let renameInput: import("@playwright/test").Locator;
  try {
    renameInput = await waitForRenameInput(page);
  } catch {
    // Retry
    await clickWord(page, "stg_events", { line: 6 });
    await page.waitForTimeout(1000);
    await page.keyboard.press("F2");
    renameInput = await waitForRenameInput(page, 15_000);
  }

  // Clear the current name and type the new name with deliberate pacing
  await renameInput.selectText();
  await page.waitForTimeout(500);
  await page.keyboard.type("stg_activity_events", { delay: 60 });
  await page.waitForTimeout(2000);

  // Press Enter to apply the rename across all files
  await page.keyboard.press("Enter");
  await page.waitForTimeout(3000);

  timer.markDemoEnd();

  // --- Capture gif ---
  const crop = await getEditorBounds(page);
  const viewport = page.viewportSize();
  await page.close();
  await saveVideo(page, {
    feature: "rename",
    name: "rename-model-across-project",
    timer,
    crop: crop ?? undefined,
    viewportSize: viewport ?? undefined,
  });
});
