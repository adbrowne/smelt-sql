#!/usr/bin/env -S npx tsx
/**
 * Generate standalone markdown documentation from the demo media assets.
 *
 * Outputs:
 *   docs/demos/output/lsp-features.md   — Full feature showcase (for GitHub viewing)
 *   docs/demos/output/getting-started.md — Quick-start guide with the 3 best gifs
 *
 * Usage:
 *   npx tsx docs/demos/generate-docs.ts
 */

import * as fs from "fs";
import * as path from "path";

const MEDIA_DIR = path.resolve(__dirname, "media");
const OUTPUT_DIR = path.resolve(__dirname, "output");

interface FeatureSection {
  title: string;
  slug: string;
  description: string;
  gif?: string;
  screenshots: string[];
}

const features: FeatureSection[] = [
  {
    title: "Real-Time Diagnostics",
    slug: "diagnostics",
    description:
      "Catch errors the moment you make them. smelt validates SQL continuously: " +
      "undefined model references, undeclared columns, type mismatches, and parse errors " +
      "all surface as you type.",
    gif: "typo-caught-instantly.gif",
    screenshots: ["01-clean-pipeline.png", "04-undeclared-column.png"],
  },
  {
    title: "Go-to-Definition",
    slug: "goto-definition",
    description:
      "Trace data lineage by jumping directly to definitions — from a smelt.ref() call " +
      "to the upstream model, from a column reference to where it's defined, or from a " +
      "CTE usage to its definition.",
    gif: "trace-pipeline.gif",
    screenshots: ["05-cte-definition-jump.png"],
  },
  {
    title: "Hover Information",
    slug: "hover",
    description:
      "Hover over any model reference, source, or CTE to see its full schema — " +
      "column names, types, and where the data comes from.",
    screenshots: [
      "01-model-schema-on-hover-editor.png",
      "02-upstream-schema-lineage.png",
      "03-source-schema-on-hover-editor.png",
    ],
  },
  {
    title: "Code Completion",
    slug: "completion",
    description:
      "Build queries faster with context-aware completions. smelt suggests model names, " +
      "source names, and column names based on upstream schemas.",
    gif: "build-query-with-completions.gif",
    screenshots: ["01-model-name-completions-editor.png"],
  },
  {
    title: "Find References",
    slug: "references",
    description:
      'Answer "who uses this model?" with a single keystroke. Find References shows ' +
      "every downstream consumer of a model or every usage of a CTE within a file.",
    screenshots: [
      "01-find-model-consumers-editor.png",
      "02-find-cte-references-editor.png",
    ],
  },
  {
    title: "Rename Refactoring",
    slug: "rename",
    description:
      "Rename a model and have every reference across the project update automatically. " +
      "The LSP shows a preview of all changes before applying them.",
    gif: "rename-model-across-project.gif",
    screenshots: ["01-rename-preview-editor.png"],
  },
  {
    title: "Code Actions & Quick Fixes",
    slug: "code-actions",
    description:
      "When smelt detects an error, it often suggests a fix. Reference a model that " +
      "doesn't exist yet? smelt offers to create the SQL file for you.",
    gif: "create-model-from-ref.gif",
    screenshots: ["01-create-model-quickfix-editor.png"],
  },
];

function mediaPath(slug: string, filename: string): string {
  return `media/${slug}/${filename}`;
}

function scanMediaDir(): Map<string, string[]> {
  const result = new Map<string, string[]>();
  if (!fs.existsSync(MEDIA_DIR)) {
    console.error(`Media directory not found: ${MEDIA_DIR}`);
    process.exit(1);
  }
  for (const dir of fs.readdirSync(MEDIA_DIR)) {
    const fullPath = path.join(MEDIA_DIR, dir);
    if (fs.statSync(fullPath).isDirectory()) {
      const files = fs.readdirSync(fullPath).sort();
      result.set(dir, files);
    }
  }
  return result;
}

