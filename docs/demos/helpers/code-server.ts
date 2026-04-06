/**
 * Helpers for launching and managing code-server instances for Playwright tests.
 */
import { type ChildProcess, spawn, execSync } from "node:child_process";
import * as path from "node:path";
import * as fs from "node:fs";
import * as os from "node:os";
import * as net from "node:net";

/** Root of the smelt-sql repository */
export const REPO_ROOT = path.resolve(__dirname, "..", "..", "..");

/** Path to the canonical demo workspace (read-only source of truth) */
export const DEMO_WORKSPACE_SOURCE = path.join(
  REPO_ROOT,
  "examples",
  "demo_workspace"
);

/** Default port for code-server */
export const DEFAULT_PORT = 18080;

/** Get the base URL for code-server */
export function codeServerURL(port = DEFAULT_PORT): string {
  return process.env.CODE_SERVER_URL ?? `http://localhost:${port}`;
}

/** Path to the packaged VSIX extension */
export function vsixPath(): string {
  return path.join(REPO_ROOT, "editors", "vscode", "smelt-0.1.0.vsix");
}

/**
 * Create a temporary copy of the demo workspace.
 * Tests that modify files (rename, code actions) operate on this copy
 * so we never need `git checkout` to restore the original.
 */
export function createWorkspaceCopy(): string {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "smelt-demo-"));
  const dest = path.join(tmpDir, "demo_workspace");
  execSync(`cp -r ${DEMO_WORKSPACE_SOURCE} ${dest}`);
  return dest;
}

/** Wait until a TCP port is accepting connections */
async function waitForPort(
  port: number,
  host = "127.0.0.1",
  timeoutMs = 30_000
): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      await new Promise<void>((resolve, reject) => {
        const sock = net.createConnection({ port, host }, () => {
          sock.destroy();
          resolve();
        });
        sock.on("error", reject);
      });
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 500));
    }
  }
  throw new Error(`Timed out waiting for port ${port}`);
}

export interface CodeServerHandle {
  process: ChildProcess;
  url: string;
  port: number;
  /** Path to the temporary workspace copy (cleaned up on stop) */
  workspacePath: string;
  /** Path to the temporary user-data-dir (cleaned up on stop) */
  userDataDir: string;
  stop(): void;
}

/**
 * Create a fresh user-data-dir with settings pre-configured for demos.
 * This avoids inheriting stale state from a previous session.
 */
function createUserDataDir(): string {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "code-server-data-"));
  fs.mkdirSync(path.join(tmpDir, "User"), { recursive: true });
  fs.writeFileSync(
    path.join(tmpDir, "User", "settings.json"),
    JSON.stringify(
      {
        "workbench.startupEditor": "none",
        "workbench.tips.enabled": false,
        "workbench.welcomePage.walkthroughs.openOnInstall": false,
        "chat.commandCenter.enabled": false,
        "chat.editor.enabled": false,
        "security.workspace.trust.enabled": false,
        "security.workspace.trust.startupPrompt": "never",
        "screencastMode.fontSize": 28,
        "screencastMode.verticalOffset": 5,
        "screencastMode.mouseIndicatorSize": 30,
      },
      null,
      2
    )
  );
  return tmpDir;
}

/**
 * Install the smelt VSIX extension into code-server.
 * This must be done as a separate step before launching the server,
 * because --install-extension causes code-server to exit after installing.
 */
async function installExtension(userDataDir: string): Promise<void> {
  const codeServerBin = process.env.CODE_SERVER_BIN ?? "code-server";
  const vsix = vsixPath();

  return new Promise<void>((resolve, reject) => {
    const child = spawn(
      codeServerBin,
      [
        "--user-data-dir",
        userDataDir,
        "--install-extension",
        vsix,
        "--force",
      ],
      {
        stdio: ["ignore", "pipe", "pipe"],
        env: {
          ...process.env,
          PATH: `${path.join(REPO_ROOT, "target", "debug")}:${process.env.PATH}`,
        },
      }
    );
    child.on("close", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`Extension install failed with code ${code}`));
    });
    child.on("error", reject);
  });
}

/**
 * Launch code-server pointing at a fresh copy of the demo workspace
 * with the smelt extension installed.
 *
 * Callers should call handle.stop() in afterAll / teardown.
 */
export async function launchCodeServer(
  opts: {
    port?: number;
    skipExtensionInstall?: boolean;
  } = {}
): Promise<CodeServerHandle> {
  const port = opts.port ?? DEFAULT_PORT;

  // Create isolated workspace copy and user-data-dir
  const workspacePath = createWorkspaceCopy();
  const userDataDir = createUserDataDir();

  // Install the extension first (separate step — code-server exits after install)
  if (!opts.skipExtensionInstall) {
    await installExtension(userDataDir);
  }

  const args: string[] = [
    "--user-data-dir",
    userDataDir,
    "--port",
    String(port),
    "--auth",
    "none",
    "--disable-telemetry",
    "--disable-update-check",
    "--disable-workspace-trust",
    "--disable-getting-started-override",
    workspacePath,
  ];

  const codeServerBin =
    process.env.CODE_SERVER_BIN ?? "code-server";

  const child = spawn(codeServerBin, args, {
    stdio: ["ignore", "pipe", "pipe"],
    env: {
      ...process.env,
      // Ensure the smelt LSP binary is on the PATH — built with cargo build -p smelt-lsp
      PATH: `${path.join(REPO_ROOT, "target", "debug")}:${process.env.PATH}`,
    },
  });

  // Log stderr for debugging
  child.stderr?.on("data", (chunk: Buffer) => {
    if (process.env.DEBUG) {
      process.stderr.write(`[code-server] ${chunk}`);
    }
  });

  await waitForPort(port);

  const url = `http://localhost:${port}`;
  return {
    process: child,
    url,
    port,
    workspacePath,
    userDataDir,
    stop() {
      child.kill("SIGTERM");
      // Clean up temp directories
      try {
        fs.rmSync(workspacePath, { recursive: true, force: true });
        fs.rmSync(userDataDir, { recursive: true, force: true });
      } catch {
        // Best effort
      }
    },
  };
}
