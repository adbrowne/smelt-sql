# Web UI

smelt includes a built-in web interface for visualizing your model dependency graph and triggering runs.

## Launching the UI

```bash
smelt ui
```

By default, the UI is served at `http://127.0.0.1:3000`.

## Options

| Flag | Default | Description |
|---|---|---|
| `--host` | `127.0.0.1` | Host address to bind to. |
| `--port` | `3000` | Port to serve the UI on. |
| `--project-dir` | `.` | Path to the smelt project root. |

Example with custom host and port:

```bash
smelt ui --host 0.0.0.0 --port 8080
```

## Features

- **Dependency graph** -- Interactive visualization of your model DAG. See how models connect and identify upstream/downstream relationships at a glance.
- **Run execution** -- Trigger model runs directly from the UI without switching to the terminal.

!!! note
    The web UI is a newer feature and is under active development. Additional capabilities such as run history display and interval coverage visualization are planned.

## Further reading

- [Model Selection](model-selection.md) for filtering which models appear in the graph
- [Incremental Models](incremental-models.md) for monitoring incremental processing status via the CLI
