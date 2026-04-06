# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: diagnostics.spec.ts >> Screenshot: "Type mismatch across models"
- Location: tests/diagnostics.spec.ts:142:5

# Error details

```
TimeoutError: locator.waitFor: Timeout 60000ms exceeded.
Call log:
  - waiting for locator('.squiggly-error, .squiggly-warning, .squiggly-info').first() to be visible

```

# Test source

```ts
  70  |  * @returns The hover widget locator (if it appeared).
  71  |  */
  72  | export async function hoverWord(
  73  |   page: Page,
  74  |   word: string,
  75  |   opts?: { line?: number }
  76  | ): Promise<Locator> {
  77  |   if (opts?.line) {
  78  |     await goToLine(page, opts.line);
  79  |   }
  80  | 
  81  |   // Find the word in the visible editor and hover over it
  82  |   const wordSpan = page
  83  |     .locator(".view-lines .view-line")
  84  |     .locator(`span:has-text("${word}")`)
  85  |     .first();
  86  | 
  87  |   await wordSpan.hover();
  88  | 
  89  |   // Wait for hover widget (use .first() — code-server may render multiple hover containers)
  90  |   const hoverWidget = page.locator(".monaco-hover-content").first();
  91  |   await hoverWidget.waitFor({ timeout: 5000 });
  92  | 
  93  |   return hoverWidget;
  94  | }
  95  | 
  96  | /**
  97  |  * Trigger code completion (Ctrl+Space) and wait for the suggest widget.
  98  |  */
  99  | export async function triggerCompletion(page: Page): Promise<Locator> {
  100 |   await page.keyboard.press("Control+Space");
  101 |   const suggestWidget = page.locator(".editor-widget.suggest-widget");
  102 |   await suggestWidget.waitFor({ timeout: 5000 });
  103 |   return suggestWidget;
  104 | }
  105 | 
  106 | /**
  107 |  * Go to definition (F12) from the current cursor position.
  108 |  */
  109 | export async function goToDefinition(page: Page): Promise<void> {
  110 |   await page.keyboard.press("F12");
  111 |   // Wait for navigation to happen
  112 |   await page.waitForTimeout(1500);
  113 | }
  114 | 
  115 | /**
  116 |  * Find references (Shift+F12) from the current cursor position.
  117 |  * Returns the references widget locator.
  118 |  */
  119 | export async function findReferences(page: Page): Promise<Locator> {
  120 |   await page.keyboard.press("Shift+F12");
  121 |   // Wait for the references panel/peek widget to appear
  122 |   const refsWidget = page.locator(
  123 |     ".reference-zone-widget, .references-view"
  124 |   );
  125 |   await refsWidget.first().waitFor({ timeout: 5000 });
  126 |   return refsWidget.first();
  127 | }
  128 | 
  129 | /**
  130 |  * Trigger rename (F2) and type the new name.
  131 |  */
  132 | export async function rename(page: Page, newName: string): Promise<void> {
  133 |   await page.keyboard.press("F2");
  134 |   // Wait for rename input
  135 |   const renameInput = page.locator(".rename-input input, .rename-box input");
  136 |   await renameInput.first().waitFor({ timeout: 5000 });
  137 |   await renameInput.first().fill(newName);
  138 |   await page.keyboard.press("Enter");
  139 |   // Wait for rename to apply
  140 |   await page.waitForTimeout(1000);
  141 | }
  142 | 
  143 | /**
  144 |  * Get code actions via Ctrl+. and wait for the lightbulb menu.
  145 |  */
  146 | export async function getCodeActions(page: Page): Promise<Locator> {
  147 |   await page.keyboard.press("Control+.");
  148 |   const actionsWidget = page.locator(
  149 |     ".editor-widget.action-widget, .context-view .actions-container"
  150 |   );
  151 |   await actionsWidget.first().waitFor({ timeout: 5000 });
  152 |   return actionsWidget.first();
  153 | }
  154 | 
  155 | /**
  156 |  * Wait for LSP diagnostics (squiggly underlines) to appear in the editor.
  157 |  * This indicates the smelt LSP has initialized and is running.
  158 |  */
  159 | export async function waitForDiagnostics(
  160 |   page: Page,
  161 |   opts?: { timeout?: number }
  162 | ): Promise<void> {
  163 |   const timeout = opts?.timeout ?? 30_000;
  164 |   // Diagnostics appear as decorations with squiggly classes
  165 |   await page
  166 |     .locator(
  167 |       ".squiggly-error, .squiggly-warning, .squiggly-info"
  168 |     )
  169 |     .first()
> 170 |     .waitFor({ timeout });
      |      ^ TimeoutError: locator.waitFor: Timeout 60000ms exceeded.
  171 | }
  172 | 
  173 | /**
  174 |  * Wait for the LSP to be ready by checking for the absence of loading indicators
  175 |  * and the presence of language status items.
  176 |  */
  177 | export async function waitForLSPReady(
  178 |   page: Page,
  179 |   opts?: { timeout?: number }
  180 | ): Promise<void> {
  181 |   const timeout = opts?.timeout ?? 30_000;
  182 |   // Wait a baseline amount for the extension to activate
  183 |   await page.waitForTimeout(3000);
  184 | 
  185 |   // Check status bar for smelt language indicator or just wait for diagnostics
  186 |   // The most reliable signal is that diagnostics appear on a known-broken file
  187 |   const start = Date.now();
  188 |   while (Date.now() - start < timeout) {
  189 |     const squigglies = await page
  190 |       .locator(".squiggly-error, .squiggly-warning")
  191 |       .count();
  192 |     if (squigglies > 0) return;
  193 |     await page.waitForTimeout(500);
  194 |   }
  195 | }
  196 | 
  197 | /**
  198 |  * Type text into the current editor at the cursor position.
  199 |  */
  200 | export async function typeText(page: Page, text: string): Promise<void> {
  201 |   await page.keyboard.type(text, { delay: 50 });
  202 | }
  203 | 
  204 | /**
  205 |  * Select all text in the current editor.
  206 |  */
  207 | export async function selectAll(page: Page): Promise<void> {
  208 |   await page.keyboard.press("Control+a");
  209 | }
  210 | 
  211 | /**
  212 |  * Get the currently active editor tab name.
  213 |  */
  214 | export async function getActiveTabName(page: Page): Promise<string | null> {
  215 |   const tab = page.locator(".tab.active .label-name");
  216 |   if (await tab.isVisible().catch(() => false)) {
  217 |     return tab.textContent();
  218 |   }
  219 |   return null;
  220 | }
  221 | 
```