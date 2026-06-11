import * as path from 'path';
import * as fs from 'fs';
import { execFileSync, spawn } from 'child_process';
import * as vscode from 'vscode';
import { LanguageClient } from 'vscode-languageclient/node';

interface SmeltTestInfo {
    name: string;
    uri: string;
    line: number;
}

interface PublishTestsParams {
    tests: SmeltTestInfo[];
}

interface JsonTestResult {
    name: string;
    model: string;
    status: 'pass' | 'fail';
    duration_ms: number;
    message?: string;
}

interface JsonOutput {
    results: JsonTestResult[];
}

function findSmeltCommand(workspaceRoot: string, outputChannel: vscode.OutputChannel): string | null {
    const config = vscode.workspace.getConfiguration('smelt');

    const testPath = config.get<string>('testPath');
    if (testPath && testPath.length > 0) {
        outputChannel.appendLine(`smelt CLI discovery: using configured testPath: ${testPath}`);
        return testPath;
    }

    const whichCmd = process.platform === 'win32' ? 'where.exe' : 'which';
    try {
        const result = execFileSync(whichCmd, ['smelt'], {
            timeout: 5000,
            encoding: 'utf-8'
        }).trim().split('\n')[0];
        if (result) {
            outputChannel.appendLine(`smelt CLI discovery: found on PATH: ${result}`);
            return result;
        }
    } catch {
        // not on PATH
    }

    // Cargo fallback — development mode
    let current = workspaceRoot;
    const root = path.parse(current).root;
    while (current !== root) {
        const cargoPath = path.join(current, 'Cargo.toml');
        try {
            if (fs.existsSync(cargoPath)) {
                const content = fs.readFileSync(cargoPath, 'utf-8');
                if (content.includes('[workspace]') && fs.existsSync(path.join(current, 'crates', 'smelt-cli'))) {
                    outputChannel.appendLine(`smelt CLI discovery: using cargo run from ${current}`);
                    // Return a sentinel that runHandler recognises as the cargo fallback
                    return `__cargo__:${current}`;
                }
            }
        } catch {
            // continue
        }
        current = path.dirname(current);
    }

    return null;
}

function findProjectRoot(filePath: string): string | null {
    let current = path.dirname(filePath);
    const root = path.parse(current).root;
    while (current !== root) {
        if (fs.existsSync(path.join(current, 'smelt.yml')) || fs.existsSync(path.join(current, 'smelt.yaml'))) {
            return current;
        }
        current = path.dirname(current);
    }
    return null;
}

function refreshTestItems(
    controller: vscode.TestController,
    tests: SmeltTestInfo[]
): void {
    const incoming = new Map<string, SmeltTestInfo>(tests.map(t => [t.name, t]));

    // Remove items no longer in the list
    controller.items.forEach((item) => {
        if (!incoming.has(item.id)) {
            controller.items.delete(item.id);
        }
    });

    // Add or update items
    for (const info of tests) {
        const uri = vscode.Uri.parse(info.uri);
        const range = new vscode.Range(info.line, 0, info.line, 0);

        const existing = controller.items.get(info.name);
        if (existing && existing.uri?.toString() === uri.toString()) {
            // Same file — just update range (uri is readonly)
            existing.range = range;
        } else {
            // New item or file changed — recreate (uri is readonly after creation)
            const item = controller.createTestItem(info.name, info.name, uri);
            item.range = range;
            controller.items.add(item);
        }
    }
}

