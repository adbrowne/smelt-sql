# Installation

## pip install (recommended)

The easiest way to install smelt is via pip. This installs native binaries for your platform — no Rust toolchain required.

```bash
pip install smelt-sql
```

This provides both the `smelt` CLI and the `smelt-lsp` language server.

## Standalone binaries

Download pre-built binaries from the [GitHub Releases](https://github.com/adbrowne/smelt-sql/releases) page.

Available platforms:

- Linux x86_64
- Linux aarch64
- macOS x86_64 (Intel)
- macOS aarch64 (Apple Silicon)
- Windows x86_64

Extract the archive and add the directory to your `PATH`.

## Build from source

Requires the [Rust toolchain](https://rustup.rs/).

```bash
git clone https://github.com/adbrowne/smelt-sql.git
cd smelt-sql
cargo build --release
```

Binaries will be in `target/release/`:

- `smelt` — the CLI
- `smelt-lsp` — the language server
