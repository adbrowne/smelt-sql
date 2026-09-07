# Phase 1 summary — Spec closure delta

**Shipped:**
- `docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden state)": new **Physical
  shape** paragraph — per-model sibling table `<presented table>__tombstones`, reserved
  relation-name suffix (same terms as the reserved `__` column suffix), columns exactly
  `k ∪ {t}` (verdict's `key_cols` then `clock_col`) each `NOT NULL`, PK `(k, t)`, lifecycle tied
  to the presented table.
- `docs/specs/incremental_shapes.md` §"Succession-grain constraints": new constraint 12 —
  `frozen_horizon`/`retain_departed` refused by the existing partition-grain-only /
  mutable-snapshot-only rules naming the succession grain; `deferral` admitted unchanged.
- `docs/specs/incremental_shapes.md` §Known Divergences "The succession grain": one residual
  bullet — a hand-authored model ending in `__tombstones` collides silently, no dedicated
  diagnostic (code budget held at twelve).
- `docs/specs/state.md` §"The state-structure inventory": Tombstone ledger Residency cell now
  names the per-model sibling table instead of a bare "backend table".
- `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report": delta-signature
  headline gains the succession shape text and `keyed_succession`/`event` enum values (append-
  only addition to an already append-stable field); new **Succession grain** paragraph pinning
  the text-block line order and the `succession` JSON object's fields.
- `docs/specs/incremental_models.md` §"Contract relaxations (`contract:`)": one sentence
  recording the same posture from the lattice's own spec.
- `last_reviewed` bumped to 2026-09-06 on all four edited specs.

**Decisions:**
- No new `DiagnosticCode` for the `__tombstones` name collision — recorded as behaviour
  ("collides silently") per the plan's explicit code-budget constraint, not a code to add.
- Contract-lattice: confirmed no new lattice point; wrote the refusal rationale in
  `incremental_shapes.md` itself (constraint 12) rather than only in `incremental_models.md`,
  since phase 3's tests will assert against the succession spec's own constraint list.

**For the next planner:**
- **Pre-existing, unrelated test flake found**: `crates/smelt-core/tests/baseline/
  materialize_tests.rs::checkout_scratch_is_deleted_when_materialization_fails` fails
  consistently under `cargo test`'s default parallel threads (races against sibling tests over
  a shared `/tmp/smelt-baseline-*` scratch-directory listing) and passes reliably under
  `--test-threads=1`. Confirmed unrelated to this phase: the diff here touches only
  `docs/specs/*.md`, and the failure reproduces identically with a clean working tree run of
  `cargo test -p smelt-core`. Not scheduled here (out of scope for a spec-only phase, and not
  named in this outcome's success criteria) — worth a hygiene fix (unique scratch-dir naming or
  narrower directory scan) whenever a code-touching phase is in flight, or its own outcome.
  `verify-phase.sh`'s other three gates (fmt, clippy, example_diagnostics) are green.
- Phases 3/4/8 now have a pinned oracle for the ledger DDL shape, the contract refusals, and the
  explain rendering — no further design decisions expected there for these three surfaces.

**Gates:**
- `bash .claude/scripts/verify-phase.sh`: fmt PASS, clippy PASS, `cargo test (workspace)` FAILED
  on the pre-existing unrelated flake above, `example_diagnostics` PASS. Isolated re-run
  (`cargo test -p smelt-core --test baseline -- --test-threads=1`): 21/21 PASS.
- `rg -n 'Phase [A-Z0-9]' docs/specs/incremental_shapes.md docs/specs/cli.md docs/specs/state.md docs/specs/incremental_models.md`:
  only the pre-existing timeless-oracle-rule banner in `cli.md` (not new content).
- No new `DiagnosticCode` name anywhere in the delta (confirmed by diff grep).
