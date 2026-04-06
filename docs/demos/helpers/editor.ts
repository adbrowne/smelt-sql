/**
 * High-level VS Code editor interaction helpers for Playwright.
 *
 * These work against code-server's web UI, which mirrors VS Code's DOM structure.
 */
import { type Page, type Locator } from "@playwright/test";

/** Dismiss any initial dialogs or trust prompts that code-server may show. */
export async function dismissDialogs(page: Page): Promise<void> {
  // Dismiss workspace trust dialog if present
  const trustBtn = page.locator('a.monaco-button:has-text("Yes, I trust")');
  if (await trustBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    await trustBtn.click();
  }

  // Close any welcome or getting started tabs via keyboard shortcut.
  // Tab label and close-button classes vary across code-server versions,
  // so the most reliable approach is to use the workbench command.
  // Ctrl+W closes the active editor tab.
  for (const label of ["Welcome", "Get Started"]) {
    const tab = page.locator(`.tab:has-text("${label}")`);
    if (await tab.isVisible({ timeout: 2000 }).catch(() => false)) {
      // Click the tab first to make it the active editor
      await tab.click();
      // Close via keyboard shortcut — works regardless of close-button class
      await page.keyboard.press("Control+w");
      await page.waitForTimeout(500);
      break;
    }
  }

  // Close the chat/secondary side bar if visible
  const chatPanel = page.locator('.auxiliary-bar-title :text("CHAT")');
  if (await chatPanel.isVisible({ timeout: 1000 }).catch(() => false)) {
    // Toggle secondary side bar off
    await runCommand(page, "View: Toggle Secondary Side Bar Visibility");
  }

  // Press Escape to dismiss any other overlays
  await page.keyboard.press("Escape");
}

/**
 * Enable VS Code screencast mode so keystrokes are visible in recordings.
 */
export async function enableScreencastMode(page: Page): Promise<void> {
  await runCommand(page, "Toggle Screencast Mode");
  // Wait for the screencast overlay to initialize
  await page.waitForTimeout(500);
}

/**
 * Run a VS Code command via the Command Palette (Ctrl+Shift+P).
 */
export async function runCommand(page: Page, command: string): Promise<void> {
  await page.keyboard.press("Control+Shift+p");
  const input = page.locator(".quick-input-widget input.input");
  await input.waitFor({ timeout: 5000 });
  await input.fill(command);
  await page.waitForTimeout(500);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(500);
}

/** Wait for the VS Code workbench to be fully loaded and file tree to be ready. */
export async function waitForWorkbench(page: Page): Promise<void> {
  // The main workbench container
  await page.locator(".monaco-workbench").waitFor({ timeout: 30_000 });

  // Wait for the explorer file tree to show at least one file entry.
  // This ensures the workspace folder has been indexed and files are discoverable.
  try {
    await page
      .locator(".explorer-folders-view .monaco-list-row")
      .first()
      .waitFor({ timeout: 30_000 });
  } catch {
    // Fallback: click the workspace folder in the explorer to expand it
    const folderEntry = page.locator(".explorer-folders-view .monaco-tl-twistie").first();
    if (await folderEntry.isVisible().catch(() => false)) {
      await folderEntry.click();
      await page.waitForTimeout(2000);
    }
  }

  // Give the file watcher additional time to index for Quick Open
  await page.waitForTimeout(2000);
}

/**
 * Open a file via the quick-open dialog (Ctrl+P).
 * Retries if the file index isn't ready yet (common on first load).
 * @param filePath - file name or relative path, e.g. "bad_ref.sql" or "stg_users.sql"
 */
export async function openFile(page: Page, filePath: string): Promise<void> {
  const maxAttempts = 8;
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    // Trigger quick open
    await page.keyboard.press("Control+p");
    // Wait for the quick-open input
    const input = page.locator(".quick-input-widget input.input");
    await input.waitFor({ timeout: 5000 });
    await input.fill(filePath);
    // Wait for the file list to filter
    await page.waitForTimeout(1500);

    // Check if there are matching results
    const noResults = page.locator('.quick-input-list :text("No matching results")');
    const hasNoResults = await noResults.isVisible({ timeout: 500 }).catch(() => false);

    if (hasNoResults) {
      // Dismiss and retry — the file index may not be ready yet
      await page.keyboard.press("Escape");
      if (attempt < maxAttempts) {
        await page.waitForTimeout(3000);
        continue;
      }
    }

    await page.keyboard.press("Enter");
    // Wait for editor to open with the file
    await page.waitForTimeout(1000);
    return;
  }
}

/**
 * Go to a specific line in the current editor (Ctrl+G).
 */
