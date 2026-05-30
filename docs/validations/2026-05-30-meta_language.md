## Drift Report: meta_language

**Spec**: docs/specs/meta_language.md (last_reviewed: 2026-05-16)
**Date**: 2026-05-30
**Phase**: A5 (feature sweep)

### Automated checks
- cargo fmt — PASS (pre-flight clean)
- cargo clippy — PASS (no warnings introduced; pre-flight clean)
- cargo test — PASS (pre-flight `cargo test --quiet` green; no failures)
- example_diagnostics — PASS (87 passed, 1 ignored; all `meta_*` workspaces LSP-clean)

### Surface drift
- ✅ All 65 §Surface diagnostic codes exist in `crates/smelt-db/src/` (`MetaListEmptyTypeUnknown` … `GeneratorBodyForbidsModelReflection` — exhaustive grep, zero missing).
- ✅ Lists/spread, lambdas/HOFs, pipe, reducers (incl. parameterised `concat_with`), ternary, `smelt.config.var`, `smelt.columns_of`/`ColumnRef`, wide reflection (`smelt.models.*`/`smelt.sources.*`), records, `Map<K,V>`, and multi-model production (`generates: models`/`ModelDef`) all have shipped example fixtures and broken-case fixtures under `examples/meta_*`.
- ✅ `examples/per_cohort_union` builds end-to-end via `smelt build` (5 models; generators + `union_all` reducer + `load_yaml` loader materialise correctly on DuckDB).
- ❌ **In-model meta-language constructs do not survive the CLI run/build pipeline** — see Semantics drift below and BUG-006. Multiple LSP-clean `examples/meta_*` workspaces (`meta_lists`, `meta_hofs`, `meta_polish`, `meta_workspace`, and any reflection user) fail `smelt run`/`build`.

### Semantics drift
- ✅ §"Multi-model production" W1–W4 pipeline: `examples/per_cohort_union` exercises generator emission + collision + downstream type-check end-to-end; `cargo test -p smelt-cli --test cohort_count_acceptance` is the gate.
- ✅ Reducer/loader/record/map inference rules: covered by `crates/smelt-db/src/type_inference/*::tests` (pure-function unit coverage, per References → Tests).
- ❌ **§"Lists and spread" rule 7 (empty-list spread elision) is not upheld in the CLI codegen path.** Spec: `...[]` must elide itself and adjacent commas before SQL reaches the engine. Observed: `smelt run` on `examples/meta_lists` emits `...[],` verbatim → DuckDB `Parser Error: syntax error at or near ".."`. The meta body is never meta-evaluated in the run pipeline. (BUG-006, Class 2.)
- ❌ **§"Compile-time variables" / §"Reflection" accessors are mis-classified as model dependencies by the CLI dependency validator.** `smelt run` on `examples/meta_hofs`/`meta_polish` → "references undefined model/source 'config.var'"; on `examples/meta_workspace` → "references undefined model/source 'with_tag'". The Salsa/LSP resolver correctly excludes these meta accessors; the CLI resolver does not. (BUG-006, Class 1 — asymmetric discovery, same class as BUG-005.)
- ⚠️ §"Reflection (`smelt.columns_of`)" — Known Divergence line 1235 states reflection "is not yet implemented". The *type-checker* path IS implemented (diagnostic codes exist; `examples/meta_columns` is LSP-clean), but the *expansion/codegen* of `columns_of` is genuinely absent from the run pipeline, consistent with the KD. The KD is therefore accurate for codegen but its blanket "not yet implemented" understates that type-checking + diagnostics have landed. Non-blocking; noted for the human pass.

### Invariant drift
- ⚠️ **Run Pipeline Parity (CLI ↔ UI)** — both entry points share `smelt_runtime::execute_project`, so the missing in-model meta-expansion affects the UI path identically (not a CLI-only divergence). The invariant is upheld (parity preserved); the gap is in the shared pipeline. Cannot be auto-verified end-to-end because no `execute_parity`-style gate exists (see BUG-001).
- ✅ HOF/reducer/ternary/record/map closed-registry invariants: enforced by exhaustive broken-case fixtures (`meta_hofs_broken_*`, `meta_polish_broken_*`, `meta_workspace_broken_*`, `meta_columns_broken_*`).

### Timeless-oracle drift
- ✅ No phase-vocabulary leakage in the spec body (lines < 1233). Phase references appear only in §Known Divergences (paired with `docs/plans/...` links) and §References → Plans (history) — both tolerated.
- ✅ No `Phase [A-Z0-9]` matches in `docs-site/docs/meta-language/`.

### Freshness
- last_reviewed: 2026-05-16
- The spec carries an extensive, honest Known Divergences section (lines 1233–1254) covering reflection codegen, record/Map `file_diagnostics` wiring, LSP backend dispatch, generator-driver limits, and lifted-identifier wiring.
- Verdict: **fresh** for the documented surface; the in-model-meta-in-run-pipeline gap (BUG-006) is a newly-surfaced behavioural divergence not captured by the existing KDs for the non-reflection constructs (spread, `config.var`, `with_tag`).

### Summary
- Drift items: 1 consolidated behavioural finding (BUG-006) with two root-cause classes — CLI dependency mis-extraction (Class 1, `logical_graph.rs:117`) and missing in-model meta-expansion in the run pipeline (Class 2, `commands/run.rs` → `smelt_runtime::execute_project`).
- Status: **needs-review** — Class 2 touches the Run Pipeline Parity invariant and requires meta-expansion wiring in `smelt-runtime`; Class 1's isolated fix delivers no buildable example and touches the resolver-parity seam with genuine `models`-prefix ambiguity. Logged for the post-sweep human pass; not auto-fixed.
- Recommended next step: human decision on whether to (a) wire in-model meta-expansion into `execute_project` + add an end-to-end gate, or (b) scope the shipped `meta_*` examples as LSP-only fixtures and document the run-pipeline limitation in §Known Divergences.
