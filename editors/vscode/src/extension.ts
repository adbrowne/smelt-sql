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
    console.log('sqt extension activating...');

    // Get workspace folder
    const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
    if (!workspaceFolder) {
        vscode.window.showErrorMessage('sqt: No workspace folder found');
        return;
    }

    // Find the sqt project root (where Cargo.toml is)
    const sqtRoot = findSqtRoot(workspaceFolder.uri.fsPath);
    if (!sqtRoot) {
        vscode.window.showErrorMessage('sqt: Could not find sqt project root (Cargo.toml)');
        return;
    }

    // Get server path from configuration or use default
    const config = vscode.workspace.getConfiguration('sqt');
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
            args: ['run', '--manifest-path', path.join(sqtRoot, 'Cargo.toml'), '-p', 'sqt-lsp'],
            options: {
                cwd: sqtRoot
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
            fileEvents: vscode.workspace.createFileSystemWatcher('**/models/**/*.sql')
        },
        workspaceFolder: workspaceFolder,
        outputChannelName: 'sqt Language Server'
    };

    // Create the language client
    client = new LanguageClient(
        'sqt',
        'sqt Language Server',
        serverOptions,
        clientOptions
    );

    // Start the client (this will also launch the server)
    client.start().then(() => {
        console.log('sqt language server started successfully');
        vscode.window.showInformationMessage('sqt language server is running');
    }).catch(err => {
        console.error('Failed to start sqt language server:', err);
        vscode.window.showErrorMessage(`sqt: Failed to start language server: ${err.message}`);
    });
}

export function deactivate(): Thenable<void> | undefined {
    if (!client) {
        return undefined;
    }
    return client.stop();
}

/**
 * Find the sqt project root by looking for Cargo.toml
 */
function findSqtRoot(startPath: string): string | null {
    let currentPath = startPath;
    const root = path.parse(currentPath).root;

    while (currentPath !== root) {
        const cargoPath = path.join(currentPath, 'Cargo.toml');
        try {
            if (require('fs').existsSync(cargoPath)) {
                // Check if this Cargo.toml contains sqt-lsp
                const content = require('fs').readFileSync(cargoPath, 'utf-8');
                if (content.includes('sqt-lsp')) {
                    return currentPath;
                }
            }
        } catch (e) {
            // Continue searching
        }
        currentPath = path.dirname(currentPath);
    }

    return null;
}
