# Model Selection

When working with a project that has many models, you often want to run only a subset. smelt provides `--select` and `--exclude` flags to control which models are included in a run.

These flags are available on `smelt run`, `smelt build`, and `smelt explain`.

## Selection syntax

### By name

Select a single model by its name:

```bash
smelt run --select daily_revenue
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
smelt run --select +daily_revenue
smelt run --select +tag:revenue
```

This ensures that every model `daily_revenue` depends on is also run, in the correct order.

### Include downstream dependents

Suffix with `+` to include the selected model(s) and everything that depends on them:

```bash
smelt run --select daily_revenue+
smelt run --select tag:staging+
```

### Include both upstream and downstream

Combine the prefix and suffix:

```bash
smelt run --select +daily_revenue+
```

This selects `daily_revenue`, all its upstream dependencies, and all its downstream dependents.

### Multiple selectors

The `--select` flag is repeatable. All selectors are combined (union):

```bash
smelt run --select daily_revenue --select user_activity
```

Short form with `-s`:

```bash
smelt run -s daily_revenue -s user_activity
```

## Excluding models

The `--exclude` flag uses the same syntax as `--select` but removes models from the selection:

```bash
smelt run --exclude tag:expensive
smelt run --exclude staging_legacy
```

Exclusions are applied after selections. If you use `--select` and `--exclude` together, smelt first builds the selected set, then removes the excluded models.

## Practical examples

### Run a single model

```bash
smelt run --select daily_revenue
```

### Run a model and all its dependencies

```bash
smelt run --select +daily_revenue
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
smelt run --select +daily_revenue --exclude user_activity
```

### Explain selected models

```bash
smelt explain --select +daily_revenue
smelt explain --select tag:revenue --json
```

## How selection works

1. If no `--select` flags are provided, all models are included.
2. Each `--select` flag adds models to the set. The `+` prefix/suffix expands the set by walking the dependency graph.
3. Each `--exclude` flag removes models from the set, using the same expansion rules.
4. The final set is executed in topological order (dependencies first).

!!! tip
    During development, use `--select +model_name` to run just the model you are working on along with its dependencies. This is much faster than running the entire project.

## Further reading

- [Incremental Models](incremental-models.md) for combining selection with time ranges
- [SQL Models](sql-models.md) for setting tags in YAML frontmatter
