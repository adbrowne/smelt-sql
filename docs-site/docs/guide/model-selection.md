# Model Selection

When working with a project that has many models, you often want to run only a subset. smelt provides `--select` and `--exclude` flags to control which models are included in a run.

These flags are available on `smelt run`, `smelt build`, and `smelt explain`.

## Selection syntax

### By name

Select a single model by its full canonical path:

```bash
smelt run --select marts.daily_revenue
```

When working from inside a namespace directory, use `--scope` or rely on cwd auto-scoping to shorten the name (see [Argument resolution and `--scope`](../reference/cli.md#argument-resolution-and-scope)):

```bash
smelt --scope marts run --select daily_revenue
```

### By tag

Select all models with a given tag:

```bash
smelt run --select tag:revenue
```

Tags are assigned in `smelt.yml` or in YAML frontmatter:

```yaml
# smelt.yml
models:
  daily_revenue:
    tags: [revenue, daily]
```

```sql
---
tags: [revenue, daily]
---
SELECT ...
```

### Include upstream dependencies

Prefix with `+` to include the selected model(s) and all their upstream dependencies:

```bash
smelt run --select +marts.daily_revenue
smelt run --select +tag:revenue
```

This ensures that every model `marts.daily_revenue` depends on is also run, in the correct order.

### Include downstream dependents

Suffix with `+` to include the selected model(s) and everything that depends on them:

```bash
smelt run --select marts.daily_revenue+
smelt run --select tag:staging+
```

### Include both upstream and downstream

Combine the prefix and suffix:

```bash
smelt run --select +marts.daily_revenue+
```

This selects `marts.daily_revenue`, all its upstream dependencies, and all its downstream dependents.

### Multiple selectors

The `--select` flag is repeatable. All selectors are combined (union):

```bash
smelt run --select marts.daily_revenue --select silver.user_activity
```

Short form with `-s`:

```bash
smelt run -s marts.daily_revenue -s silver.user_activity
```

## Excluding models

The `--exclude` flag uses the same syntax as `--select` but removes models from the selection:

```bash
smelt run --exclude tag:expensive
smelt run --exclude silver.staging_legacy
```

Exclusions are applied after selections. If you use `--select` and `--exclude` together, smelt first builds the selected set, then removes the excluded models.

## Practical examples

### Run a single model

```bash
smelt run --select marts.daily_revenue
```

### Run a model and all its dependencies

```bash
smelt run --select +marts.daily_revenue
```

### Run everything downstream of staging

```bash
smelt run --select tag:staging+
```

### Run everything except expensive models

```bash
smelt run --exclude tag:expensive
```

### Run a specific model with dependencies, excluding one branch

```bash
smelt run --select +marts.daily_revenue --exclude silver.user_activity
```

### Explain selected models

```bash
smelt explain --select +marts.daily_revenue
smelt explain --select tag:revenue --json
```

## How selection works

1. If no `--select` flags are provided, all models are included.
2. Each `--select` flag adds models to the set. The `+` prefix/suffix expands the set by walking the dependency graph.
3. Each `--exclude` flag removes models from the set, using the same expansion rules.
4. The final set is executed in topological order (dependencies first).

!!! tip
    During development, use `--select +model_name` to run just the model you are working on along with its dependencies. This is much faster than running the entire project.

## Selector names and scope resolution

Selector values that are plain model names (not `tag:...` or `generator_file:...` selectors) go through the same scope resolution as other CLI arguments. A bare `--select events_parsed` expands to `--select silver.events_parsed` when the active scope is `silver`. An unresolvable bare selector with multiple matches is an error — smelt never silently picks one.

The safest form for scripts and CI is the full canonical path:

```bash
smelt run --select silver.events_parsed --select gold.eventstream_with_identity
```

When working interactively from inside a namespace directory, use `--scope` to shorten repeated names:

```bash
smelt --scope silver run --select events_parsed --select sessions
```

All smelt output — including selector match lists and run summaries — always uses the canonical dot-path (e.g. `silver.events_parsed`) regardless of how you specified the selector. Copy-pasting a printed name back into a subsequent command always works.

## Further reading

- [Incremental Models](incremental-models.md) for combining selection with time ranges
- [SQL Models](sql-models.md) for setting tags in YAML frontmatter
