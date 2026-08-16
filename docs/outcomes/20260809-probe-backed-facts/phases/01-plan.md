# Phase 1 — Spec: the probe obligation rule

**Outcome:** `docs/outcomes/20260809-probe-backed-facts/outcome.md`
**Advances success criteria:** 3 (admissibility rule stated, each declaration names its
probe + firing semantics), 4 (firing → named diagnostic + remedy, in spec form), 5 (cadence
surface). Sets the oracle every later phase implements against.

## Objective

State, once and normatively, the rule that a *narrowing* declaration is admissible only if a
probe can falsify it, and give every existing declaration a named probe with firing semantics,
a diagnostic, a remedy, and a cadence. `sources.md` already carries the trust rule for source
declarations (widening trusted / narrowing verified); this phase generalises it to
**model-scoped** declarations and adds the per-declaration probe registry that phases 2–6
implement. Spec-only phase: no production behaviour changes.

## Spec delta (spec-first; the implement step makes these edits)

1. **`docs/specs/model_properties.md`** — the normative home for the rule.
   - New §Semantics section **"Probe obligation"** (placed after §"Model-scoped declarations"'s
     concepts are in scope, i.e. as the first Semantics subsection): the admissibility rule —
     *a declaration that narrows what maintenance reads or licenses a cheaper technique is
     admitted only paired with a probe that can disprove it at run time; no probe, no
     declaration*. State: what a probe is (a read-only query the maintenance layer emits and the
     driver inspects **before** any write, so a violation rolls back trivially), that a firing
     probe is always a named error diagnostic naming the violated fact and the remedy
     (repair/refresh the affected cells), never a warning and never a silent continue, and that
     a probe is never a substitute for a proof's default reject.
   - **Probe registry table** in that section, one row per declaration:
     `Declaration | Probe | What it queries | Fires when | Diagnostic | Remedy | Default cadence | Status`.
     Rows: `assert_monotonic`, `functional_dependencies:`, `bounded_domain:`,
     source `mutation_profile: append_only` posture, source `referential_integrity`
     (probe exists: `emit_count_preservation_probe`), source `key_recurrence`
     (built: `emit_recurrence_bound_probe`), source `unique_key`/`delta_identity`
     (uniqueness probe, owned by `sources.md`). `horizon_ceiling:` and the widening-only
     declarations are listed as **exempt** with the one-line reason (they widen nothing that
     narrows; a ceiling is warning-only).
   - §Constraints: add **"No narrowing declaration without its probe"** (the model-scoped twin
     of `sources.md` Constraint 8), and amend the "Declared escape hatches may only widen"
     bullet to point at the new section.
   - §Known Divergences: rewrite the `referential_integrity` "admitted ahead of its runtime
     verification" sentence in the skeleton-source-closure entry to name the tripwire's
     wiring gap as the tracked gap (this outcome), and add one gap entry per registry row whose
     Status is not `built`.
2. **`docs/specs/sources.md`** — §"Verification mechanisms" / Constraint 8 reference the new
   section by name instead of restating the rule; keep source-side probe mechanics here.
3. **`docs/specs/diagnostics.md`** — one unified-table row per new probe diagnostic:
   `DeclaredMonotonicityViolated`, `DeclaredFunctionalDependencyViolated`,
   `DeclaredBoundedDomainExceeded`, `SourceAppendOnlyViolated` (if not already present) — each
   Error, fails the consuming run transactionally, with the remedy in the "when it fires" text.
   Add a §Known Divergences line marking them specified-and-unimplemented, landing in this
   outcome's phases 2–4.
4. **`docs/specs/smelt_yml.md`** — the cadence surface: a project-level `probes:` block with
   `cadence: per_run (default) | periodic | off`, and `periodic:` taking `every_n_runs`.
   State that `off` is recorded on the run so `smelt explain` can report an unverified
   declaration, and that per-declaration override is out of scope for now (one line in
   §Known Divergences).

## Tests (red-green)

New gate `crates/smelt-logical/tests/probe_obligation.rs`, parsing the spec text:

1. `every_narrowing_declaration_has_a_probe_row` — every declaration named in
   `model_properties.md` §"Model-scoped declarations" and in `sources.md`'s narrowing list
   appears in the probe registry table with a non-empty Probe, Fires-when, Diagnostic, Remedy,
   and Cadence cell, or an explicit `exempt` row with a reason.
2. `probe_registry_built_rows_name_a_real_emitter` — every registry row whose Status is `built`
   names a `pub fn emit_*` that exists in `crates/smelt-logical/src/maintenance/emit.rs`
   (source-text scan), so the table cannot claim a probe that does not exist.
3. `probe_registry_diagnostics_are_catalogued` — every Diagnostic cell value appears in
   `docs/specs/diagnostics.md`'s unified table.
4. `probe_obligation_section_states_the_rule` — the §"Probe obligation" heading exists and its
   body contains the admissibility sentence and the never-a-warning clause (lint-level, cheap).

## Tasks

1. Write §"Probe obligation" + the registry table in `model_properties.md`; fill Status from the
   repo (`emit_recurrence_bound_probe` = built+wired; `emit_count_preservation_probe` = emitter
   built, unwired; rest = not-yet).
2. Add the two §Constraints edits and the §Known Divergences gap entries.
3. Update `sources.md` §"Verification mechanisms" + Constraint 8 to reference, not restate.
4. Add the four diagnostic rows + divergence line to `diagnostics.md`.
5. Add the `probes:` block to `smelt_yml.md` §Surface (+ its parse rules line in §Semantics).
6. Write `crates/smelt-logical/tests/probe_obligation.rs` (red first against the un-edited spec,
   then green).
7. Grep-lint the edited specs: `rg -n 'Phase [A-Z0-9]|Historical name|ratified' docs/specs/{model_properties,sources,diagnostics,smelt_yml}.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test probe_obligation`
- `rg -n '§"Probe obligation"' docs/specs` resolves to the one heading.

## Commit message

`spec(probes): the probe obligation rule — per-declaration probe registry, firing semantics, cadence`
