# Phase 1 summary — spec: decomposed-state semantics

## Shipped

- New `docs/specs/incremental_models.md` §"Decomposed state (rung 2) in keyed models"
  (after §"The algebraic maintenance ladder"): physical layout (`<output>__<part>`
  columns in the same stored table, rejects a separate state table + view),
  presentation projection + naming-collision refusal, the state-shape catalogue table
  (`AVG`→`(sum,count)`, `STDDEV_*`/`VAR_*`→`(n,Σx,Σx²)`, `MAX_BY`/`MIN_BY`→`(v,o)`,
  once-write→`(value,written)`), and the fold-rule/monoid argument per shape.
- §"The column-family catalogue" table: order-monotone overwrite's "extra licence" is now
  hidden `(v,o)` state, not a hand-written companion projection; added a **decomposed
  fold** family row (`AVG`/`STDDEV_*`/`VAR_*`, ledger-graded additive); once-write's
  combiner cell and prose widened to four admitted spellings (bare key-derived,
  no-fallback single reduction, fallback-bearing reduction, multi-candidate reduction).
- §Diagnostics: `KeyedUnknownCombiner`/`KeyedOnceWriteUnproven` prose updated for the
  above; new `KeyedStateColumnCollision` code (mirrored into `docs/specs/diagnostics.md`).
- Admission matrix (§"Admission matrix (column family × source shape)") and Derived
  execution postures (§"Derived execution postures") gained the decomposed-fold row/entry.
- §"The maintenance boundary" (Design) corrected: the keyed families no longer all sit on
  the direct-monoid rung — additive/decomposed-fold on their respective monoid+group
  rungs, extremal/order-monotone-overwrite/once-write on rung 1/2 monoid-not-group.
- Three §Known Divergences entries rewritten to residual-gap framing (implementation, not
  spec, is what's missing now), pointing at this outcome:
  once-write narrow-spellings entry, order-monotone ordering-value entry, and the
  "ladder rungs 2–4" entry split into a rung-2 entry (now spec'd, wiring pending) and a
  rungs-3–4 entry (still unspec'd, unchanged scope).
- `docs/specs/model_properties.md`: discriminants section gains a pointer to the state
  catalogue; the functional-dependency declaration row and drops the
  NULL-preservation-by-spelling framing now that decomposed state discharges it.
- `docs/specs/model_transforms.md` §"Hidden decomposed state..." bullet updated to name
  the full state-shape catalogue as the mechanism's target, not just `AVG`.

## Decisions

- Physical layout and naming (`<output>__<part>`) were pre-decided by the outcome's
  decision log; this phase just wrote the normative text and reused the exact suffix
  convention `crates/smelt-logical/src/analysis/decomposed_state.rs` already uses for
  `AVG` (`__sum`/`__count`), so phase 2+ needs no rename.
- Once-write's fallback/multi-candidate widening is expressed as "the raw reduction is
  state, the fallback/preference order is applied in `π`" — this is the mechanism that
  actually fixes the old refusal reason (preference order not preserved across windows),
  not just a relaxed admission rule.
- `MAX_BY`/`MIN_BY` and once-write's decomposed-state combiners keep the exact
  order-independence/idempotence caveats their rung-1 forms already had (ordering-key
  ties; per-key-constant provenance) — decomposing to state changes *where* the value
  lives, never the invariant's strength.
- `AVG`/`STDDEV_*`/`VAR_*` are graded **additive** (not idempotent) in the merge ledger,
  matching their `SUM`-shaped state components — this is a new fact for phase 6's
  conformance-recipe grading to encode correctly.

## For the next planner

- Phase 2 (derive state shapes in `smelt-logical`) should extend
  `decomposed_state.rs::decompose_to_state` to cover `STDDEV_*`/`VAR_*`, `MAX_BY`/`MIN_BY`,
  and once-write — today only `AVG` is encoded (confirmed while reading the module).
  `combiner_discriminants` already reports `decomposable` for the variance family per
  `model_properties.md`; check whether it's set for `MAX_BY`/`MIN_BY` and once-write too,
  or whether those need their own discriminant classification (they're not classically
  "decomposable" combiners in the F4 sense — they're order-monotone / once-write, so the
  admission-widening plumbing may need a different entry point than
  `combiner_discriminants(...).decomposable`, worth checking before phase 2 commits to a
  design).
- `KeyedStateColumnCollision` has no implementation yet — needs a classifier and test.
- The `π` purity proof (`analysis::presentation::presentation_map_purity`, F7 in
  `model_transforms.md`) needs to accept the new presentation expressions (`sum/count`
  already covered; the variance closed forms, `v`-only projection, and the
  fallback/preference-order `COALESCE` forms are new shapes it hasn't seen).
- Out of scope for this phase, flagged for later phases per the outcome's own scope:
  `smelt explain`'s actual rendering of state columns as internal state (success
  criterion 4) is phase 7 work; this phase only fixes the normative claim.
- No test changes: this was a spec-only phase per its own header.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy, full `cargo test`,
  `example_diagnostics`) — all green, unchanged by a markdown-only diff.
- `bash .claude/scripts/verify-phase.sh --fast` — re-run after the consistency fixes
  found during self-review (Design section, admission matrix, derived postures) — PASS.
- `rg -n "Phase [A-Z0-9]"` over the four edited spec files — only pre-existing hits
  (a `Phase 6` plan-link inside Known Divergences, and the diagnostics.md rule-statement
  itself, which names the banned pattern) — no new violations.
- `rg -n "companion"` — three surviving mentions, all either normative
  ("needs no companion projection") or Known-Divergences entries paired with a plan link.
- Every diagnostic code named in the edits (`KeyedUnknownCombiner`,
  `KeyedOnceWriteUnproven`, `KeyedStateColumnCollision`) resolves in
  `docs/specs/diagnostics.md`.
