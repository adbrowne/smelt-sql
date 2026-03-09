import * as path from 'path';
import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    Executable
} from 'vscode-languageclient/node';

let client: LanguageClient;

export function activate(context: vscode.ExtensionContext) {
    console.log('smelt extension activating...');

    // Get workspace folder
    const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
    if (!workspaceFolder) {
        vscode.window.showErrorMessage('smelt: No workspace folder found');
        return;
    }

    // Find the smelt project root (where Cargo.toml is)
    const smeltRoot = findSmeltRoot(workspaceFolder.uri.fsPath);
    if (!smeltRoot) {
        vscode.window.showErrorMessage('smelt: Could not find smelt project root (Cargo.toml)');
        return;
    }

    // Get server path from configuration or use default
    const config = vscode.workspace.getConfiguration('smelt');
    const serverPath = config.get<string>('serverPath');

    let serverCommand: Executable;

    if (serverPath && serverPath.length > 0) {
        // Use pre-built binary
        serverCommand = {
            command: serverPath,
            args: []
        };
    } else {
        // Use cargo run
        serverCommand = {
            command: 'cargo',
            args: ['run', '--manifest-path', path.join(smeltRoot, 'Cargo.toml'), '-p', 'smelt-lsp'],
            options: {
                cwd: smeltRoot
            }
        };
    }

    const serverOptions: ServerOptions = serverCommand;

    // Options to control the language client
    const clientOptions: LanguageClientOptions = {
        documentSelector: [
            { scheme: 'file', pattern: '**/models/**/*.sql' }
        ],
        synchronize: {
            fileEvents: [
                vscode.workspace.createFileSystemWatcher('**/models/**/*.sql'),
                vscode.workspace.createFileSystemWatcher('**/models/**/*.py'),
            ]
        },
        workspaceFolder: workspaceFolder,
        outputChannelName: 'smelt Language Server'
    };

    // Create the language client
    client = new LanguageClient(
        'smelt',
        'smelt Language Server',
        serverOptions,
        clientOptions
    );

    // Start the client (this will also launch the server)
    client.start().then(() => {
        console.log('smelt language server started successfully');
        vscode.window.showInformationMessage('smelt language server is running');
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
            if (require('fs').existsSync(cargoPath)) {
                const content = require('fs').readFileSync(cargoPath, 'utf-8');

                // Check if this is the smelt project by looking for:
                // 1. Direct mention of smelt-lsp in Cargo.toml, OR
                // 2. Workspace with crates/smelt-lsp directory
                if (content.includes('smelt-lsp')) {
                    return currentPath;
                }

                if (content.includes('[workspace]')) {
                    const lspPath = path.join(currentPath, 'crates', 'smelt-lsp');
                    if (require('fs').existsSync(lspPath)) {
                        return currentPath;
                    }
                }
            }
        } catch (e) {
            // Continue searching
        }
        currentPath = path.dirname(currentPath);
    }

    return null;
}
