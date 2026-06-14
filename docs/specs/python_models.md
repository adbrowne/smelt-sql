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
---
materialization: table
---
{unions}
"""
```

- Each decorated function produces one model.
- The function name is the model name.
- The function must accept one argument (`project`) and return a SQL string.
- The returned SQL string may include YAML frontmatter using the plain `---` / `---` single-model delimiter format (no `name:` in the opening delimiter). The model's identity comes from the function name, not the frontmatter, so the multi-model `--- name: <model> ---` section delimiter (per `models.md`) never appears in Python output. If a `name:` *key* appears inside the frontmatter body, it must equal the function name; a mismatch is a `PythonModelNameMismatch` error.
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
| `directory` | `str \| None` | If set, return only models whose `directory` attribute (the final component of the model's full `path`) equals this string |

Both parameters can be combined (intersection). Omitting both returns all models.

Each `ModelInfo` has:

| Attribute | Type | Description |
|-----------|------|-------------|
| `name` | `str` | Model name (leaf segment of the canonical address) |
| `tags` | `list[str]` | Tags from effective tag set (frontmatter + smelt.yml config) |
| `path` | `str` | Full workspace-relative path of the model's source file, `/`-normalised. This is the same `path` vocabulary the meta-language `ModelRef` uses (`meta_language.md` §"`ModelRef` meta record type"), so the two reflection surfaces agree. For a generator-emitted model (whether the emitter is a SQL `generates: models` generator or another producer), `path` is the **generator file's** path. |
| `directory` | `str` | Final path component of `path` (the model's parent directory name). Defined as a function of `path` so the two attributes never disagree; for a generator-emitted model it is the generator file's directory. |

The `project` context includes all SQL models discovered before Python evaluation begins (including SQL-generator emissions surfaced through the combined evaluation loop — see Semantics — Iterative evaluation), plus all Python-derived models from previous evaluation rounds. `find_models` surfaces generator-emitted models on equal terms with hand-authored ones.

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

The model's **leaf name** is always the Python function name, exactly. A function named `daily_revenue` produces a model whose leaf name is `daily_revenue`. Its full canonical address is path-derived exactly as for SQL models — the function name prefixed by the address of the file's directory (the file's workspace-relative path minus any `paths:` prefix, per `architecture.md` §"Resolution"). A `daily_revenue` function in a file at the address root is `smelt.daily_revenue`; the same function in `py/marts.py` (directory address `marts`) is `smelt.marts.daily_revenue`.

If a returned SQL string's frontmatter includes a `name:` key, it must either be absent or exactly equal the function name. A `name:` value that differs from the function name is a `PythonModelNameMismatch` diagnostic (Error severity). Because the diagnostic is an Error, it **blocks the build** under the fail-loud / diagnostic-parity rule (`architecture.md` §"Fail-loud discipline") — there is no recovery in which the build proceeds, so the frontmatter is *not* silently dropped and no "defaults apply" fallback runs. At analysis time (LSP), the model is retained with its other frontmatter keys intact and only the offending `name:` key is flagged, so the editor surfaces the mismatch without discarding legitimate `materialization` / `tags` / `owner` values. This rule exists because the function name is the stable identity — allowing the frontmatter to override it would silently produce a model under a different name than what the author wrote at the call site.

### Iterative evaluation

Python models and SQL `generates: models` generators (per `meta_language.md` §"Multi-model production") run in **one combined, fully-interleaved fixed-point loop**, not two separate passes. There is no one-directional layering: a Python `find_models` call observes SQL-generator emissions, and a SQL generator's literal `smelt.<path>` references observe Python-derived models, both within the same loop. Each round:

1. Build the project context from **all currently known models** — hand-authored SQL models, SQL-generator emissions, and Python-derived models — produced by every prior round.
2. Re-run every SQL generator and every Python `@model` file against that context; collect each emission's canonical address, frontmatter, and SQL content.
3. Parse each returned/emitted SQL string as a smelt model.
4. Compare the resulting **model set** (each model keyed by its canonical address, with its frontmatter and SQL content) to the previous round's set.

**Determinism and termination (combined loop).** The loop is bounded at 5 rounds. It **terminates when the model set stabilises across a round** — the set of canonical addresses and every model's frontmatter and SQL content are byte-identical to the previous round. Within a round, generators (SQL and Python) are evaluated in a fixed order: ascending by `path`, then by `name` as a tiebreaker (the same total order as wide reflection, `meta_language.md` §"Reflection: `smelt.models`, `smelt.sources`, `ModelRef`, `SourceRef`" rule 2), so the combined evaluation is deterministic and re-evaluation over an unchanged workspace produces a byte-equal result. The `path`-then-`name` tiebreak is load-bearing precisely because the combined loop is harder to make deterministic than a single one-directional pass: it fixes the order in which co-emitting generators contribute. If the model set has not stabilised after 5 rounds, smelt reports a non-convergence error.

### Circular dependency detection

Circularity is **non-convergence**: a set of generators (Python or SQL) whose combined output never stabilises — it oscillates or grows unbounded — across the 5 bounded rounds. When the model set differs from the previous round at round 5, smelt reports a non-convergence (circular meta-dependency) error. A generator that tags its emissions `staging` and a second generator that queries `tag=staging` is *not* circular by construction: a monotonically-growing-then-stable model set converges within the bound and is the supported self-referential-generation pattern (see §Design — "Iterative evaluation for self-referential generation"). Only output that fails to stabilise is an error.

### Execution modes

When compiled with the `python` feature (default for binary releases), Python is executed in-process via PyO3 — an embedded Python interpreter. The project context is passed directly as a Rust data structure with no subprocess overhead.

When the `python` feature is absent, smelt invokes `python -m smelt.runner <file>` as a subprocess. The project context is passed via stdin as JSON; results are read from stdout as JSON.

### Returned SQL string

The return value of a `@model` function must be a string containing valid SQL. The SQL may optionally start with a YAML frontmatter block:

```
---
<frontmatter keys>
---
<SQL body>
```

All standard model frontmatter keys (`materialization`, `tags`, `owner`, etc.) are valid in the returned string. If frontmatter is absent, defaults apply as they would for any SQL model file.

When a `name:` key appears in the frontmatter, it must equal the function name. A mismatch emits `PythonModelNameMismatch` (Error), which blocks the build; at analysis time the model is retained with its other frontmatter keys and only the bad `name:` key is flagged (see §Semantics — "Model name derivation").

## Design

**Compile-time Python, not Jinja.** Python model generation runs once at compile time and produces plain SQL. This avoids the template-execution-per-run overhead of Jinja, enables proper error messages (Python exceptions with tracebacks rather than template expansion failures), and keeps the query engine free of Python dependencies. The trade-off is that Python cannot access live query results — it can only operate on the project's structural metadata. Jinja was rejected because Jinja templates mix control flow with SQL syntax (no IDE, no type-checker, no LSP understands them), have opaque failure modes at render time, and force the executor to interpret template logic — coupling execution to a Python runtime. Runtime-access Python (calling a function each time the query runs) was rejected for the same reason: it couples execution to a Python runtime and prevents static analysis.

**`find_models` as the only context API.** The project context deliberately exposes only structural metadata (name, tags, directory), not schema, type, or lineage information. This keeps the API stable: schema and type information changes with implementation; structural metadata changes only when models are added or removed. A richer API would require Python evaluation to depend on type inference, creating a circular dependency in the compiler pipeline. Exposing schema/type was rejected specifically to avoid that circularity: if Python models can inspect column types, and column types come from type-checking Python-generated SQL, the compiler pipeline has no topological ordering.

**Iterative evaluation for self-referential generation.** A Python model may generate models that in turn are used by other generators — Python *or* SQL — and vice versa (e.g., a "tag all staging models" generator that a marts generator queries). The single combined fixed-point loop handles these cases without requiring explicit ordering between the Python and SQL generator families: both observe each other's emissions round-over-round until the model set stabilises. The convergent self-referential pattern (a model set that grows then stabilises) is the supported mechanism; circularity is reserved for output that never stabilises (see §Semantics — "Circular dependency detection"). A fully-interleaved loop was chosen over a one-directional Python-after-SQL layering so that neither family is privileged — a SQL generator can consume Python-derived models and a Python generator can consume SQL-generator emissions — at the cost of a stricter determinism obligation (fixed `path`-then-`name` within-round evaluation order) to keep the bounded loop reproducible.

**Function name = model name.** Using the function name as the model name follows the same "identity falls out of structure" principle as file-stem naming for SQL models. Python output uses plain `---` / `---` single-model frontmatter — never the `--- name: <model> ---` multi-model section delimiter (per `models.md`), which would clash with the Layer-1 SQL parser's reading of that token. A `name:` *key* inside the frontmatter body is tolerated only when it repeats the function name exactly; mismatching it is an error so authors get a diagnostic rather than a silently-misrouted model.

## Constraints & Invariants

1. **Python evaluation is compile-time only.** Python code is not executed during database query time.
2. **`@model` functions must accept exactly one argument** — the project context. Functions with other signatures are an error at evaluation time.
3. **Return value must be a string.** Returning non-string (e.g., `None`, a list) is a Python-level runtime error during compilation.
4. **Canonical addresses are unique.** A Python model's canonical address is **path-derived, identical to SQL models** — the function name is the leaf segment, prefixed by the address of the file's directory (its workspace-relative path minus any `paths:` prefix, per `architecture.md` §"Resolution"). A `@model def users` in `py/archive.py` resolves to `smelt.archive.users`, not bare `smelt.users`. Two `@model` functions resolving to the same canonical `smelt.<path>` address — or one matching a SQL model's canonical address — produce a `DuplicateAddress` error. Uniqueness is keyed on the full canonical address (directory prefix + function name), not on the bare leaf name in isolation.
5. **The combined evaluation loop converges within 5 rounds.** Python models and SQL `generates: models` generators share one interleaved fixed-point. If the model set has not stabilised (byte-identical addresses, frontmatter, and SQL content to the prior round) after 5 rounds, smelt reports a non-convergence error and halts. Within-round generator order is `path` then `name`, making the loop deterministic.
6. **Python SDK must be discoverable.** If no SDK path can be resolved, Python model evaluation fails with a clear error.

## Known Divergences / Open Questions

- **`@model()` (called form) recognition.** The called form `@model()` is recognized by file scanning but the behavior when arguments are passed to `@model(...)` is undefined.
- **LSP support for Python models is partial.** Go-to-definition from SQL references to Python-derived model names works via decorator-line scanning. Diagnostics inside Python files (type errors, undefined refs in the generated SQL) are not surfaced by the LSP until the SQL is generated and parsed.
- **Iterative convergence error handling.** When convergence fails after 5 rounds, the error message does not identify which Python model is causing the oscillation.
- **`directory` filter semantics (user guide gap).** The `directory` parameter of `find_models()` matches the `ModelInfo.directory` attribute — the final component of the model's full `path`, not the full path (see §Surface). The `path` attribute and the path-derived `directory` definition are implemented but not yet documented in the user guide (`docs-site/docs/guide/python-models.md`).
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
