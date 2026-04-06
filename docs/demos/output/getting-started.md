# Getting Started with smelt's Editor Support

smelt includes a Language Server that gives you real-time feedback as you write SQL. Here's what it looks like in action.

## 1. Catch Errors Instantly

Typos, undefined references, and undeclared columns are flagged as you type — no need to run the pipeline first.

![Diagnostics](media/diagnostics/typo-caught-instantly.gif)

## 2. Navigate Your Pipeline

Jump from any `smelt.ref()` call directly to the upstream model. Trace data lineage without leaving your editor.

![Go-to-definition](media/goto-definition/trace-pipeline.gif)

## 3. Build Queries Faster

Get context-aware completions for model names, source names, and columns.

![Completions](media/completion/build-query-with-completions.gif)

## Next Steps

- See the [full feature showcase](lsp-features.md) for all LSP capabilities
- Follow the [Editor Setup guide](../../docs-site/docs/guide/editor-setup.md) to configure your editor
