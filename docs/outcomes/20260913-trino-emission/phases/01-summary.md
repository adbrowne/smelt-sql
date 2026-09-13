# Phase 1 summary — Trino's emission surface stated in `multi_backend.md`

**Shipped:**
- `docs/specs/multi_backend.md` gains Trino to six places: §"Parity contract" (Trino's scope —
  full-refresh + ephemeral only, maintenance excluded, Iceberg per-table-commit reason),
  §"CI tiering" (the `trino-integration` job's trigger set, and the never-skip-green rule for its
  live legs), §"Operator lowering" (`^`→`POWER`, native `%`, `//`→`DIV`/plain-`/`/refuse,
  `::`→`CAST` alongside GoogleSQL/Spark), §"Clause-level dialect refusals" (no `QUALIFY`, no
  trailing commas, the open `PIVOT` decision, the positive `[a, b]` array-literal finding),
  §"Cross-engine emission audit" (Trino's live-execution schema leg, DuckDB-reference value leg,
  the new Trino gates-by-tier row, and the general `unverified`/`passing`/`gap` three-way rule
  that closes the implicit-`Native` hole for every dialect, not just Trino), and §Known
  Divergences (both existing Trino entries now name the closing phases).
- `crates/smelt-cli/tests/trino_emission_spec_freshness.rs` — 6 new standing freshness tests,
  red before the spec edits, green after.
- The §Surface `SMELT_TRINO_URL` sentence now scopes its skip claim to the backend's own
  integration tests and explicitly excludes the audit legs, resolving the contradiction with the
  never-skip-green rule.

**Decisions:**
- The `unverified`/`passing`/`gap` three-way vocabulary is stated as a **general rule over all
  dialects** (Trino is the dialect that forced it, not a special case) — this is the exact
  language phase 2's coverage gate must reuse, per the plan's instruction.
- `::`'s Trino verdict is framed via the existing `supports_double_colon_cast` capability flag
  (already `false` for GoogleSQL/Spark/Trino in the matrix) rather than as a new registry
  narrative, since `::` is a capability-flag lowering, not a `BuiltinRegistry` operator entry —
  more accurate than treating it identically to `^`/`//`.
- Left the capability matrix's `supports_pivot` row (currently `✓` for Trino) untouched: T1's
  hand-forward says Trino measured `supports_pivot: false`, so that cell is already wrong, but
  correcting it is phase 4's decision (the lowering-vs-refusal call), not phase 1's.

**For the next planner:**
- **Known inconsistency to resolve in phase 4**: `docs/specs/multi_backend.md`'s capability
  matrix (§Surface, `supports_pivot` row) currently shows Trino as `✓`, but the T1 hand-forward
  and this outcome's own text state Trino measured `supports_pivot: false`. Phase 4 must flip
  that cell when it lands the PIVOT decision — flagging so it isn't missed.
- Phase 2's coverage gate should reuse the exact three-way vocabulary (`unverified`/`passing`/
  `gap`) fixed here rather than inventing new terms.
- No production code touched — spec-only phase as planned.

**Gates:**
- `cargo test -p smelt-cli --test trino_emission_spec_freshness` — 6/6 passed
- `cargo test -p smelt-cli --test trino_spec_freshness` — 5/5 passed (T1's gate stayed green)
- `rg -n 'Phase [A-Z0-9]|Historical name|pre-cut|ratified|category error' docs/specs/multi_backend.md` — no matches
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `git status --porcelain .claude/` — empty (no baseline files touched)
