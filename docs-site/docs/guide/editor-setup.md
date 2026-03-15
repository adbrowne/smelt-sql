# Editor Setup

smelt provides a Language Server Protocol (LSP) implementation for real-time feedback in your editor.

## VSCode

Install the **smelt** extension from the [VS Marketplace](https://marketplace.visualstudio.com/items?itemName=smelt.smelt).

The extension automatically discovers the `smelt-lsp` binary using this priority chain:

1. **User config** — `smelt.serverPath` setting in VSCode
2. **Python environment** — if you installed via `pip install smelt-sql`
3. **PATH lookup** — if `smelt-lsp` is on your system PATH
4. **Cargo fallback** — `cargo run -p smelt-lsp` (development only, requires Cargo.toml)

### Features

- Syntax highlighting for SQL + smelt extensions
- Real-time diagnostics for undefined refs and parse errors
- Go-to-definition for `smelt.ref()` calls
- Hover information with type details
- Column completions with table alias support

### Settings

| Setting | Description |
|---------|-------------|
| `smelt.serverPath` | Explicit path to `smelt-lsp` binary |
| `smelt.trace.server` | LSP trace level (`off`, `messages`, `verbose`) |

## Other editors

Any editor with LSP support can use `smelt-lsp`. Start the server and configure your editor's LSP client to connect to it:

```bash
smelt-lsp
```

The server communicates over stdin/stdout using the standard LSP protocol.
