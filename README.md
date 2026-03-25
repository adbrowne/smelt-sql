# smelt

Modern data transformation framework.

[Documentation](https://smeltsql.com/)

Andrew's Christmas 2025 holiday project.

## What is smelt?

smelt is a data transformation framework that separates **logical transformation definitions** from **physical execution planning**. Unlike dbt which uses Jinja templates, smelt parses and understands the semantics of your SQL, enabling:

- **Automatic incrementalization** - Framework analyzes what's safe to run incrementally
- **Cross-engine deployment** - Split work across DuckDB, Spark, Postgres, etc.
- **Static analysis** - Type checking, LSP support, and semantic validation
- **Rule-based optimization** - Automatic rewrites with learning from history

## Quick Example

```sql
-- models/daily_revenue.sql
---
name: daily_revenue
materialization: table
incremental:
  enabled: true
  event_time_column: order_date
  partition_column: order_date
tags: [revenue, daily]
---

SELECT
  order_date,
  customer_id,
  SUM(amount) as daily_revenue
FROM smelt.ref('orders')
WHERE order_date IS NOT NULL
GROUP BY 1, 2
```

```bash
# Run all models
smelt run

# Run incrementally (only process new data)
smelt run --event-time-start 2025-01-01 --event-time-end 2025-01-02

# Preview what would run
smelt run --dry-run --verbose
```

## Current Status

**Phase 1 Complete (December 2025)**: YAML frontmatter metadata support
- ✅ Single-model files with frontmatter
- ✅ Multi-model files with section delimiters
- ✅ SQL-first configuration precedence
- ✅ Full backward compatibility

**Phases 1-9 Complete (December 2024)**: Parser, LSP, CLI with multi-backend support (DuckDB + Spark stub), and basic incremental materialization.

See [docs/ROADMAP.md](docs/ROADMAP.md) for implementation status.

## Documentation

- **[docs/DESIGN.md](docs/DESIGN.md)** - Full language specification and architecture
- **[docs/ROADMAP.md](docs/ROADMAP.md)** - Implementation status and next steps
- **[docs/lsp_quickstart.md](docs/lsp_quickstart.md)** - Getting started with the LSP
- **[docs/architecture_overview.md](docs/architecture_overview.md)** - System design

## Getting Started

```bash
# Build
cargo build

# Run tests
cargo test

# Run the CLI
cargo run -p smelt-cli -- run --project-dir examples/timeseries

# Run the LSP server
cargo run -p smelt-lsp
```

## Model Configuration

smelt supports **YAML frontmatter** in SQL files for model-level configuration. This keeps configuration close to code while maintaining full SQL compatibility.

### Single-Model File

```sql
---
name: user_summary
materialization: table
incremental:
  enabled: true
  event_time_column: updated_at
  partition_column: summary_date
tags: [users, daily]
owner: analytics-team
description: Daily user activity summary
---

SELECT
  DATE(updated_at) as summary_date,
  user_id,
  COUNT(DISTINCT session_id) as session_count,
  SUM(revenue) as total_revenue
FROM smelt.ref('sessions')
WHERE updated_at IS NOT NULL
GROUP BY 1, 2
```

### Multi-Model File

You can define multiple models in a single file using section delimiters:

```sql
--- name: raw_events ---
materialization: table
description: Raw event stream
---

SELECT event_id, user_id, event_time, event_type
FROM source.events

--- name: processed_events ---
materialization: view
tags: [derived]
---

SELECT
  event_id,
  user_id,
  DATE(event_time) as event_date,
  event_type
FROM smelt.ref('raw_events')
WHERE event_type IS NOT NULL
```

### Configuration Precedence

**SQL file metadata > smelt.yml > defaults**

Frontmatter in SQL files overrides `smelt.yml` configuration, allowing you to:
- Keep project-wide defaults in `smelt.yml`
- Override per-model settings in SQL frontmatter
- Gradually migrate from `smelt.yml` to inline configuration

### Supported Metadata Fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Model name (optional in single-model files) |
| `materialization` | `table` \| `view` | How to materialize the model |
| `incremental.enabled` | boolean | Enable incremental updates |
| `incremental.event_time_column` | string | Column for time-based filtering |
| `incremental.partition_column` | string | Column for partition deletion |
| `tags` | string[] | Tags for organization |
| `owner` | string | Team or person responsible |
| `description` | string | Model documentation |
| `backend_hints` | object | Backend-specific settings (future) |

### Backward Compatibility

Files without frontmatter continue to work:
- Model name inferred from filename (`user_summary.sql` → `user_summary`)
- Configuration loaded from `smelt.yml`
- No breaking changes to existing projects

## Key Differentiators from dbt

| Aspect | dbt | smelt |
|--------|-----|-------|
| Model definition | Jinja templates | Parsed semantic models |
| Incrementalization | Manual `is_incremental()` | Automatic semantic analysis |
| Type checking | None (runtime errors) | Static analysis with LSP |
| Cross-engine | One target per project | Split work across engines |
| Optimization | Manual | Rule-based with learning |

## License

MIT License - Copyright (c) 2025 Andrew Browne
