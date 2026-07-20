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
| `--host` | `127.0.0.1` | Host address to bind to. Binding to a non-loopback address requires `--allow-remote`. |
| `--port` | `3000` | Port to serve the UI on. |
| `--allow-remote` | off | Required to bind `--host` to a non-loopback address; see [Network exposure](#network-exposure). |
| `--project-dir` | `.` | Path to the smelt project root. |

Example with a custom port on loopback (no extra flag needed):

```bash
smelt ui --port 8080
```

Example binding to all interfaces, which requires the explicit opt-in:

```bash
smelt ui --host 0.0.0.0 --port 8080 --allow-remote
```

## Network exposure

`smelt ui` has no authentication and no HTTPS. It is designed to be used from the machine running it, over `http://127.0.0.1:<port>`. Anyone who can reach the bound address can read your project's models and trigger runs, so binding to a non-loopback host (`0.0.0.0`, a LAN IP, etc.) always requires `--allow-remote` — without it, `smelt ui` refuses to start rather than silently falling back to loopback — and logs a startup warning once bound. If you need remote access, prefer tunneling over SSH or a VPN to a loopback bind rather than exposing the server directly.

## Features

### Dependency graph

Interactive visualization of your model DAG using React Flow. See how models connect and identify upstream/downstream relationships at a glance. Click a model node to open its detail panel.

### Model detail panel

Click any model in the graph to see:

- **Columns** with inferred types and nullability
- **Upstream and downstream** dependencies
- **Incremental configuration** (granularity, partition column, etc.)
- **Batch safety** indicators (green/yellow/red badges)
- **Diagnostics** -- parse errors, type errors, and undeclared column warnings

### Run planner

Plan runs interactively before executing:

- Select models to include (with tag filtering)
- Set time range for incremental models
- Configure batch size and per-partition execution
- Preview the execution plan with per-batch detail before committing

### Run execution and monitoring

Execute runs directly from the UI with real-time progress:

- **Progress bars** for each model and batch
- **Live event log** streamed via WebSocket
- **Cancellation** -- stop a running pipeline between batches
- **Run history** -- browse past runs with expandable details

Only one run can execute at a time.

### Live file watching

The UI watches your project files for changes. When you save a model or configuration file, the UI automatically recompiles and updates the graph, diagnostics, and model details -- no manual refresh needed.

## Further reading

- [Model Selection](model-selection.md) for filtering which models appear in the graph
- [Incremental Models](incremental-models.md) for monitoring incremental processing status via the CLI
- [CLI Commands](../reference/cli.md#smelt-ui) for all `smelt ui` flags
