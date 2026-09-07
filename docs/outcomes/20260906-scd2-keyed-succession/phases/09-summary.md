# Phase 9 summary — Fixture and docs for the succession grain

**Shipped:**
- `examples/scd2_succession/` — a new worked-example workspace: `models/sources/customer_changes.yml`
  (arrival-partitioned, `append_only`, `is_deleted NOT NULL`) and `models/customer_history.sql`
  (row-local projection + `LEAD` + `QUALIFY NOT is_deleted`), plus a `README.md`.
- `docs-site/docs/guide/scd2-succession.md` — the user guide: why no declaration is needed, the
  admitted SQL outline, the derived `(k, t)` identity, the delete filter, the pre-window clamp,
  arrival- vs event-time partitioning, the tombstone ledger, a `smelt explain` sample, and a
  refusal-code table. Navigated from `docs-site/mkdocs.yml` (after "Incremental Models") and
  cross-linked from `docs-site/docs/guide/incremental-models.md`.
- `docs-site/docs/reference/diagnostics.md` gained a `## Succession grain` section: all twelve
  codes with severity/trigger, plus a worked `### Example: SuccessionDeleteFilterMisplaced`.
- `docs/specs/incremental_shapes.md` §References §"The succession grain" gained a **User docs**
  bullet.
- `examples/README.md`'s directory table gained the `scd2_succession/` row.
- Five new tests: `scd2_succession_no_diagnostics` (Salsa/CLI), `scd2_succession_workspace_clean`
  (real LSP), `example_workspace_customer_history_is_a_succession_cell` (grain/identity/technique/
  ledger recognition against the real fixture, not a staged one),
  `docs_site_diagnostics_reference_lists_every_succession_code` (two-sided code-list parity),
  `succession_guide_page_is_navigated_and_covers_the_grain` (nav + content coverage).

**Decisions:**
- `customer_changes` has no seed data (a declared-only source), so `smelt build` fails in the
  standalone build env; added to `example_builds.rs`'s `KNOWN_UNBUILDABLE` allow-list following
  the existing convention (`source_mutation_profile_declared`, `horizon_ceiling_tight`, …) rather
  than seeding data the fixture doesn't need for its purpose (succession recognition is already
  proven end-to-end by the conformance/explain suites). Logged in outcome.md Decision log.
- `crates/smelt-lsp/tests/example_workspaces.rs` grew 7 lines past its large-file baseline for
  one new per-example test; bumped with a sign-off note rather than restructuring the file (a
  mechanical one-test addition matching the file's existing pattern).

**For the next planner:**
- Phase 10 (validate and close) still needs to rewrite the "No implementation exists yet" /
  "generative conformance recipes … do not exist yet" / "conformance pool has no
  arrival-partitioned source recipe" Known Divergences entries in `docs/specs/incremental_shapes.md`
  (lines ~1901-1928) — all now stale, since phases 2-8 landed the classifier, emitters, runtime
  dispatch, and the arrival-partitioned recipe family. This phase deliberately left them alone
  (out of phase 9's scope per its own spec-delta note).
- Nothing else was deferred; all five planned tests and both docs tasks landed as specified.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full workspace
  test including the newly-added `KNOWN_UNBUILDABLE` entry, `example_diagnostics`)
- `cargo test -p smelt-lsp --test example_workspaces` — PASS (36 tests)
- `cargo test -p smelt-cli --test explain_maintenance` — PASS (54 tests)
- `cargo test -p smelt-cli --test cli_docs_coverage --test docs_front_door --test explain_docs_freshness` — PASS
- `cd docs-site && uv run mkdocs build` — built clean (pre-existing unrelated anchor warnings only)
- `bash .claude/scripts/large-file-check.sh` — PASS (after the sign-off baseline bump above)
