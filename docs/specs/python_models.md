---
feature: python_models
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Python Models

> **What this is.** A normative spec for Python model files — the `@model` decorator, the `project` context API, model name derivation, compile-time evaluation, and Python interpreter resolution.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### File format

Python model files are `.py` files in any non-excluded directory under the project root (discovery is project-wide; `paths:` only strips address prefixes — see `smelt_yml.md` and `architecture.md` §"Resolution"). A file must import and use the `@model` decorator from the smelt Python SDK:

```python
from smelt import model

@model
def combined_events(project):
    sources = project.find_models(tag="source")
    unions = "\nUNION ALL\n".join(
        f"SELECT * FROM smelt.{m.name}" for m in sources
    )
    return f"""
--- name: combined_events ---
materialization: table
---
{unions}
"""
```

- Each decorated function produces one model.
- The function name is the model name.
- The function must accept one argument (`project`) and return a SQL string.
- The returned SQL string may include YAML frontmatter using the `--- name: ... ---` / `---` delimiter format. If a `name:` field appears, it must equal the function name; a mismatch is a `PythonModelNameMismatch` error.
- Multiple `@model`-decorated functions in one file each produce an independent model.
- Non-decorated functions are helper utilities and are ignored.

### `@model` decorator

```python
from smelt import model

@model
def my_model(project):
    ...
```

Both `@model` and `@model()` (called form) are recognized.

### `project` context API

The `project` argument is a `ProjectContext` instance. It provides read-only access to the set of all models discovered in the project at the time Python evaluation begins.

#### `project.find_models(tag=None, directory=None)`

Returns a list of `ModelInfo` objects for models matching the given filters.

| Parameter | Type | Description |
|-----------|------|-------------|
| `tag` | `str \| None` | If set, return only models whose tag set includes this tag |
| `directory` | `str \| None` | If set, return only models whose source directory equals this string |

Both parameters can be combined (intersection). Omitting both returns all models.

Each `ModelInfo` has:

| Attribute | Type | Description |
|-----------|------|-------------|
| `name` | `str` | Model name |
| `tags` | `list[str]` | Tags from effective tag set (frontmatter + smelt.yml config) |
| `directory` | `str` | Directory name of the model's source file |

The `project` context includes all SQL models discovered before Python evaluation begins, plus all Python-derived models from previous evaluation rounds. See Semantics — Iterative evaluation.

### Python interpreter configuration

| Method | Description |
|--------|-------------|
| `SMELT_PYTHON` env var | Path to the Python executable; highest priority |
| `python:` in `smelt.yml` | Project-level Python executable path |
| `python3` on PATH | Fallback |
| `python` on PATH | Final fallback |

The Python SDK path is resolved via:

| Method | Description |
|--------|-------------|
| `SMELT_PYTHON_SDK` env var | Path to the SDK directory |
| `<project_dir>/python/` | Project-local SDK |
| Walk up 5 parent directories | Monorepo support — finds first `python/smelt/` ancestor |

## Semantics

### Compile-time evaluation

Python model functions are evaluated at **compile time** — during `smelt build`, `smelt run`, or LSP analysis — not at database query execution time. The Python code generates SQL strings; those strings are then parsed and processed identically to SQL model files.

### Model name derivation

The model name is always the Python function name, exactly. A function named `daily_revenue` produces a model named `daily_revenue`, addressable as `smelt.daily_revenue` (universal `smelt.<path>` form per `architecture.md` §"Resolution") in other models.

If a returned SQL string includes a `--- name: X ---` frontmatter header, the `name:` field must either be absent or exactly equal the function name. A `name:` value that differs from the function name is a `PythonModelNameMismatch` diagnostic (Error severity); the frontmatter is dropped and model defaults apply. This rule exists because the function name is the stable identity — allowing the frontmatter to override it would silently produce a model under a different name than what the author wrote at the call site.

### Iterative evaluation

Python model discovery runs in rounds (up to 5):

1. Build the project context from all currently known models (SQL + previously evaluated Python models).
2. Execute each Python file; collect the returned SQL strings and their `name` values.
3. Parse each returned SQL string as a smelt model.
4. If the set of models and their SQL content is identical to the previous round, evaluation converges and stops.
5. Otherwise, repeat.

This allows Python models that generate models by querying the project context to stabilize. If evaluation has not converged after 5 rounds, smelt reports an error.

### Circular dependency detection

Each `find_models()` call is recorded in the evaluation result. If a Python model's output would include itself in the project context passed to it in a subsequent round (i.e., a model queries for models with a tag it itself carries), smelt detects a circular meta-dependency and reports an error.

### Execution modes

When compiled with the `python` feature (default for binary releases), Python is executed in-process via PyO3 — an embedded Python interpreter. The project context is passed directly as a Rust data structure with no subprocess overhead.

When the `python` feature is absent, smelt invokes `python -m smelt.runner <file>` as a subprocess. The project context is passed via stdin as JSON; results are read from stdout as JSON.

### Returned SQL string

The return value of a `@model` function must be a string containing valid SQL. The SQL may optionally start with a YAML frontmatter block:

```
--- name: <model_name> ---
<frontmatter keys>
---
<SQL body>
```

All standard model frontmatter keys (`materialization`, `tags`, `owner`, etc.) are valid in the returned string. If frontmatter is absent, defaults apply as they would for any SQL model file.

