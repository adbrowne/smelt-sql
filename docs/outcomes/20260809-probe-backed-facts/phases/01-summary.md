# Phase 1 summary — Spec: the probe obligation rule

**Shipped:**
- `docs/specs/model_properties.md` §Semantics gets a new first subsection **"Probe obligation"**:
  the admissibility rule ("no probe, no declaration"), what a probe is (read-only,
  before-any-write), the never-a-warning/never-a-silent-continue firing rule, and an 8-row
  **probe registry table** (`assert_monotonic`, `functional_dependencies:`, `bounded_domain:`,
  source append-only posture, `referential_integrity`, `key_recurrence`, `unique_key`/
  `delta_identity`, plus two `exempt` rows — `horizon_ceiling:` and superseded
  `nondeterministic_columns`) — and a **"Probe cadence"** subsection.
- §Constraints: new bullet **"No narrowing declaration without its probe"** (twin of
  `sources.md` Constraint 8); §Known Divergences: rewrote the `referential_integrity`
  "admitted ahead of verification" sentence to name the probe by row, and added a bullet
  listing the five not-yet-built registry rows.
- `docs/specs/sources.md` — Semantics §4 and Constraint 8 now cross-reference
  `model_properties.md` §"Probe obligation" as the generalized rule, keeping their own
  source-specific mechanics as the concrete instance.
- `docs/specs/diagnostics.md` — three new rows (`DeclaredMonotonicityViolated`,
  `DeclaredFunctionalDependencyViolated`, `DeclaredBoundedDomainExceeded`) plus two
  previously-missing-from-the-unified-table rows the registry now cites
  (`SourceMutationProfileViolated`, `SourceUniqueKeyViolated` — both already normatively
  defined in `sources.md`'s own local table but never propagated into the unified catalogue);
  a §Known divergences bullet marking the three new codes specified-and-unimplemented.
- `docs/specs/smelt_yml.md` — new top-level `probes:` key (`cadence: per_run | periodic |
  off`, `periodic.every_n_runs`), a Semantics bullet, and two Known Divergences bullets
  (unimplemented in `Config`; per-declaration override open).
- `crates/smelt-logical/tests/probe_obligation.rs` — 4 tests, all green: every expected
  declaration has a registry row or exempt row with all cells filled; every `built`-status
  row names a real `pub fn emit_*` in `crates/smelt-logical/src/maintenance/emit.rs`; every
  named diagnostic is catalogued in `diagnostics.md`; the section states the admissibility
  and never-a-warning/never-silent-continue sentences.

**Decisions:**
- 2026-08-10: Registry rows for `referential_integrity` and `key_recurrence` cite the two
  existing emitters (`emit_count_preservation_probe`, `emit_recurrence_bound_probe`); the
  other five declarations get `not-yet` rows describing the probe's shape without naming a
  nonexistent function, so the `built`-only emitter-existence gate stays honest.
- 2026-08-10: `diagnostics.md`'s unified table was missing `SourceMutationProfileViolated`
  and `SourceUniqueKeyViolated` even though `sources.md`'s own local diagnostic table already
  defines them — added both rather than inventing new codes or weakening the test, since the
  registry needed to cite real, catalogued names.
- 2026-08-10: `probes:` is spec-only — no `Config` field exists yet, so an authored block
  today just parses as a warned-on unknown key (harmless per `smelt_yml.md` §"Unknown keys").
  Flagged explicitly in Known Divergences rather than left implicit.

**For the next planner:**
- Phase 2 (probe emitters for FD, `bounded_domain`, append-only posture, `assert_monotonic`)
  has its target function names implied by the registry table's Probe cells (not literal —
  the table describes the mechanism, not a locked-in symbol name) and must add the three new
  `DiagnosticCode` variants plus wire `SourceMutationProfileViolated`'s append-only leg.
- Phase 2/4 also needs `probes:` landed in `crates/smelt-core/src/config.rs` (currently
  absent) before cadence has any runtime effect — flagged here so it isn't rediscovered.
- Out of scope, correctly deferred: `diagnostics.md`'s unified table is still missing other
  `sources.md`-local codes not touched by this phase's registry (e.g. `SourceRetentionExceeded`)
  — a pre-existing drift, not this outcome's job to fully reconcile, but worth a note if a
  later diagnostics-focused phase sweeps the file.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace test,
  example_diagnostics). Required `DUCKDB_LIB_DIR=~/.local/lib/duckdb` +
  `LD_LIBRARY_PATH=~/.local/lib/duckdb:$LD_LIBRARY_PATH` — `/usr/local/lib` has no
  `libduckdb.so` in this worktree.
- `cargo test -p smelt-logical --test probe_obligation` — 4/4 passed.
- `rg -n '§"Probe obligation"' docs/specs` — resolves; multiple cross-references, one owning
  heading.
