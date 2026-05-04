# Editor Setup

smelt provides a Language Server Protocol (LSP) implementation for real-time feedback in your editor.

!!! info "See it in action"
    For visual demos of every LSP feature -- diagnostics, go-to-definition, hover, completions, references, rename, and code actions -- see [Editor Features](editor-features.md).

## VSCode

Install the **smelt** extension from the [VS Marketplace](https://marketplace.visualstudio.com/items?itemName=smelt.smelt).

The extension automatically discovers the `smelt-lsp` binary using this priority chain:

1. **User config** — `smelt.serverPath` setting in VSCode
2. **Python environment** — if you installed via `pip install smelt-sql`
3. **PATH lookup** — if `smelt-lsp` is on your system PATH
4. **Cargo fallback** — `cargo run -p smelt-lsp` (development only, requires Cargo.toml)

### Features

- Syntax highlighting for SQL + smelt extensions
- Real-time diagnostics (see [Diagnostics](#diagnostics) below)
- Go-to-definition (see [Go-to-definition](#go-to-definition) below)
- Hover information with type details
- Column completions with table alias support

### Settings

| Setting | Description |
|---------|-------------|
| `smelt.serverPath` | Explicit path to `smelt-lsp` binary |
| `smelt.trace.server` | LSP trace level (`off`, `messages`, `verbose`) |

## Go-to-definition

The LSP supports go-to-definition for multiple targets:

| Cursor position | Jumps to |
|---|---|
| `smelt.model_name` | The referenced model's SQL file |
| `smelt.sources.schema.table` | The per-entity source `.yml` file |
| CTE name in FROM/JOIN | The CTE definition in the WITH clause |
| Table alias (e.g., `t` in `t.column`) | Where the alias is defined |
| Column reference | The column's definition in the upstream SELECT, CTE, or source `.yml` |

Column go-to-definition traces through `SELECT *` wildcards to find the original definition. If a column name is ambiguous (unqualified with multiple possible sources), multiple locations are returned.

## Diagnostics

The LSP reports errors and warnings in real time as you edit:

- **Parse errors** -- Syntax mistakes in SQL
- **Undefined refs** -- `smelt.model` where `model` does not exist
- **Undeclared columns** -- References to columns not present in upstream model schemas or source `.yml` declarations
- **Unsupported syntax** -- Features like PIVOT/UNPIVOT that smelt does not support

Undeclared column diagnostics catch typos and schema drift at edit time, before you run anything. For example:

```sql
SELECT visitor_id FROM smelt.sources.raw.sessions
-- Error: Column 'visitor_id' not found in source 'raw.sessions'
```

!!! tip
    Column diagnostics require that your per-entity source `.yml` files declare column names (types are optional). Without column declarations, the LSP cannot validate column references against external sources.

## Other editors

Any editor with LSP support can use `smelt-lsp`. Start the server and configure your editor's LSP client to connect to it:

```bash
smelt-lsp
```

The server communicates over stdin/stdout using the standard LSP protocol.