function generateFullPage(mediaMap: Map<string, string[]>): string {
  const lines: string[] = [];
  lines.push("# smelt LSP Features");
  lines.push("");
  lines.push(
    "Visual showcase of smelt's Language Server Protocol features. " +
      "All features work in real time as you edit — no build step required."
  );
  lines.push("");

  // Table of contents
  lines.push("## Contents");
  lines.push("");
  for (const feature of features) {
    lines.push(
      `- [${feature.title}](#${feature.title.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/-$/, "")})`
    );
  }
  lines.push("");

  for (const feature of features) {
    lines.push(`## ${feature.title}`);
    lines.push("");
    lines.push(feature.description);
    lines.push("");

    if (feature.gif) {
      const p = mediaPath(feature.slug, feature.gif);
      if (
        mediaMap.get(feature.slug)?.includes(feature.gif) ||
        fs.existsSync(path.join(MEDIA_DIR, feature.slug, feature.gif))
      ) {
        lines.push(`![${feature.title}](${p})`);
        lines.push("");
      }
    }

    for (const screenshot of feature.screenshots) {
      const p = mediaPath(feature.slug, screenshot);
      const label = screenshot
        .replace(/^\d+-/, "")
        .replace(/\.png$/, "")
        .replace(/-/g, " ");
      if (
        mediaMap.get(feature.slug)?.includes(screenshot) ||
        fs.existsSync(path.join(MEDIA_DIR, feature.slug, screenshot))
      ) {
        lines.push(`![${label}](${p})`);
        lines.push("");
      }
    }

    // List all other files in the directory for completeness
    const allFiles = mediaMap.get(feature.slug) || [];
    const referenced = new Set([
      feature.gif,
      ...feature.screenshots,
    ].filter(Boolean));
    const extra = allFiles.filter((f) => !referenced.has(f));
    if (extra.length > 0) {
      lines.push("<details>");
      lines.push("<summary>All media in this section</summary>");
      lines.push("");
      for (const file of extra) {
        const p = mediaPath(feature.slug, file);
        if (file.endsWith(".gif")) {
          lines.push(`![${file}](${p})`);
        } else {
          lines.push(`![${file}](${p})`);
        }
        lines.push("");
      }
      lines.push("</details>");
      lines.push("");
    }
  }

  return lines.join("\n");
}

function generateGettingStarted(): string {
  const lines: string[] = [];
  lines.push("# Getting Started with smelt's Editor Support");
  lines.push("");
  lines.push(
    "smelt includes a Language Server that gives you real-time feedback as you write SQL. " +
      "Here's what it looks like in action."
  );
  lines.push("");

  lines.push("## 1. Catch Errors Instantly");
  lines.push("");
  lines.push(
    "Typos, undefined references, and undeclared columns are flagged as you type — " +
      "no need to run the pipeline first."
  );
  lines.push("");
  lines.push(
    `![Diagnostics](${mediaPath("diagnostics", "typo-caught-instantly.gif")})`
  );
  lines.push("");

  lines.push("## 2. Navigate Your Pipeline");
  lines.push("");
  lines.push(
    "Jump from any `smelt.ref()` call directly to the upstream model. " +
      "Trace data lineage without leaving your editor."
  );
  lines.push("");
  lines.push(
    `![Go-to-definition](${mediaPath("goto-definition", "trace-pipeline.gif")})`
  );
  lines.push("");

  lines.push("## 3. Build Queries Faster");
  lines.push("");
  lines.push(
    "Get context-aware completions for model names, source names, and columns."
  );
  lines.push("");
  lines.push(
    `![Completions](${mediaPath("completion", "build-query-with-completions.gif")})`
  );
  lines.push("");

  lines.push("## Next Steps");
  lines.push("");
  lines.push(
    "- See the [full feature showcase](lsp-features.md) for all LSP capabilities"
  );
  lines.push(
    "- Follow the [Editor Setup guide](../../docs-site/docs/guide/editor-setup.md) to configure your editor"
  );
  lines.push("");

  return lines.join("\n");
}

// Main
const mediaMap = scanMediaDir();
console.log("Scanned media directories:");
for (const [dir, files] of mediaMap) {
  console.log(`  ${dir}/: ${files.length} files`);
}

fs.mkdirSync(OUTPUT_DIR, { recursive: true });

const fullPage = generateFullPage(mediaMap);
const fullPagePath = path.join(OUTPUT_DIR, "lsp-features.md");
fs.writeFileSync(fullPagePath, fullPage);
console.log(`\nGenerated: ${fullPagePath}`);

const gettingStarted = generateGettingStarted();
const gettingStartedPath = path.join(OUTPUT_DIR, "getting-started.md");
fs.writeFileSync(gettingStartedPath, gettingStarted);
console.log(`Generated: ${gettingStartedPath}`);

console.log("\nDone.");
