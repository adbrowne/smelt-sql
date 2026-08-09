# Phase 1 summary — Spec: the repair family

**Shipped:**
- `docs/specs/incremental_models.md`: new §"The repair family" (after §"Per-cell write
  addressing") — what it is, why it's correct (slice completeness reuses key temporal locality),
  admission obligations, ledger grading.
- New obligation 7 in §"Per-cell admission" (affected-key discovery), cited by number from §"The
  repair family" rather than restated.
- `diff_patch` write pattern: added to the registry enum, given its own subsection under §"The
  write-pattern set is open" (insert/update/delete legs, contract facts required, delete-leg
  slice-completeness gating, Idempotent grade).
- Refusal narrowing: §"Reprocessing" now routes a merged-window change through the repair family
  before refusing; `KeyedReprocessedWindow` and `KeyedRetractableContribution` prose (both spec
  files) name the failing repair obligation instead of refusing unconditionally.
- §"The plan matrix" 2×2 prose + technique bullet gain a one-sentence cross-reference: the repair
  family lands in the column-scoped re-derivation corner, not a fifth corner.
- Two new diagnostics, `MaintenanceRepairKeysNotDiscoverable` / `MaintenanceRepairSliceUnbounded`,
  added to both `incremental_models.md` §Diagnostics and `diagnostics.md` (with an unimplemented
  note).
- `docs/specs/model_properties.md`: new §Derived proofs row + §Semantics section "Affected-key
  discovery" (`derive_affected_keys`) — sound-over-approximation rule, fail-closed
  `NotDiscoverable`. §Interactions updated (seven → eight proofs, obligation list 2/4/5/6/7).
- New Known Divergences entry in `incremental_models.md` naming this outcome as the tracking
  artifact for the unbuilt derivation/emission.

**Decisions:**
- Corner placement: the repair family lands in the **column-scoped re-derivation** corner (full
  read, targeted write) of the 2×2, not a new corner — it's recompute-a-region's targeted-write
  refinement, but the write dimension is what moves.
- The three admission obligations are not a parallel list: (a) derivable group key and (b)
  bounded per-group read footprint are the *existing* obligations 6 and 4 respectively, cited by
  number; only (c) affected-key discovery is genuinely new (obligation 7).
- Slice completeness — the repair family's correctness premise — is not a new proof; it reuses
  key temporal locality verbatim. `diff_patch`'s delete-leg gating cites the same premise rather
  than restating it.
- `diff_patch`'s delete leg is conditional on slice completeness; without it the pattern degrades
  explicitly to insert+update (stated, not silently dropped).

**For the next planner:**
- Phase 2 implements `derive_affected_keys` (`model_properties.md` §"Affected-key discovery") —
  the entry point name and verdict shape (`Keys{cols}` / `NotDiscoverable{reason}`) are fixed by
  this phase's spec text.
- Phase 3 (per-group recompute technique) needs the corner placement decision (column-scoped
  re-derivation, not a new corner) to slot correctly into existing plan-cell derivation code.
- Phase 5 (refusal narrowing wiring) touches both `KeyedReprocessedWindow` and
  `KeyedRetractableContribution` call sites — both diagnostics now need to carry which repair
  obligation failed, not just fire unconditionally; check `crates/smelt-db` diagnostic mapping
  and `crates/smelt-logical` classifier sites for both codes.
- Not done, out of scope for this phase: no Rust changes at all (spec-only phase, per plan).
- Pre-existing, unrelated: the local dev environment's `DUCKDB_LIB_DIR` in `~/.bashrc`/shell
  default points at a location with no `libduckdb.so`; the working library lives at
  `~/.local/lib/duckdb`. Needed `DUCKDB_LIB_DIR=/home/andrew/.local/lib/duckdb` and matching
  `LD_LIBRARY_PATH` to get `verify-phase.sh` and `cargo test` to link at all. Worth fixing in the
  shell profile or `.claude/scripts/verify-phase.sh` defaults if it recurs.

**Gates:**
- `bash .claude/scripts/verify-phase.sh --fast` — PASS (fmt, clippy, example_diagnostics).
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — PASS (1 passed).
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md docs/specs/model_properties.md
  docs/specs/diagnostics.md` — two pre-existing hits, both permitted (a Known-Divergences entry
  paired with a plan link; the diagnostics.md timeless-oracle rule's own text). No new hits.
- Diagnostic-code resolution: both new codes appear in `diagnostics.md`; all new `§"…"`
  cross-references resolve to real headings (spot-checked with `rg`).
- Full `cargo test --quiet` (background run, `DUCKDB_LIB_DIR=/home/andrew/.local/lib/duckdb`) —
  PASS, exit code 0.
