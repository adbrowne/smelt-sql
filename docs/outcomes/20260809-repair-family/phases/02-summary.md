# Phase 2 summary — Delta discovery names affected keys

**Shipped:**
- `crates/smelt-logical/src/analysis/affected_keys.rs` — `derive_affected_keys(delta, sql, ctx)`,
  pure and walk-backed. `AffectedKeys::{Keys{cols}, NotDiscoverable{reason}}` (`Serialize`),
  `DeltaShape { source, columns, keyed }`, `AffectedKeyContext { unique_key, join }`.
- Registered in `analysis/mod.rs` (alphabetical, before `bounded_domain`).
- `docs/specs/model_properties.md` §Surface "Derived proofs" — Affected-key discovery row:
  `not-yet` → `partial (proof derived; not yet consumed by plan derivation)`.
- 10/10 plan tests green: 9 unit tests in `affected_keys.rs`, 1 integration test
  (`crates/smelt-logical/tests/affected_keys.rs`, the CTE-rename-chain case).

**Decisions:**
- Grain resolution mirrors `maintenance::derive::row_identity_with_context`'s exact precedence:
  declared `unique_key` first, else `model_property_vector`'s proven `Grain` key gated on
  `!has_fan_out_join`, else `NotDiscoverable` (row_identity instead falls back to `WholeRow` —
  affected-keys has no such fallback, per the plan).
- Reused `analysis::fingerprint`'s leaf classifier (`classify_ref`/`scan_expr_for_source`/
  `relation_matches_source`) by widening their visibility to `pub(crate)` rather than copying
  them — a new `GrainProvenanceTransfer` in `affected_keys.rs` calls them, restricted to the
  grain-column subset of the select list instead of every projected column (`fingerprint`'s own
  `select_projection` stays untouched and its 15 existing tests are unaffected).
  Introduced zero new `.contains("` sites — `walk_coverage` gate green with no new leaf-classifier
  tags needed.
- A grain column resolving with zero dependency on the delta's source (e.g. a literal, or a
  column from an unrelated source) is treated as "no requirement" rather than a refusal — no
  spec example covers this corner; it reads consistently with the sound-over-approximation
  license (§"Affected-key discovery") but is worth flagging to the next planner as an
  under-specified edge the spec prose doesn't name explicitly.

**For the next planner:**
- Phase 3 (per-group recompute technique) is unblocked: `derive_affected_keys` exists but is not
  called from anywhere in `maintenance::derive` yet — wiring is phase 3's job per the outcome's
  phase table.
- The zero-dependency corner above (grain column independent of the delta's own source) has no
  test coverage and no spec sentence pinning its verdict; if phase 3's admission logic depends on
  it, worth a one-line spec clarification first.
- `AffectedKeyContext` has no builder beyond `with_unique_key` — phase 3 will likely want a
  `JoinContext`-carrying constructor too if a repair cell's admission needs to fold in the same
  per-edge declared-key facts `row_identity_with_context`'s callers already do.

**Gates:**
- `cargo test -p smelt-logical --lib affected_keys` — 9 passed.
- `cargo test -p smelt-logical --test affected_keys` — 1 passed.
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed (no new raw-scan violations).
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings, full workspace test,
  example_diagnostics).
