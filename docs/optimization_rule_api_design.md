# Optimization Rule API Design

## Overview

smelt supports optimizer rules written in both Rust and Python. Rust rules are compiled into the optimizer; Python rules are discovered at runtime via entry points and executed through a PyO3 bridge.

Python support is gated behind the `python` cargo feature flag. Without it, only Rust rules run and there is no Python dependency.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  Optimizer (smelt-optimizer)                        │
│                                                     │
│  ┌─────────────┐  ┌──────────────┐                  │
│  │ Rust rules  │  │ Python bridge│ (feature=python) │
│  │ cube_split  │  │ (PyO3)       │                  │
│  │ incremental │  └──────┬───────┘                  │
│  └─────────────┘         │                          │
└──────────────────────────┼──────────────────────────┘
                           │
              ┌────────────▼────────────────┐
              │  Python entry points        │
              │  smelt.optimizer_rules      │
              │                             │
              │  ┌────────────────────────┐ │
              │  │ smelt-rules-builtin    │ │
              │  │ (or any pip package)   │ │
              │  └────────────────────────┘ │
              │            │                │
              │  ┌─────────▼──────────────┐ │
              │  │ smelt-sdk (pure Python) │ │
              │  │ Base classes & types    │ │
              │  └────────────────────────┘ │
              └─────────────────────────────┘
```

## Writing a Python Rule

### 1. Create a package

```
my-smelt-rules/
├── pyproject.toml
└── src/
    └── my_rules/
        ├── __init__.py
        └── my_rule.py
```

### 2. Implement the rule

```python
from smelt_sdk import (
    OptimizerRule,
    ModelInfo,
    SelectAnalysis,
    Opportunity,
    ReplaceWithPlan,
    SetIncremental,
    CountDistinct,
    GroupByKey,
    CreateTemp,
    FinalQuery,
    DropTemp,
)


class MyRule(OptimizerRule):
    def name(self) -> str:
        return "my_rule"

    def detect(
        self,
        model: ModelInfo,
        analysis: SelectAnalysis | None,
    ) -> Opportunity | None:
        """Return an Opportunity if this rule applies, else None."""
        if analysis is None:
            return None
        # Check for patterns in analysis.items, model.sql, etc.
        return Opportunity(
            rule_name=self.name(),
            model=model.name,
            description="Description of the optimization",
            data={"key": "value"},
        )

    def rewrite(
        self,
        model: ModelInfo,
        analysis: SelectAnalysis | None,
    ) -> ReplaceWithPlan | SetIncremental | None:
        """Return a transformation, or None if rewrite is not possible."""
        # Build execution steps
        steps = [
            CreateTemp(name="__temp", sql="SELECT ..."),
            FinalQuery(sql="SELECT * FROM __temp"),
            DropTemp(name="__temp"),
        ]
        return ReplaceWithPlan(model=model.name, steps=steps)
```

### 3. Register via entry points

In `pyproject.toml`:

```toml
[project]
name = "my-smelt-rules"
dependencies = ["smelt-sdk"]

[project.entry-points."smelt.optimizer_rules"]
my_rule = "my_rules.my_rule:MyRule"
```

### 4. Install and use

```bash
pip install smelt-sdk
pip install -e .

# Run smelt with Python rules enabled
cargo run --features python -p smelt-cli -- run
```

## Available Types (smelt-sdk)

### Model types

| Type | Fields |
|------|--------|
| `ModelInfo` | `name: str`, `sql: str`, `refs: list[str]`, `incremental_config: IncrementalConfig \| None` |
| `IncrementalConfig` | `partition_column: str` |

### SELECT analysis types

| Type | Fields |
|------|--------|
| `SelectAnalysis` | `items: list[SelectItemKind]`, `from_text: str`, `where_text: str \| None`, `group_by_exprs: list[str]`, `has_cube_split_annotation: bool` |
| `CountDistinct` | `argument: str`, `alias: str` |
| `OtherAggregate` | `text: str`, `alias: str` |
| `GroupByKey` | `text: str`, `alias: str` |

### Execution step types

| Type | Fields |
|------|--------|
| `CreateTemp` | `name: str`, `sql: str` |
| `AppendToTemp` | `name: str`, `sql: str` |
| `FinalQuery` | `sql: str` |
| `DropTemp` | `name: str` |

### Transformation types (return from `rewrite`)

| Type | Fields |
|------|--------|
| `ReplaceWithPlan` | `model: str`, `steps: list[ExecutionStep]` |
| `SetIncremental` | `model: str`, `event_time_column: str`, `partition_column: str` |

### Opportunity (return from `detect`)

| Type | Fields |
|------|--------|
| `Opportunity` | `rule_name: str`, `model: str`, `description: str`, `data: dict` |

## How It Works

1. The Rust optimizer runs all built-in Rust rules first
2. When the `python` feature is enabled, it then calls `python_bridge::run_python_rules()`
3. The bridge discovers all Python rules registered under the `smelt.optimizer_rules` entry point group
4. For each model, the Rust parser pre-computes `SelectAnalysis` and converts it to a Python dataclass
5. Each Python rule's `rewrite()` method is called with the model and analysis
6. Returned `ReplaceWithPlan` or `SetIncremental` objects are converted back to Rust `Transformation` values
7. Python rule transformations are appended to the Rust rule results

## Built-in Python Rules

The `smelt-rules-builtin` package provides Python ports of both Rust rules:

- **`cube_split_python`** — Detects `-- smelt:cube_split` annotation with 2+ COUNT(DISTINCT) expressions
- **`incremental_python`** — Detects incremental config and validates partition column

These serve as proof-of-concept implementations and can be used for testing.

## Building

```bash
# Without Python (default)
cargo build

# With Python support
cargo build --features python

# CLI with Python
cargo build -p smelt-cli --features python

# Run tests
cargo test                                  # Rust tests (no Python)
cargo test --features python                # Rust + Python bridge tests
pytest python/smelt-rules-builtin/tests/    # Python rule unit tests
```

## Design Decisions

1. **Pure Python SDK**: `smelt-sdk` has no Rust dependency — it's testable with plain pytest
2. **Entry point discovery**: Standard Python packaging convention via `importlib.metadata`
3. **Feature-gated**: Zero-cost when Python is not needed (`cargo build` works without Python)
4. **Analysis pre-computed in Rust**: Python rules receive parsed `SelectAnalysis` from the Rust parser, avoiding the need for Python SQL parsing libraries
5. **Environment inheritance**: PyO3 respects the active virtualenv/conda environment automatically
