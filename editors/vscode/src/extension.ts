import * as path from 'path';
import * as fs from 'fs';
import { execFileSync } from 'child_process';
import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    Executable
} from 'vscode-languageclient/node';

let client: LanguageClient;
let outputChannel: vscode.OutputChannel;

interface LspCommand {
    command: string;
    args: string[];
    options?: { cwd?: string };
}

/**
 * Discover the smelt-lsp binary using a priority chain:
 * 1. User config (smelt.serverPath setting)
 * 2. Python environment (pip-installed smelt-sql package)
 * 3. $PATH lookup (standalone binary install)
 * 4. Cargo fallback (development — only if Cargo.toml found)
 */
function findLspCommand(workspaceRoot: string): LspCommand | null {
    const config = vscode.workspace.getConfiguration('smelt');

    // 1. User config — explicit path to binary
    const serverPath = config.get<string>('serverPath');
    if (serverPath && serverPath.length > 0) {
        outputChannel.appendLine(`LSP discovery: using user-configured path: ${serverPath}`);
        return { command: serverPath, args: [] };
    }

    // 2. Python environment — pip-installed smelt-sql package
    for (const python of ['python3', 'python']) {
        try {
            const result = execFileSync(python, [
                '-c',
                'from smelt_sql import lsp_binary_path; print(lsp_binary_path())'
            ], { timeout: 5000, encoding: 'utf-8' }).trim();
            if (result && fs.existsSync(result)) {
                outputChannel.appendLine(`LSP discovery: found via ${python} smelt_sql package: ${result}`);
                return { command: result, args: [] };
            }
        } catch {
            // Python not available or smelt_sql not installed — continue
        }
    }

    // 3. $PATH lookup — standalone binary
    const whichCmd = process.platform === 'win32' ? 'where.exe' : 'which';
    try {
        const result = execFileSync(whichCmd, ['smelt-lsp'], {
            timeout: 5000,
            encoding: 'utf-8'
        }).trim().split('\n')[0];
        if (result) {
            outputChannel.appendLine(`LSP discovery: found on PATH: ${result}`);
            return { command: result, args: [] };
        }
    } catch {
        // Not on PATH — continue
    }

    // 4. Cargo fallback — development mode (only if smelt Cargo.toml exists)
    const smeltRoot = findSmeltRoot(workspaceRoot);
    if (smeltRoot) {
        outputChannel.appendLine(`LSP discovery: using cargo run from ${smeltRoot}`);
        return {
            command: 'cargo',
            args: ['run', '--manifest-path', path.join(smeltRoot, 'Cargo.toml'), '-p', 'smelt-lsp'],
            options: { cwd: smeltRoot }
        };
    }

    return null;
}

export function activate(context: vscode.ExtensionContext) {
    outputChannel = vscode.window.createOutputChannel('smelt Language Server');
    outputChannel.appendLine('smelt extension activating...');

    // Get workspace folder
    const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
    if (!workspaceFolder) {
        vscode.window.showErrorMessage('smelt: No workspace folder found');
        return;
    }

    const lspCommand = findLspCommand(workspaceFolder.uri.fsPath);
    if (!lspCommand) {
        vscode.window.showErrorMessage(
            'smelt: Could not find smelt-lsp binary. Install via `pip install smelt-sql`, ' +
            'add smelt-lsp to your PATH, or set smelt.serverPath in settings.'
        );
        return;
    }

    const serverCommand: Executable = {
        command: lspCommand.command,
        args: lspCommand.args,
        options: lspCommand.options ? { cwd: lspCommand.options.cwd } : undefined
    };

    const serverOptions: ServerOptions = serverCommand;

    const clientOptions: LanguageClientOptions = {
        documentSelector: [
            { scheme: 'file', pattern: '**/models/**/*.sql' },
            { scheme: 'file', pattern: '**/sources.yml' },
            { scheme: 'file', pattern: '**/sources.yaml' },
        ],
        synchronize: {
            fileEvents: [
                vscode.workspace.createFileSystemWatcher('**/models/**/*.sql'),
                vscode.workspace.createFileSystemWatcher('**/models/**/*.py'),
                vscode.workspace.createFileSystemWatcher('**/sources.{yml,yaml}'),
            ]
        },
        workspaceFolder: workspaceFolder,
        outputChannel: outputChannel
    };

    client = new LanguageClient(
        'smelt',
        'smelt Language Server',
        serverOptions,
        clientOptions
    );

    client.start().then(() => {
        outputChannel.appendLine('smelt language server started successfully');
    }).catch(err => {
        console.error('Failed to start smelt language server:', err);
        vscode.window.showErrorMessage(`smelt: Failed to start language server: ${err.message}`);
    });
}

export function deactivate(): Thenable<void> | undefined {
    if (!client) {
        return undefined;
    }
    return client.stop();
}

/**
 * Find the smelt project root by looking for Cargo.toml
 */
function findSmeltRoot(startPath: string): string | null {
    let currentPath = startPath;
    const root = path.parse(currentPath).root;

    while (currentPath !== root) {
        const cargoPath = path.join(currentPath, 'Cargo.toml');
        try {
            if (fs.existsSync(cargoPath)) {
                const content = fs.readFileSync(cargoPath, 'utf-8');

                if (content.includes('smelt-lsp')) {
                    return currentPath;
                }

                if (content.includes('[workspace]')) {
                    const lspPath = path.join(currentPath, 'crates', 'smelt-lsp');
                    if (fs.existsSync(lspPath)) {
                        return currentPath;
                    }
                }
            }
        } catch {
            // Continue searching
        }
        currentPath = path.dirname(currentPath);
    }

    return null;
}
