# docs-site

Documentation site for smelt, built with [MkDocs Material](https://squidfunk.github.io/mkdocs-material/).

## Setup

Dependencies are managed via uv with a local `pyproject.toml`.

```bash
cd docs-site
uv sync
```

## Commands

```bash
# Build the site (output to site/)
uv run mkdocs build

# Live preview with hot reload
uv run mkdocs serve
```

## Structure

- `mkdocs.yml` — Site configuration and nav structure
- `docs/` — Markdown source files
- `site/` — Build output (gitignored)
