# sqt VSCode Extension

Language support for sqt (Semantic Query Tool) data pipeline models.

## Features

- **Syntax Highlighting**: Highlights SQL keywords and sqt template syntax (`{{ ref() }}`, `{{ config() }}`)
- **Diagnostics**: Shows errors for undefined model references
- **Go to Definition**: Ctrl+Click (Cmd+Click on Mac) on `{{ ref('model_name') }}` to jump to the model definition
- **Incremental Updates**: Fast feedback powered by Salsa incremental compilation

## Requirements

- **Rust toolchain**: Required to build the language server
- **sqt project**: The extension activates when it detects a workspace with `models/*.sql` files

## Installation

### Option 1: Install from VSIX (Recommended for testing)

1. Build the extension:
   ```bash
   cd editors/vscode
   npm install
   npm run compile
   npm run package
   ```

2. Install the generated `.vsix` file in VSCode:
   - Open VSCode
   - Go to Extensions view (Cmd+Shift+X)
   - Click "..." menu → "Install from VSIX..."
   - Select `sqt-0.1.0.vsix`

### Option 2: Development Mode

1. Install dependencies:
   ```bash
   cd editors/vscode
   npm install
   ```

2. Open the `editors/vscode` folder in VSCode

3. Press F5 to launch Extension Development Host

4. Open a workspace containing sqt models

## Usage

### Project Structure

Your sqt project should have this structure:

```
my-project/
├── models/
│   ├── raw_events.sql
│   ├── user_sessions.sql
│   └── user_stats.sql
└── (rest of your project)
```

### Features in Action

**Diagnostics:**
```sql
-- This will show an error
SELECT * FROM {{ ref('nonexistent_model') }}
```

**Go to Definition:**
```sql
-- Ctrl+Click on 'raw_events' to jump to raw_events.sql
SELECT * FROM {{ ref('raw_events') }}
```

**Syntax Highlighting:**
```sql
-- Template expressions are highlighted
{{ config(materialized='table') }}

SELECT user_id, COUNT(*)
FROM {{ ref('events') }}
GROUP BY user_id
```

## Configuration

Access settings via: Preferences → Settings → Extensions → sqt

- **sqt.serverPath**: Path to pre-built `sqt-lsp` binary (optional)
  - If not set, uses `cargo run -p sqt-lsp` (slower startup)
  - For better performance, build once and set path:
    ```bash
    cargo build --release -p sqt-lsp
    # Then set path to: target/release/sqt-lsp
    ```

- **sqt.trace.server**: Enable server communication tracing for debugging
  - Options: `off`, `messages`, `verbose`
  - Use `verbose` to debug LSP issues

## Troubleshooting

### Language server not starting

**Check Output:**
1. View → Output
2. Select "sqt Language Server" from dropdown
3. Look for error messages

**Common Issues:**

- **Cargo not found**: Ensure Rust toolchain is installed and in PATH
- **No workspace folder**: VSCode needs an open folder (not just files)
- **No models/ directory**: Create a `models/` directory with `.sql` files

### Diagnostics not showing

1. Check that file is in `models/` directory
2. Check file has `.sql` extension
3. Try saving the file (triggers re-analysis)
4. Check Output panel for errors

### Go-to-definition not working

1. Ensure the referenced model exists
2. Model file should be in `models/` directory
3. Model name should match filename (e.g., `{{ ref('users') }}` → `models/users.sql`)

## Development

To work on the extension:

```bash
cd editors/vscode

# Install dependencies
npm install

# Compile TypeScript
npm run compile

# Watch mode (auto-recompile on changes)
npm run watch

# Package for distribution
npm run package
```

## Known Limitations

- Parser is currently naive (string matching for `{{ ref() }}`)
- Diagnostics don't show exact line/column yet
- No auto-completions yet (coming soon)
- Full file sync (will add incremental sync)

## Roadmap

- [ ] Auto-completions for model names in `ref()`
- [ ] Hover to show model schema and dependencies
- [ ] Code actions for optimization suggestions
- [ ] Incremental file sync
- [ ] Better error messages with exact positions
- [ ] Syntax highlighting for nested SQL in templates

## License

MIT License - See LICENSE file for details
