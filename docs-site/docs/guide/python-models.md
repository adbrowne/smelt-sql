# Python Models

Python models let you generate SQL dynamically. Use them when you need conditional logic, loops, or access to project metadata that plain SQL cannot provide.

## When to use Python models

- Dynamically union multiple source tables
- Generate SQL conditionally based on project configuration
- Loop over a list of models or columns
- Access project metadata to build queries programmatically

For straightforward transforms, prefer [SQL models](sql-models.md). Python models add complexity and should be reserved for cases where static SQL is not sufficient.

## Writing a Python model

A Python model is a `.py` file in your `models/` directory that uses the `@model` decorator:

```python
from smelt import model


@model
def combined_events(project):
    """Combine all event source models into a single UNION ALL."""
    children = project.find_models(tag="event_source")
    if not children:
        return "SELECT event_id, user_id, event_time, event_type FROM smelt.models.raw_events"
    refs = [f"SELECT * FROM smelt.models.{m.name}" for m in children]
    return " UNION ALL ".join(refs)
```

The function name becomes the model name. The function must return a string containing valid SQL (with `smelt.models.<name>` and `smelt.sources.<name>` calls as needed).

### The project context

The `project` argument provides access to project metadata:

- **`project.find_models(tag=...)`** -- Find all models with a given tag. Returns a list of model objects with a `name` attribute.
- **`project.find_models(directory=...)`** -- Find all models in a given directory.

### Multiple models in one file

A single Python file can define multiple models. Each function decorated with `@model` becomes a separate model:

```python
from smelt import model


@model
def active_users(project):
    """Users with recent activity."""
    return """
    SELECT user_id, COUNT(*) as activity_count
    FROM smelt.models.user_sessions
    WHERE session_count > 0
    GROUP BY user_id
    """


@model
def inactive_users(project):
    """Users with no recent activity."""
    return "SELECT user_id FROM smelt.models.user_stats WHERE total_sessions = 0"
```

### Helper functions

Python files without `@model` decorators are not treated as models. You can use them as utility modules:

```python
# models/helpers.py -- not a model, just a helper module
def format_union(model_names):
    """Helper to build UNION ALL queries from a list of model names."""
    refs = [f"SELECT * FROM smelt.models.{name}" for name in model_names]
    return " UNION ALL ".join(refs)
```

### Returning frontmatter

You can include YAML frontmatter in the returned SQL string to set materialization and other options:

```python
from smelt import model


@model
def daily_summary(project):
    return """
---
materialization: table
tags: [daily, summary]
---
SELECT
    DATE(event_time) as event_date,
    COUNT(*) as event_count
FROM smelt.models.events
GROUP BY 1
"""
```

## Configuration

### Python interpreter

smelt needs to know which Python interpreter to use. Set it in `smelt.yml` or via an environment variable:

**smelt.yml:**

```yaml
python: /usr/bin/python3
```

**Environment variable:**

```bash
export SMELT_PYTHON=/usr/bin/python3
smelt run
```

The `SMELT_PYTHON` environment variable takes precedence over the `python` field in `smelt.yml`.

!!! note
    If neither is set, smelt will attempt to find `python3` on your PATH.

## Further reading

- [SQL Models](sql-models.md) for standard SQL model syntax
- [Model Selection](model-selection.md) for running specific models
