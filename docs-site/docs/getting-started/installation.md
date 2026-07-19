# Installation

## pip install (recommended)

The easiest way to install smelt is via pip. This installs native binaries for your platform — no Rust toolchain required.

```bash
pip install smelt-sql
```

This provides both the `smelt` CLI and the `smelt-lsp` language server.

### Platform support

Pre-built wheels are available for:

- Linux x86_64
- Linux aarch64 (ARM64)
- macOS aarch64 (Apple Silicon)
- Windows x86_64

**Python 3.9–3.14** are supported. If your platform isn't listed, pip will automatically build from source using the sdist — a Rust toolchain is required in that case.

## Standalone binaries

Download pre-built binaries from the [GitHub Releases](https://github.com/adbrowne/smelt-sql/releases) page.

Available platforms:

- Linux x86_64
- Linux aarch64
- macOS aarch64 (Apple Silicon)
- Windows x86_64

Extract the archive and add the directory to your `PATH`.

Intel Macs (macOS x86_64) do not have a prebuilt standalone binary; install
via `pip install smelt-sql` instead, which builds from source using the
sdist (a Rust toolchain is required).

## Docker

A container image is published to GitHub Container Registry on every release:

```bash
docker run --rm -v "$PWD":/workspace -w /workspace ghcr.io/adbrowne/smelt:latest build
```

Pin to a specific version instead of `latest` for reproducible CI runs:

```bash
docker run --rm ghcr.io/adbrowne/smelt:0.5.0 --version
```

The image bundles `smelt` and the DuckDB shared library it links against; mount
your project directory as a volume and run any `smelt` subcommand as the
container's entrypoint.

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
