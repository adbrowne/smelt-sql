/**
 * Screenshot and video capture helpers with optional annotations.
 */
import { type Page } from "@playwright/test";
import * as path from "node:path";
import * as fs from "node:fs";

/** Base directory for generated media */
export const MEDIA_DIR = path.resolve(__dirname, "..", "media");

/**
 * Ensure a media subdirectory exists and return its path.
 */
export function mediaDir(feature: string): string {
  const dir = path.join(MEDIA_DIR, feature);
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

export interface ScreenshotOptions {
  /** Feature name — used as subdirectory under media/ */
  feature: string;
  /** Filename (without extension) */
  name: string;
  /** Optional clip region */
  clip?: { x: number; y: number; width: number; height: number };
  /** Whether to capture full page */
  fullPage?: boolean;
}

/**
 * Take a screenshot and save it to media/<feature>/<name>.png
 * @returns The path to the saved screenshot.
 */
export async function screenshot(
  page: Page,
  opts: ScreenshotOptions
): Promise<string> {
  const dir = mediaDir(opts.feature);
  const filePath = path.join(dir, `${opts.name}.png`);

  await page.screenshot({
    path: filePath,
    clip: opts.clip,
    fullPage: opts.fullPage ?? false,
  });

  return filePath;
}

/**
 * Take a screenshot focused on the editor area only (no sidebar/statusbar).
 * Falls back to full-page if the editor area can't be located.
 */
export async function screenshotEditor(
  page: Page,
  opts: Omit<ScreenshotOptions, "clip">
): Promise<string> {
  const dir = mediaDir(opts.feature);
  const filePath = path.join(dir, `${opts.name}.png`);

  // Try to screenshot just the editor area
  const editor = page.locator(".editor-container").first();
  if (await editor.isVisible().catch(() => false)) {
    await editor.screenshot({ path: filePath });
  } else {
    await page.screenshot({ path: filePath });
  }

  return filePath;
}

/**
 * Take a screenshot that includes the hover widget or other overlay.
 * Uses full-page screenshot to capture overlays that may be positioned absolutely.
 */
export async function screenshotWithOverlay(
  page: Page,
  opts: Omit<ScreenshotOptions, "clip" | "fullPage">
): Promise<string> {
  const dir = mediaDir(opts.feature);
  const filePath = path.join(dir, `${opts.name}.png`);

  await page.screenshot({
    path: filePath,
    fullPage: false,
  });

  return filePath;
}
