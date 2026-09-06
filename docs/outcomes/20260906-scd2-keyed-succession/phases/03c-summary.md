# Phase 3c summary — gate hygiene: path drift after the large-file splits

**Shipped:**
- `crates/smelt-logical/tests/support/module_source.rs` — shared `read_module(repo_root, rel_stem)`
  helper: resolves `<stem>.rs` if it still exists, else concatenates every non-test-only `.rs`
  file under `<stem>/` (reusing `test_only_files::is_test_only`). Four unit tests, including the
  panic-on-absence case.
- `contract_lattice_spec::frozen_horizon_triple_is_complete` and
  `::explain_contract_rendering_is_single_owned` now read via `read_module` instead of a single
  vanished `.rs` path; the latter's ownership check is strengthened to an exactly-once scan
  (`matches("pub fn effective_contract").count() == 1`) over the whole `contract/` module rather
  than presence in one file.
- New `contract_lattice_spec::gate_detects_a_missing_leg` — negative proof (fixture module missing
  `clamp_write_range`) that the rewritten reads still fail on a real absence.
- `docs/specs/state.md` §References fixed to cite `maintenance/availability/` (was `.rs`).
- Swept all six anchor specs (`incremental_shapes`, `incremental_models`, `model_properties`,
  `model_transforms`, `diagnostics`, `sources`) plus `state.md` for dead backtick-quoted
  `crates/...` paths; fixed 8 more stale `<x>.rs` → `<x>/` citations (mechanical, module now a
  directory) across `incremental_shapes.md`, `incremental_models.md`, `diagnostics.md`,
  `sources.md`, plus two citations in `diagnostics.md` that pointed at a flat
  `crates/smelt-db/tests/{diagnostics_catalogue,struct_field_type}.rs` when the tests actually
  live under `tests/integration/`.

**Decisions:**
- Raised `.claude/large-file-baseline.txt`'s `contract_lattice_spec.rs` entry 450 → 488 lines
  (`large-file-check.sh --update`) rather than splitting the file. The growth is the new negative
  proof test plus two `#[path]` includes — legitimate gate-hygiene content, not accretion, and the
  file remains at 488/1500 of the default cap. The script's own docs name a baseline bump with a
  sign-off note as one of three valid responses to growth; this note is it.
- `state_docs_freshness::spec_references_are_live` needed no code change — it already resolves
  a directory path via `.exists()`; the only fix needed was correcting the spec's citation text.

**For the next planner:**
- The dead-citation sweep was scoped to this outcome's six anchor specs (+ `state.md`) per the
  plan; other `docs/specs/*.md` files were not swept and may carry the same `<x>.rs` drift from
  the earlier large-file-split branches (worth a one-off sweep, not urgent).
- `verify-phase.sh` is fully green on this branch now — phases 4–10 have a clean baseline to
  build on. No remaining red gate to hand to phase 10.

**Gates:**
- `cargo test -p smelt-logical --test contract_lattice_spec` — 23 passed.
- `cargo test -p smelt-cli --test state_docs_freshness` — 4 passed.
- `bash .claude/scripts/verify-phase.sh` — fmt-check, clippy (both feature sets), full
  `cargo test` (workspace), `example_diagnostics` — ALL GREEN.
