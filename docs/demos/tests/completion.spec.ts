/**
 * Phase 4: Code Completion Demo
 *
 * Showcases intelligent, context-aware completions that know your data model —
 * model names inside ref(), source tables inside source(), and column names
 * from upstream schemas. Not just SQL keywords.
 *
 * Produces 1 animated gif + 2 screenshots.
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
  triggerCompletion,
} from "../helpers/editor";
import {
  screenshot,
  screenshotEditor,
  screenshotWithOverlay,
  mediaDir,
  VideoTimer,
  saveVideo,
  getEditorBounds,
} from "../helpers/capture";

let server: CodeServerHandle;

test.beforeAll(async () => {
  await mediaDir("completion");

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
 * Helper: wait for the suggest widget to appear and have visible items.
 */
async function waitForSuggestWidget(
  page: import("@playwright/test").Page,
  timeout = 10_000
): Promise<void> {
  // Wait for the suggest widget container to appear
  await page
    .locator(".editor-widget.suggest-widget.visible, .editor-widget.suggest-widget[style*='display']")
    .first()
    .waitFor({ timeout });
  // Wait for at least one completion item to render
  await page
    .locator(".editor-widget.suggest-widget .monaco-list-row")
    .first()
    .waitFor({ timeout: 5000 });
  // Let the widget fully render
  await page.waitForTimeout(500);
}

// ---------------------------------------------------------------------------
// Screenshot 1: "Model name completions"
// Inside ref(''), show the completion dropdown listing all available models.
// ---------------------------------------------------------------------------
test('Screenshot: "Model name completions"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open daily_revenue.sql which has smelt.ref('stg_events')
  await openFile(page, "daily_revenue.sql");
  await page.waitForTimeout(2000);

  // Go to the end of the file, add a new line, and type a ref to trigger completions
  await page.keyboard.press("Control+End");
  await page.waitForTimeout(300);
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");

  // Type a partial ref — the LSP triggers completions on the ' character
  await page.keyboard.type("-- FROM smelt.ref('", { delay: 30 });
  await page.waitForTimeout(500);

  // The ' trigger character should auto-show completions. If not, trigger manually.
  const suggestVisible = await page
    .locator(".editor-widget.suggest-widget .monaco-list-row")
    .first()
    .isVisible({ timeout: 2000 })
    .catch(() => false);

  if (!suggestVisible) {
    await triggerCompletion(page);
  }

  await waitForSuggestWidget(page);

  // Verify we see model names in the completion list
  const completionItems = page.locator(
    ".editor-widget.suggest-widget .monaco-list-row"
  );
  const count = await completionItems.count();
  expect(count).toBeGreaterThan(0);

  // Screenshot showing the completion dropdown with model names
  await screenshotWithOverlay(page, {
    feature: "completion",
    name: "01-model-name-completions",
  });

  await screenshotEditor(page, {
    feature: "completion",
    name: "01-model-name-completions-editor",
  });

  // Dismiss and undo our edits
  await page.keyboard.press("Escape");
  await page.keyboard.press("Control+z");
  await page.keyboard.press("Control+z");
  await page.keyboard.press("Control+z");
});