When a `name:` field appears in the frontmatter, it must equal the function name. A mismatch emits `PythonModelNameMismatch` (Error) and the frontmatter is dropped.

## Design

**Compile-time Python, not Jinja.** Python model generation runs once at compile time and produces plain SQL. This avoids the template-execution-per-run overhead of Jinja, enables proper error messages (Python exceptions with tracebacks rather than template expansion failures), and keeps the query engine free of Python dependencies. The trade-off is that Python cannot access live query results — it can only operate on the project's structural metadata. Jinja was rejected because Jinja templates mix control flow with SQL syntax (no IDE, no type-checker, no LSP understands them), have opaque failure modes at render time, and force the executor to interpret template logic — coupling execution to a Python runtime. Runtime-access Python (calling a function each time the query runs) was rejected for the same reason: it couples execution to a Python runtime and prevents static analysis.

**`find_models` as the only context API.** The project context deliberately exposes only structural metadata (name, tags, directory), not schema, type, or lineage information. This keeps the API stable: schema and type information changes with implementation; structural metadata changes only when models are added or removed. A richer API would require Python evaluation to depend on type inference, creating a circular dependency in the compiler pipeline. Exposing schema/type was rejected specifically to avoid that circularity: if Python models can inspect column types, and column types come from type-checking Python-generated SQL, the compiler pipeline has no topological ordering.

**Iterative evaluation for self-referential generation.** A Python model may generate models that in turn are used by other Python models (e.g., a "tag all staging models" generator that a marts generator queries). The iterative fixed-point approach handles these cases without requiring explicit ordering.

**Function name = model name.** Using the function name as the model name follows the same "identity falls out of structure" principle as file-stem naming for SQL models. The `--- name: X ---` field is tolerated in the returned SQL only when it repeats the function name exactly; mismatching it is an error so authors get a diagnostic rather than a silently-misrouted model.

## Constraints & Invariants

1. **Python evaluation is compile-time only.** Python code is not executed during database query time.
2. **`@model` functions must accept exactly one argument** — the project context. Functions with other signatures are an error at evaluation time.
3. **Return value must be a string.** Returning non-string (e.g., `None`, a list) is a Python-level runtime error during compilation.
4. **Canonical addresses are unique.** Two Python `@model` functions whose names resolve to the same canonical `smelt.<path>` address — or whose name matches a SQL model's canonical address — produce a `DuplicateAddress` error. Uniqueness is keyed on the full canonical address (the function name), not on the bare leaf name in isolation.
5. **Iterative evaluation converges within 5 rounds.** If models have not stabilized after 5 rounds, smelt reports an error and halts.
6. **Python SDK must be discoverable.** If no SDK path can be resolved, Python model evaluation fails with a clear error.

## Known Divergences / Open Questions

- **`@model()` (called form) recognition.** The called form `@model()` is recognized by file scanning but the behavior when arguments are passed to `@model(...)` is undefined.
- **LSP support for Python models is partial.** Go-to-definition from SQL references to Python-derived model names works via decorator-line scanning. Diagnostics inside Python files (type errors, undefined refs in the generated SQL) are not surfaced by the LSP until the SQL is generated and parsed.
- **Iterative convergence error handling.** When convergence fails after 5 rounds, the error message does not identify which Python model is causing the oscillation.
- **`directory` filter semantics.** The `directory` parameter of `find_models()` matches the model file's parent directory name (just the final path component, not the full path). This behavior is not documented in the user guide.
- **Python model hash for schema tracking.** The `model_hash` stored in `.smelt/schemas/` for a Python-derived model is the SHA-256 of the generated SQL string, not the Python source file. Changes to the Python file that produce identical SQL do not trigger schema migration checks.
- **PyO3 vs subprocess behavior parity.** The two execution modes (in-process PyO3 and subprocess) should produce identical results, but this is not tested. Edge cases in SDK path resolution may differ.
- **Python `@model` functions cannot emit multiple models via `generates: models`.** The `generates: models` frontmatter directive and the `ModelDef` record type (per `meta_language.md` §"Multi-model production") are SQL meta-language features. A Python `@model` function returns a single SQL string for a single model; the `ModelDef` value type is not exposed in the Python SDK context, and a Python file cannot declare `generates: models`. Workspaces that need multi-model emission must author the generator as a `.sql` (or `.gen.sql`) file. The boundary is intentional: the SQL meta-language has compile-time type checking and full LSP support, which the Python execution path (in-process PyO3 or subprocess) does not currently expose at the `ModelDef`-shape granularity. Tracked in `docs/plans/20260509-meta-language-overall.md`.

## References

- **Code**:
  - `python/smelt/core.py` — `@model` decorator, `ProjectContext`, `ModelInfo`
  - `python/smelt/runner.py` — subprocess entry point (`python -m smelt.runner`)
  - `crates/smelt-cli/src/python.rs` — discovery orchestration, iterative evaluation, subprocess mode
  - `crates/smelt-core/src/python_models.rs` — in-process PyO3 execution
  - `crates/smelt-core/src/python_utils.rs` — interpreter resolution, `build_decorator_map()`
  - `crates/smelt-lsp/src/python_scan.rs` — LSP decorator scanning for go-to-definition
- **User docs**:
  - `docs-site/docs/guide/python-models.md`
- **Related specs**:
  - `models.md` — SQL model files, frontmatter schema, `smelt.<path>` addressing
  - `smelt_yml.md` — `python:` config key
