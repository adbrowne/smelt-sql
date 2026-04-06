/**
 * Screenshot and video capture helpers with optional annotations.
 */
import { type Page } from "@playwright/test";
import * as path from "node:path";
import * as fs from "node:fs";
import { execSync } from "node:child_process";

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

// ---------------------------------------------------------------------------
// Video capture helpers
// ---------------------------------------------------------------------------

/**
 * Tracks timing for video capture. Create one at the start of the test,
 * then call markDemoStart() when the interesting actions begin.
 */
export class VideoTimer {
  private readonly pageCreatedAt: number;
  private demoStartAt: number | null = null;
  private demoEndAt: number | null = null;

  constructor() {
    this.pageCreatedAt = Date.now();
  }

  /** Mark when the demo actions begin (after setup/priming). */
  markDemoStart(): void {
    this.demoStartAt = Date.now();
  }

  /** Mark when the demo actions end. If not called, uses saveVideo() call time. */
  markDemoEnd(): void {
    this.demoEndAt = Date.now();
  }

  /** Get the trim start time in seconds (relative to video start). */
  get startSec(): number {
    if (!this.demoStartAt) return 0;
    return Math.max(0, (this.demoStartAt - this.pageCreatedAt) / 1000 - 0.5);
  }

  /** Get the demo duration in seconds. */
  durationSec(now?: number): number {
    const end = this.demoEndAt ?? now ?? Date.now();
    const start = this.demoStartAt ?? this.pageCreatedAt;
    return (end - start) / 1000 + 0.5;
  }
}

export interface SaveVideoOptions {
  /** Feature name — used as subdirectory under media/ */
  feature: string;
  /** Filename (without extension) */
  name: string;
  /** VideoTimer instance with demo start/end markers */
  timer: VideoTimer;
  /** Crop to editor area bounding box in viewport coordinates (optional) */
  crop?: { x: number; y: number; width: number; height: number };
  /** Viewport size — needed to scale crop coordinates to video resolution */
  viewportSize?: { width: number; height: number };
  /** Output fps (default: 12) */
  fps?: number;
  /** Max output width in pixels (default: 960) */
  maxWidth?: number;
}

/**
 * Save the Playwright auto-recorded video as a trimmed, cropped gif.
 *
 * Must be called AFTER `page.close()` so the video file is finalized.
 * The page object is still usable for `page.video()?.path()` after close.
 *
 * @returns Path to the generated gif.
 */
export async function saveVideo(
  page: Page,
  opts: SaveVideoOptions
): Promise<string> {
  const dir = mediaDir(opts.feature);
  const gifPath = path.join(dir, `${opts.name}.gif`);

  // Get the raw video path — available after page.close()
  const video = page.video();
  if (!video) throw new Error("No video recording on this page");

  const rawPath = await video.path();
  if (!rawPath || !fs.existsSync(rawPath)) {
    throw new Error(`Video file not found: ${rawPath}`);
  }

  const ss = opts.timer.startSec;
  const duration = opts.timer.durationSec();
  const fps = opts.fps ?? 12;
  const maxWidth = opts.maxWidth ?? 960;

  // Probe the actual video resolution — Playwright may record at a scaled-down size
  let videoWidth = 0;
  let videoHeight = 0;
  try {
    const probe = execSync(
      `ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 "${rawPath}"`,
      { encoding: "utf-8" }
    ).trim();
    const [w, h] = probe.split(",").map(Number);
    videoWidth = w;
    videoHeight = h;
  } catch {
    // If probe fails, skip cropping
  }

  // Build ffmpeg filter chain
  const filters: string[] = [];
  if (opts.crop && videoWidth > 0 && opts.viewportSize) {
    // Scale crop coordinates from viewport space to video space
    const scaleX = videoWidth / opts.viewportSize.width;
    const scaleY = videoHeight / opts.viewportSize.height;
    const cx = Math.floor((opts.crop.x * scaleX) / 2) * 2;
    const cy = Math.floor((opts.crop.y * scaleY) / 2) * 2;
    const cw = Math.floor((opts.crop.width * scaleX) / 2) * 2;
    const ch = Math.floor((opts.crop.height * scaleY) / 2) * 2;
    if (cx + cw <= videoWidth && cy + ch <= videoHeight && cw > 0 && ch > 0) {
      filters.push(`crop=${cw}:${ch}:${cx}:${cy}`);
    }
  }
  filters.push(`fps=${fps}`);
  filters.push(`scale=${maxWidth}:-1:flags=lanczos`);

  // Two-pass gif for good color quality
  const palettePath = path.join(dir, `${opts.name}-palette.png`);
  const filterStr = filters.join(",");

  // Pass 1: generate palette
  execSync(
    `ffmpeg -y -ss ${ss} -t ${duration} -i "${rawPath}" ` +
      `-vf "${filterStr},palettegen=stats_mode=diff" "${palettePath}"`,
    { stdio: "pipe" }
  );

  // Pass 2: generate gif using palette
  execSync(
    `ffmpeg -y -ss ${ss} -t ${duration} -i "${rawPath}" -i "${palettePath}" ` +
      `-lavfi "${filterStr}[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=5" "${gifPath}"`,
    { stdio: "pipe" }
  );

  // Clean up palette
  fs.unlinkSync(palettePath);

  return gifPath;
}

/**
 * Get the bounding box of the editor area for cropping videos.
 * Call this BEFORE page.close().
 */
export async function getEditorBounds(
  page: Page
): Promise<{ x: number; y: number; width: number; height: number } | null> {
  const editor = page.locator(".editor-container").first();
  if (await editor.isVisible().catch(() => false)) {
    const box = await editor.boundingBox();
    if (box) {
      // Round to even numbers (required by many video codecs)
      return {
        x: Math.floor(box.x / 2) * 2,
        y: Math.floor(box.y / 2) * 2,
        width: Math.floor(box.width / 2) * 2,
        height: Math.floor(box.height / 2) * 2,
      };
    }
  }
  return null;
}
