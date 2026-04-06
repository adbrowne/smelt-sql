/**
 * Helpers for launching and managing code-server instances for Playwright tests.
 */
import { type ChildProcess, spawn } from "node:child_process";
import * as path from "node:path";
import * as net from "node:net";

/** Root of the smelt-sql repository */
export const REPO_ROOT = path.resolve(__dirname, "..", "..", "..");

/** Path to the demo workspace */
export const DEMO_WORKSPACE = path.join(
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
  stop(): void;
}

/**
 * Install the smelt VSIX extension into code-server.
 * This must be done as a separate step before launching the server,
 * because --install-extension causes code-server to exit after installing.
 */
async function installExtension(): Promise<void> {
  const codeServerBin = process.env.CODE_SERVER_BIN ?? "code-server";
  const vsix = vsixPath();

  return new Promise<void>((resolve, reject) => {
    const child = spawn(
      codeServerBin,
      ["--install-extension", vsix, "--force"],
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
 * Launch code-server pointing at the demo workspace with the smelt extension installed.
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

  // Install the extension first (separate step — code-server exits after install)
  if (!opts.skipExtensionInstall) {
    await installExtension();
  }

  const args: string[] = [
    "--port",
    String(port),
    "--auth",
    "none",
    "--disable-telemetry",
    "--disable-update-check",
    "--disable-workspace-trust",
    "--disable-getting-started-override",
    DEMO_WORKSPACE,
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
    stop() {
      child.kill("SIGTERM");
    },
  };
}
