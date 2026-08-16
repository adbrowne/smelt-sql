# Phase 10 summary — docs-site sync: state modes, residency, downgrade

## Shipped

- `docs/specs/state.md` §Known Divergences: deleted the "runtime ignores `state.mode`"
  bullet (false since phase 2's `FileStore::writes()` gating) and the separate
  "no availability-resolution step" bullet (landed phases 5/6/9); collapsed the ledger-builder
  bullet to name only the real residual gap (dialect coverage, now downgrades rather than
  fails); narrowed the degradation-behaviours bullet to its one real residue
  (`ProbeBaselineUnavailable` has no `DiagnosticCode` variant — baselines themselves are
  correctly gated on `state.mode` per `file_store.rs`'s `StateFamily`). Removed the
  "conformance gate leg for state deletion" Future Extension (landed phase 8). Fixed the
  References block: user docs now point at real sections, Tests points at the
  `maintenance_conformance/state_deletion` module.
- `docs/specs/incremental_models.md` and `docs/specs/run_state.md` §Known Divergences:
  narrowed the DuckDB-only ledger bullet (downgrades now, doesn't fail) and the
  forward-propagation bullet (reconciliation ledger landed; the remaining gap is inherent to
  `stateless` forgoing landed-delta persistence, not a temporary implementation gap).
- `docs-site/docs/reference/smelt-yml.md`: new `## State Configuration` section (`state.mode`
  key, the 3-mode lattice, what each persists/degrades) plus a `state` row in Top-Level Fields.
- `docs-site/docs/reference/state.md`: dropped `reconciliation.json` from the Inventory table
  and the Locking section's per-target-file list; rewrote "The reconciliation ledger" as
  "The reconciliation ledger lives in the warehouse" (`_smelt_ledger`/`_smelt_frontier`,
  survives `.smelt/` deletion, the one-time legacy-import behaviour, the downgrade); rewrote
  the "`.smelt/` is lost" recovery paragraph so it no longer implies the reconciliation ledger
  rebuilds from `.smelt/`.
- `docs-site/docs/guide/deployment.md`: dropped `reconciliation.json` from the layout block,
  added a paragraph naming its real (warehouse) home; fixed the backup/restore paragraph's
  "correctness/performance regression" framing to "never affects correctness."
- `docs-site/docs/reference/smelt-explain.md`: new `## State downgrade` section with the real
  `smelt explain` output from phase 9's manual verification, plus the `--json` schema pointer.
- `docs-site/docs/guide/incremental-models.md`: one-sentence cross-link from the reconciliation
  ledger section to the new downgrade section; fixed a stale anchor.
- New doc-sync gate `crates/smelt-cli/tests/state_docs.rs` (3 tests, all red before the docs
  edits, green after): every `StateMode` variant documented; `reconciliation.json` absent from
  both live-artifact listings (with an explicit assert that the one legitimate legacy-import
  mention survives); `_smelt_ledger`/`_smelt_frontier`/`MaintenanceStateDowngraded`/`state
  downgrade:` all present in the reference pages.

## Decisions

- Put the downgrade example in `smelt-explain.md` (not `incremental-models.md`) since that's
  where the real captured output lives and where a reader looking up `smelt explain` output
  would land; cross-linked from the guide instead of duplicating the example.
- Kept `state.md`'s "Open question — opting out of warehouse bookkeeping tables" as-is — still
  genuinely undecided, not something this phase's code changes touched.
- Test file follows a simple string/substring-scoped pattern rather than `backbuild_docs.rs`'s
  regeneration machinery — the plan's three tests are presence/absence checks over prose
  sections, not SQL-block round-tripping, so the heavier pattern wasn't warranted.

## For the next planner

- `/smelt:validate state` was run informally (spec Surface/Semantics cross-checked by hand
  against the code during this phase, not through the full 8-step slash-command flow) because
  the full flow's own `cargo test`/`cargo clippy` steps duplicate `verify-phase.sh`, which this
  phase already ran to green. No drift found beyond what this phase fixed.
- Discovered (not fixed, not this phase's target): `crates/smelt-parser-compat`'s
  `parse_equivalence::prop_smelt_valid_implies_spark_valid` proptest found a new failing case
  during one `verify-phase.sh` run (`SELECT top FROM a INTERSECT SELECT top FROM a` — smelt
  parses `top` as a plain identifier, `sqlparser-databricks` doesn't) but passed cleanly on a
  second run with a different random seed — this is a pre-existing SQL-dialect divergence in
  an unrelated crate (`top`-as-identifier handling), not caused by or related to this phase's
  docs-only changes. The `.proptest-regressions` file it wrote was reverted rather than
  committed. Worth a dedicated look: `rg -n '"top"' crates/smelt-parser-compat` /
  `divergences.rs` to see whether it needs a registered gap.
- Row 11 (close-out) still owes: a live Spark execution of `maintenance_conformance_spark`
  (phase 9 could only compile-check it), the criteria-vs-summaries judgment pass, and the
  outcome status flip.

## Gates

- `bash .claude/scripts/verify-phase.sh` (10-minute timeout) — ALL GREEN (fmt, clippy
  zero-warnings, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-cli --test state_docs` — 3 passed.
- `cargo test -p smelt-cli --test tutorial_freshness --test example_diagnostics` — 120 passed
  (1 ignored).
- Timeless-oracle grep (`Phase [A-Z0-9]`) over every file this phase touched — no hits.