export async function goToLine(page: Page, line: number): Promise<void> {
  await page.keyboard.press("Control+g");
  const input = page.locator(".quick-input-widget input.input");
  await input.waitFor({ timeout: 5000 });
  await input.fill(String(line));
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);
}

/**
 * Hover over a word in the current editor and wait for the hover widget.
 * Uses Ctrl+G to go to the line, then moves to the word's position.
 * @returns The hover widget locator (if it appeared).
 */
export async function hoverWord(
  page: Page,
  word: string,
  opts?: { line?: number }
): Promise<Locator> {
  if (opts?.line) {
    await goToLine(page, opts.line);
  }

  // Find the word in the visible editor and hover over it
  const wordSpan = page
    .locator(".view-lines .view-line")
    .locator(`span:has-text("${word}")`)
    .first();

  await wordSpan.hover();

  // Wait for hover widget (use .first() — code-server may render multiple hover containers)
  const hoverWidget = page.locator(".monaco-hover-content").first();
  await hoverWidget.waitFor({ timeout: 5000 });

  return hoverWidget;
}

/**
 * Trigger code completion (Ctrl+Space) and wait for the suggest widget.
 */
export async function triggerCompletion(page: Page): Promise<Locator> {
  await page.keyboard.press("Control+Space");
  const suggestWidget = page.locator(".editor-widget.suggest-widget");
  await suggestWidget.waitFor({ timeout: 5000 });
  return suggestWidget;
}

/**
 * Go to definition (F12) from the current cursor position.
 */
export async function goToDefinition(page: Page): Promise<void> {
  await page.keyboard.press("F12");
  // Wait for navigation to happen
  await page.waitForTimeout(1500);
}

/**
 * Find references (Shift+F12) from the current cursor position.
 * Returns the references widget locator.
 */
export async function findReferences(page: Page): Promise<Locator> {
  await page.keyboard.press("Shift+F12");
  // Wait for the references panel/peek widget to appear
  const refsWidget = page.locator(
    ".reference-zone-widget, .references-view"
  );
  await refsWidget.first().waitFor({ timeout: 5000 });
  return refsWidget.first();
}

/**
 * Trigger rename (F2) and type the new name.
 */
export async function rename(page: Page, newName: string): Promise<void> {
  await page.keyboard.press("F2");
  // Wait for rename input
  const renameInput = page.locator(".rename-input input, .rename-box input");
  await renameInput.first().waitFor({ timeout: 5000 });
  await renameInput.first().fill(newName);
  await page.keyboard.press("Enter");
  // Wait for rename to apply
  await page.waitForTimeout(1000);
}

/**
 * Get code actions via Ctrl+. and wait for the lightbulb menu.
 */
export async function getCodeActions(page: Page): Promise<Locator> {
  await page.keyboard.press("Control+.");
  const actionsWidget = page.locator(
    ".editor-widget.action-widget, .context-view .actions-container"
  );
  await actionsWidget.first().waitFor({ timeout: 5000 });
  return actionsWidget.first();
}

/**
 * Wait for LSP diagnostics (squiggly underlines) to appear in the editor.
 * This indicates the smelt LSP has initialized and is running.
 */
export async function waitForDiagnostics(
  page: Page,
  opts?: { timeout?: number }
): Promise<void> {
  const timeout = opts?.timeout ?? 30_000;
  // Diagnostics appear as decorations with squiggly classes
  await page
    .locator(
      ".squiggly-error, .squiggly-warning, .squiggly-info"
    )
    .first()
    .waitFor({ timeout });
}

/**
 * Wait for the LSP to be ready by checking for the absence of loading indicators
 * and the presence of language status items.
 */
export async function waitForLSPReady(
  page: Page,
  opts?: { timeout?: number }
): Promise<void> {
  const timeout = opts?.timeout ?? 30_000;
  // Wait a baseline amount for the extension to activate
  await page.waitForTimeout(3000);

  // Check status bar for smelt language indicator or just wait for diagnostics
  // The most reliable signal is that diagnostics appear on a known-broken file
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const squigglies = await page
      .locator(".squiggly-error, .squiggly-warning")
      .count();
    if (squigglies > 0) return;
    await page.waitForTimeout(500);
  }
}

/**
 * Type text into the current editor at the cursor position.
 */
export async function typeText(page: Page, text: string): Promise<void> {
  await page.keyboard.type(text, { delay: 50 });
}

/**
 * Select all text in the current editor.
 */
export async function selectAll(page: Page): Promise<void> {
  await page.keyboard.press("Control+a");
}

/**
 * Get the currently active editor tab name.
 */
export async function getActiveTabName(page: Page): Promise<string | null> {
  const tab = page.locator(".tab.active .label-name");
  if (await tab.isVisible().catch(() => false)) {
    return tab.textContent();
  }
  return null;
}
