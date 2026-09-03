## Drift Report: incremental_shapes

**Spec**: docs/specs/incremental_shapes.md (last_reviewed: 2026-08-16 → corrected to
2026-09-03 in this phase)
**Date**: 2026-09-03

### Scope note

Same scope note as the companion `incremental_models` report: this targets outcome
`20260815-definition-delta-migrate`'s criterion 19 closure plus the general divergence-drift
discipline, not a from-scratch full-spec audit.

### Criterion 19 cross-check

- ✅ "A window-forward keyed run with no event-time window silently full-refreshes instead of
  refusing" — bullet removed. Confirmed shipped: `crates/smelt-runtime/src/execute.rs`
  refuses the windowless window-forward keyed arm unless `--full-refresh` is set (phase 29,
  commit `af20dfe3`); regression test
  `crates/smelt-runtime/tests/keyed_run_window_required.rs`.
- ✅ "`safety_overrides:` on a key-addressed model is not a hard error" — bullet removed.
  Confirmed shipped: `MetadataError::KeyedForbidsSafetyOverrides` /
  `DiagnosticCode::KeyedForbidsSafetyOverrides`, named in §"Key-grain declaration" and the
  Surface diagnostics table (line 358).
- `rg -n "silently full-refreshes|safety_overrides.*hard error is not"
  docs/specs/incremental_shapes.md` — no hits for either retired phrase.

### Freshness (finding, corrected in this phase)

- `last_reviewed` read `2026-08-16`, but `git log -1 --format=%cI --
  docs/specs/incremental_shapes.md` shows the file's own most recent commit
  (`af20dfe3`, phase 29) is `2026-09-03T14:47:43+10:00` — the phase 29 edit (deleting the two
  criterion-19 bullets, adding the `KeyedForbidsSafetyOverrides` surface entry) did not bump
  the frontmatter date. **Fixed in this phase**: `last_reviewed` set to `2026-09-03`.

### Out-of-scope cross-check

The remaining "The key grain" §Known Divergences bullets (once-write nullability route,
re-run-tolerant frontier write, `KeyedRetractableContribution`, ledger transactionality,
`smelt explain` guarantee ledger, locality route 2/3 gaps, derived execution postures,
generative pool NULL payload, pattern-function template file, `NOW()`/`CURRENT_*` rejection,
departed-key retention, ladder rungs 3-4, `key_per_partition`) all map to one of: the
still-`queued` `20260815-keyed-grain-residue` sibling outcome, the "Genuinely large product
calls" list, or a named decision record
(`docs/research/20260816-open-questions-triage.md`). No orphans found in this section — this
spec's residues are already tracked by the sibling outcome the 2026-08-15 revision spun out.

The "The partition grain" §Known Divergences bullets similarly map to
`20260815-partition-grain-residue` (still `queued`) or a cited plan/decision record. No
orphans found.

### Timeless-oracle check

`grep -nE "Phase [A-Z0-9]+"` over the body (excluding References→Plans links) — no hits.

### Automated checks

Deferred to the shared gate sweep (`phases/31-summary.md` § Gates).

### Summary

- Drift items: 1 (stale `last_reviewed` — fixed in this phase). No bullet-removal drift; both
  criterion-19 bullets already correctly absent.
- Recommended next step: none.
