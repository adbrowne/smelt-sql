## Drift Report: python_models

**Spec**: docs/specs/python_models.md (last_reviewed: 2026-05-05)
**Date**: 2026-05-31
**Phase**: C9 (feature sweep)

### Automated checks
- cargo fmt — PASS
- cargo clippy --all-targets — PASS (zero warnings)
- cargo test — PASS (no failures; 15/15 `smelt-cli` `python::tests` green, subprocess path)
- example_diagnostics — PASS (87 passed)
- example_workspaces — PASS (27 passed)

### Surface drift
- ✅ `.py` files in `paths:` discovered via `@model` decorator scanning (`python_utils::has_model_decorator`, `scan_for_model_decorators`).
- ✅ Each decorated function = one model; multiple per file supported (`test_multiple_models_one_file`).
- ✅ Non-decorated functions ignored (`test_non_model_python_files_skipped`).
- ✅ `project.find_models(tag=, directory=)` — both filters + intersection in `python/smelt/core.py`; `ModelInfo.{name,tags,directory}` populated by `build_project_context`.
- ✅ Interpreter resolution (`SMELT_PYTHON`, `python:`, `python3`, `python`) — `python_utils::find_python`. SDK resolution (`SMELT_PYTHON_SDK`, `<project_dir>/python/`, walk-up) — `python_utils::find_python_sdk`.
- ❌ **`@model()` called form crashes** (BUG-039 — FIXED). Spec §Surface "@model decorator": "Both `@model` and `@model()` (called form) are recognized." The Rust/LSP scanner accepts `@model()` (`has_model_decorator` matches `@model(`), but the SDK `model(func)` in `python/smelt/core.py:26` raised `TypeError: model() missing 1 required positional argument: 'func'` at execution. Fixed: `model` now accepts the called form (decorator-factory pattern).
- ❌ **`--- name: X ---` frontmatter name override is not honored when X ≠ function name** (BUG-038 — needs-review). Spec §"Model name derivation": "If a returned SQL string includes a `--- name: X ---` frontmatter header, that name is used instead of the function name." Code (`crates/smelt-cli/src/python.rs:269`) always uses `output.name` (the function name) and the Multi-section match at `python.rs:244` keys on `output.name`, so a non-matching name silently drops the *entire* frontmatter (materialization etc.) and keeps the function name. Builds exit 0 as a view.

### Semantics drift
- ✅ Compile-time evaluation (subprocess/PyO3) — `discover_python_models`.
- ✅ Iterative evaluation up to 5 rounds + convergence (`test_find_models_convergence`); non-convergence error (`python.rs:297`).
- ✅ Circular meta-dependency detection (`validate_fixed_point`, `test_circular_meta_dependency`, `test_validate_fixed_point_*`).
- ✅ Returned SQL frontmatter parsing for the matching-name / no-name forms (`matchname`/`singlefm` probes: `materialization: table` applied correctly).
- ⚠️ Model-name derivation override path — see BUG-038.

### Invariant drift
- ✅ Inv 1 (compile-time only) — Python never invoked at query time.
- ✅ Inv 2 (exactly one arg) — wrong-arity functions raise a Python `TypeError` at evaluation time (probed; matches "error at evaluation time"). Error surfaces as a raw traceback rather than a smelt diagnostic (acceptable per spec wording).
- ✅ Inv 3 (return must be string) — `test_non_string_return`; runner checks `isinstance(sql, str)`.
- ❌ **Inv 4 (model names unique) not enforced** (BUG-040 — needs-review). A Python model whose name collides with a SQL model (or another Python model) is silently de-duplicated by the `LogicalGraph` HashMap (kept one), exit 0, no diagnostic — spec says "configuration error". Documented as "current behavior" by `test_python_model_name_collision`. Same class as BUG-021 (duplicate model names unrejected).
- ✅ Inv 5 (converge within 5 rounds) — enforced (`python.rs:297`).
- ✅ Inv 6 (SDK discoverable) — `find_python_sdk` returns a clear error when no `python/smelt/` is found.

### Timeless-oracle drift
- ✅ No phase-vocabulary leakage in `docs/specs/python_models.md` body.
- ⚠️ `crates/smelt-cli/src/python.rs` carries several `// TODO Phase 5:` comments (code comments, not spec/user-doc body — out of scope for the timeless-oracle rule).

### Freshness
- last_reviewed: 2026-05-05
- most recent code change: 2026-05-28 (canonical `smelt.<path>` addressing, #125)
- Verdict: fresh enough; spec still describes behavior accurately apart from the logged divergences.

### Summary
- Drift items: 3 — 1 surface (BUG-039, fixed), 1 surface/derivation (BUG-038, needs-review), 1 invariant (BUG-040, needs-review).
- Recommended next step: resolve BUG-038/BUG-040 in the post-sweep human pass (both are spec-vs-code conflicts that ripple into LSP identity / cross-feature duplicate-name policy).
