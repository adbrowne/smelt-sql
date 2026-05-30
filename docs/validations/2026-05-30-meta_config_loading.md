## Drift Report: meta_config_loading

**Spec**: docs/specs/meta_config_loading.md (last_reviewed: 2026-05-14)
**Date**: 2026-05-30
**Phase**: B5 (feature-sweep)

### Automated checks
- cargo fmt — PASS (`cargo fmt --all -- --check` clean)
- cargo clippy — PASS (zero warnings, all-targets)
- cargo test — PASS (full workspace green; pre-flight baseline clean before probing)
- example_diagnostics — PASS (88 tests; 87 passed, 1 ignored — includes `examples/meta_config` loader acceptance gate)

### Surface drift
- ✅ **Loader calls** — `smelt.config.load_yaml` / `load_json` dispatch present (`type_inference/loader_and_reflection.rs`); `load_toml` reserved → `ConfigLoaderTomlNotYetSupported`.
- ✅ **Schema authoring** — inline record, named `smelt.record`, `List<S>`, `Map<Text, S>` all admissible; non-admissible → `ConfigLoaderSchemaForbidden`.
- ✅ **Path resolution** — literal-only (`ConfigLoaderPathNotLiteral`), workspace-escape (`ConfigLoaderPathEscapesWorkspace`), backslash (`ConfigLoaderPathBackslash`), missing file (`ConfigLoaderFileNotFound`). Salsa input registration via `LoaderFileInput`.
- ✅ **Validation diagnostics** — all 13 `ConfigLoader*` codes defined (`diagnostics_types.rs:509-546`) and emitted in production code (loader.rs / queries/loader.rs / type_inference). Content-validation wired into the LSP diagnostics path (`loader_call_diagnostics_for_file`, loader.rs:581).
- ❌ **Per-target overlay** — **drift (BUG-014)**. Spec §"Per-target overlay" asserts `smelt build --target X` reads `<basename>.<target>.<ext>`, merges it (List replace / record deep-merge / Map per-key replace), and validates the overlay file. The merge logic exists and is unit-tested (`loader_resolved_value_with_overlay`, loader.rs:220; overlay merge tests at loader.rs:1634/1694/1743) but has **zero production callers** — the generator-expansion path `collect_loader_values` (project.rs:1325) always calls base-only `loader_resolved_value` (project.rs:1411) and smelt-db carries no build-target input. End-to-end repro: `examples/meta_config_overlay_probe` built `--target prod` emits the base value (`revenue >= 100`, not the overlay's `>= 999`); an invalid overlay file builds exit-0 with no diagnostic. Not listed in §Known Divergences.

### Semantics drift
- ✅ **Workspace containment / literal path / schema admissibility / TOML-reserved** — covered by `type_inference/tests.rs`.
- ✅ **Per-format YAML/JSON parsing, scalar coercion, root-shape, duplicate-key, null-coercion** — covered by `loader.rs::tests` (record/list/map roots, parse errors, null coercion, JSON strict-mode).
- ✅ **Salsa-tracked inputs / deterministic re-evaluation** — `loader_file_text_is_salsa_input`, `loader_resolved_value_invalidated_on_file_change`, `overlay_file_change_invalidates_loader_value` (tests.rs).
- ❌ **Validation produces diagnostics in the run pipeline** — **drift (BUG-015)**. §Constraints "Validation produces structured diagnostics" + §Surface validation codes hold in the LSP path but **not** the CLI run/build pipeline: a schema-violating config silently drops the generated model (`smelt build` exit 0, model vanishes from `explain --json`). Root: `collect_loader_values` swallows loader diagnostics (project.rs:1433) and `execute_project` doesn't run `file_diagnostics`. Same asymmetric class as BUG-006/BUG-011.
- ⚠️ **Overlay diagnostics anchoring** — §"A target overlay file that does not validate … anchored at the overlay file's offending row" — unverifiable because overlay resolution never runs in production (subsumed by BUG-014).

### Invariant drift
- ✅ **No file access outside workspace / no network** — path-canonicalisation + scheme rejection upheld (`ConfigLoaderPathEscapesWorkspace`).
- ✅ **Schema is the source of truth for the loaded type** — `infer_loader_call_smelt_type` synthesises the schema's type.
- ✅ **Deterministic re-evaluation** — Salsa-memoized `loader_resolved_value`; no clock/random/env in the loader path.
- ❌ **"Salsa inputs cover every loaded file … Per-target overlay files are also Salsa inputs"** — overlay files are **not** registered/consulted in production (BUG-014). The base file is a Salsa input; the overlay is never read.

### Timeless-oracle drift
- ✅ Spec body is clean. `grep -nE "Phase [A-Z0-9]+"` matches only inside §Known Divergences (paired with `docs/plans/...` links) and §References → Plans (history) — both tolerated. (Note: a `LoaderFileInput` *code* comment at `lib.rs:200` says "Phase 6 will add per-target overlay inputs" — that is source-code phase vocabulary, not spec/user-doc drift; it correctly marks the unlanded overlay wiring behind BUG-014.)

### Freshness
- last_reviewed: 2026-05-14
- most recent code change to References → Code: loader subsystem actively maintained; spec's Known-Divergences list is current except that it **overclaims** per-target overlay as implemented (BUG-014). The spec is otherwise fresh; the overclaim is a content fix, not a staleness sweep.
- Verdict: **fresh structurally**, but §Surface "Per-target overlay" + §Known-Divergences need a content correction once BUG-014 is resolved (wire overlay, or demote to a divergence).

### Summary
- Drift items: 2 (BUG-014 — per-target overlay unwired in production, major, architectural; BUG-015 — run-pipeline doesn't surface loader content diagnostics, major, BUG-006 class). Both logged `needs-review` (architectural / systemic; the loop does not auto-fix or edit specs).
- The loader **layer** (parsing, schema validation, scalar coercion, Salsa invalidation, all 13 diagnostic codes) is mature and well-tested; the gaps are at the run-pipeline wiring seam (overlay resolution + diagnostic surfacing), consistent with the BUG-006 family.
- Recommended next step: for the post-sweep human pass — resolve BUG-014 via option (a) (thread build target into smelt-db, register overlay inputs, call `loader_resolved_value_with_overlay`) and fold BUG-015 into the BUG-006 run-pipeline-diagnostic-gating decision. No spec edit in-loop.