function runTests(
    run: vscode.TestRun,
    items: vscode.TestItem[],
    smeltCmd: string,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    // Group items by project root so we batch per-project
    const byRoot = new Map<string, vscode.TestItem[]>();
    for (const item of items) {
        const filePath = item.uri?.fsPath;
        const projectRoot = filePath ? findProjectRoot(filePath) : null;
        const key = projectRoot ?? '__unknown__';
        const group = byRoot.get(key) ?? [];
        group.push(item);
        byRoot.set(key, group);
    }

    const tasks: Promise<void>[] = [];

    for (const [projectRoot, groupItems] of byRoot) {
        if (projectRoot === '__unknown__') {
            for (const item of groupItems) {
                run.errored(item, new vscode.TestMessage('Could not determine smelt project root for this test'));
            }
            continue;
        }

        const task = new Promise<void>((resolve) => {
            const itemMap = new Map<string, vscode.TestItem>(groupItems.map(i => [i.id, i]));
            const selectArgs = groupItems.flatMap(i => ['--select', i.id]);

            let command: string;
            let args: string[];

            if (smeltCmd.startsWith('__cargo__:')) {
                const cargoRoot = smeltCmd.slice('__cargo__:'.length);
                command = 'cargo';
                args = ['run', '--manifest-path', path.join(cargoRoot, 'Cargo.toml'), '-p', 'smelt-cli', '--', 'test', '--json', ...selectArgs];
            } else {
                command = smeltCmd;
                args = ['test', '--json', ...selectArgs];
            }

            outputChannel.appendLine(`Running: ${command} ${args.join(' ')} (cwd: ${projectRoot})`);

            for (const item of groupItems) {
                run.started(item);
            }

            let stdout = '';
            const proc = spawn(command, args, { cwd: projectRoot });

            proc.stdout.on('data', (data: Buffer) => { stdout += data.toString(); });
            proc.stderr.on('data', (data: Buffer) => { outputChannel.appendLine(data.toString()); });

            proc.on('close', () => {
                try {
                    const output: JsonOutput = JSON.parse(stdout.trim());
                    for (const result of output.results) {
                        const item = itemMap.get(result.name);
                        if (!item) { continue; }
                        const duration = result.duration_ms;
                        if (result.status === 'pass') {
                            run.passed(item, duration);
                        } else {
                            run.failed(item, new vscode.TestMessage(result.message ?? 'test failed'), duration);
                        }
                    }
                    // Any items not mentioned in results = errored
                    const mentioned = new Set(output.results.map(r => r.name));
                    for (const [name, item] of itemMap) {
                        if (!mentioned.has(name)) {
                            run.errored(item, new vscode.TestMessage('No result returned for this test'));
                        }
                    }
                } catch (e) {
                    outputChannel.appendLine(`Failed to parse smelt test JSON output: ${e}\nRaw output: ${stdout}`);
                    for (const item of groupItems) {
                        run.errored(item, new vscode.TestMessage(`Failed to parse test output: ${e}`));
                    }
                }
                resolve();
            });

            proc.on('error', (err) => {
                for (const item of groupItems) {
                    run.errored(item, new vscode.TestMessage(`Failed to spawn smelt: ${err.message}`));
                }
                resolve();
            });
        });

        tasks.push(task);
    }

    return Promise.all(tasks).then(() => undefined);
}

export function createSmeltTestController(
    context: vscode.ExtensionContext,
    client: LanguageClient,
    workspaceRoot: string,
    outputChannel: vscode.OutputChannel
): vscode.TestController {
    const controller = vscode.tests.createTestController('smelt', 'smelt Tests');

    controller.createRunProfile(
        'Run',
        vscode.TestRunProfileKind.Run,
        async (request, token) => {
            const smeltCmd = findSmeltCommand(workspaceRoot, outputChannel);
            if (!smeltCmd) {
                vscode.window.showErrorMessage(
                    'smelt: Could not find smelt CLI binary. Add smelt to your PATH or set smelt.testPath in settings.'
                );
                return;
            }

            // Collect the items to run: either the explicit inclusion list or all items
            const toRun: vscode.TestItem[] = [];
            if (request.include) {
                for (const item of request.include) {
                    toRun.push(item);
                }
            } else {
                controller.items.forEach(item => toRun.push(item));
            }

            const run = controller.createTestRun(request);
            try {
                await runTests(run, toRun, smeltCmd, outputChannel);
            } finally {
                run.end();
            }
        },
        true
    );

    // Subscribe to LSP test-discovery notifications
    client.onNotification('smelt/publishTests', (params: PublishTestsParams) => {
        outputChannel.appendLine(`smelt/publishTests received: ${params.tests.length} test(s)`);
        refreshTestItems(controller, params.tests);
    });

    context.subscriptions.push(controller);
    return controller;
}
