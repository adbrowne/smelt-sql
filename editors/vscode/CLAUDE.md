# editors/vscode/CLAUDE.md

VSCode extension — language client that launches the `smelt-lsp` server, provides syntax highlighting for smelt SQL, and auto-activates when a `models/` directory is detected.

## How to build and test

```bash
cd editors/vscode
npm install
npm run compile       # one-shot TypeScript compile
npm run watch         # watch mode (auto-recompile on changes)

# Launch Extension Host for interactive testing
# Open the editors/vscode folder in VSCode and press F5.
# This opens a new window with the extension loaded.

# Package as VSIX (requires Node 18+)
npm run package
```

## Gotchas

- **The extension launches the LSP binary.** By default it resolves `smelt-lsp` from the workspace `target/debug/` or `target/release/` directory. If the binary is missing or stale, the extension activates but all LSP features are silently absent. Run `cargo build -p smelt-lsp` before pressing F5.
- **Multi-project discovery is done inside the LSP, not the extension.** If a feature works in `cargo test -p smelt-lsp --test example_workspaces` but not in VSCode, suspect the layer above the LSP first: the extension's workspace folder configuration, or multi-project discovery in `Backend::initialize`. See root `CLAUDE.md` §Architectural invariants — **Workspace Loading Parity** is the relevant rule.
- **`syntaxes/`** contains the TextMate grammar for smelt SQL syntax highlighting. Changes here take effect on the next extension reload (Ctrl+Shift+P → "Developer: Reload Window") without recompiling TypeScript.
- **`npm run package`** calls `vsce package` and requires `@vscode/vsce` (installed as a dev dep). The output is a `.vsix` file installable via "Install from VSIX" in VSCode.
- **TypeScript target is `ES2020`.** Check `tsconfig.json` before using very new JS features — the extension host may lag VS Code's Node version.

## Where things live

- `src/` — TypeScript extension source (`extension.ts` is the entry point)
- `syntaxes/` — TextMate grammar files for syntax highlighting
- `package.json` — extension manifest (activation events, contributes, dependencies)
- `language-configuration.json` — bracket matching, comment tokens, auto-closing pairs