// ---------------------------------------------------------------------------
// Screenshot 2: "Source table completions"
// Inside source(''), show the completion dropdown listing all available source
// tables — demonstrates context-aware completions beyond just model refs.
// ---------------------------------------------------------------------------
test('Screenshot: "Source table completions"', async ({ page }) => {
  await setupPage(page);
  await primeLSP(page);

  // Open stg_events.sql which uses smelt.source('raw.events')
  await openFile(page, "stg_events.sql");
  await page.waitForTimeout(2000);

  // Go to end of file and type a source call to trigger source completions
  await page.keyboard.press("Control+End");
  await page.waitForTimeout(300);
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");

  // Type a source call — the ' trigger character should show source table completions
  await page.keyboard.type("-- FROM smelt.source('", { delay: 30 });
  await page.waitForTimeout(1000);

  // Wait for or trigger source completions
  const suggestVisible = await page
    .locator(".editor-widget.suggest-widget .monaco-list-row")
    .first()
    .isVisible({ timeout: 3000 })
    .catch(() => false);

  if (!suggestVisible) {
    await triggerCompletion(page);
  }

  try {
    await waitForSuggestWidget(page);

    const completionItems = page.locator(
      ".editor-widget.suggest-widget .monaco-list-row"
    );
    const count = await completionItems.count();
    expect(count).toBeGreaterThan(0);

    await screenshotWithOverlay(page, {
      feature: "completion",
      name: "02-source-completions",
    });

    await screenshotEditor(page, {
      feature: "completion",
      name: "02-source-completions-editor",
    });
  } finally {
    // Dismiss and undo
    await page.keyboard.press("Escape");
    for (let i = 0; i < 5; i++) {
      await page.keyboard.press("Control+z");
    }
  }
});

// ---------------------------------------------------------------------------
// Gif: "Build a query with completions"
// Demonstrate model name + source table completions in sequence, showing how
// completions guide you through writing a query.
// ---------------------------------------------------------------------------
test('Video: "Build a query with completions"', async ({ page }) => {
  const timer = new VideoTimer();
  await setupPage(page);
  await primeLSP(page);

  // Open stg_events.sql — simple model, good canvas for demo
  await openFile(page, "stg_events.sql");
  await page.waitForTimeout(2000);

  // --- Demo starts ---
  timer.markDemoStart();

  // Step 1: Show model name completions inside ref()
  await page.keyboard.press("Control+End");
  await page.waitForTimeout(1000);
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");

  // Type a ref — the ' trigger character should show model completions
  await page.keyboard.type("-- smelt.ref('", { delay: 60 });
  await page.waitForTimeout(1000);

  let suggestUp = await page
    .locator(".editor-widget.suggest-widget .monaco-list-row")
    .first()
    .isVisible({ timeout: 2000 })
    .catch(() => false);

  if (!suggestUp) {
    await triggerCompletion(page);
  }

  try {
    await waitForSuggestWidget(page);
  } catch {
    await page.keyboard.type("stg_", { delay: 60 });
    await page.waitForTimeout(500);
    await triggerCompletion(page);
    await waitForSuggestWidget(page);
  }

  // Pause so viewer can see model name completions
  await page.waitForTimeout(2500);

  // Dismiss and clean up
  await page.keyboard.press("Escape");
  await page.waitForTimeout(500);
  await page.keyboard.press("Control+Shift+k"); // Delete line
  await page.keyboard.press("Control+Shift+k"); // Delete extra blank
  await page.waitForTimeout(500);

  // Step 2: Show source table completions inside source()
  await page.keyboard.press("Control+End");
  await page.waitForTimeout(500);
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");

  await page.keyboard.type("-- smelt.source('", { delay: 60 });
  await page.waitForTimeout(1000);

  suggestUp = await page
    .locator(".editor-widget.suggest-widget .monaco-list-row")
    .first()
    .isVisible({ timeout: 2000 })
    .catch(() => false);

  if (!suggestUp) {
    await triggerCompletion(page);
  }

  try {
    await waitForSuggestWidget(page);
    // Pause so viewer can see source completions
    await page.waitForTimeout(2500);
  } catch {
    // Best-effort — the model name completions are the primary demo
    await page.waitForTimeout(1000);
  }

  timer.markDemoEnd();

  // --- Capture gif ---
  const crop = await getEditorBounds(page);
  const viewport = page.viewportSize();
  await page.keyboard.press("Escape");
  await page.close();
  await saveVideo(page, {
    feature: "completion",
    name: "build-query-with-completions",
    timer,
    crop: crop ?? undefined,
    viewportSize: viewport ?? undefined,
  });
});
