# Outcome: Every built-in's Trino spelling is a stated, probed verdict — no built-in reaches Trino on an implicit `Native`

**Created:** 2026-09-13
**Status:** active
**Driver:** loop. Docker only, no credential, no human gate. The audit's live legs must emit
`<<PHASE_BLOCKED>>` when the coordinator is unreachable, **never skip green** — an audit that
skips is indistinguishable from an audit that passes, which is the precise failure this outcome
exists to prevent.
**Depends on:** `docs/outcomes/20260913-trino-target-spine` (T1) — the audit's legs execute SQL
against a live engine, so the HTTP client and the compose tier must exist first.
**Source:** T2 of the five-outcome Trino programme agreed 2026-09-13. Pattern followed:
`docs/outcomes/20260904-dialect-emission-vocabulary` (how the emission vocabulary and its gates
were established) and the `dialect_audit` harness's own two-leg design.
**Spec anchors:** `docs/specs/multi_backend.md` §"Parity contract", §"Operator lowering",
§"Clause-level dialect refusals", §"Emission is scoped to call position", §"Template emission",
§"Operand-conditional verdicts", §"Statement-level lowering", §"Cross-engine emission audit",
§"Output-schema type conformance"; `docs/specs/architecture.md` §"Constraints & Invariants"
item 14 (function-registry single ownership); `docs/reference/dialect-coverage.md`

## The outcome

Trino's column in the emission registry is honest. Every built-in smelt recognises either
carries an explicit `(DialectId::Trino, Position, Emission)` verdict, or has been *executed*
against a live Trino coordinator in both directions — does the printed SQL run, and does it
compute the same value — and recorded as verified. Nothing arrives at Trino on the registry's
silent default.

That default is the whole problem. `Signature::emission_at` ends "*a pair with no entry at all
is `Native`*", so the moment `DialectId::Trino` exists, several hundred built-ins claim Trino
spells them exactly as smelt does. That claim compiles, passes every offline test, and is
wrong for at least `^` (no such operator on Trino), `//`, `::`, `[a,b]`, trailing commas,
`QUALIFY` and `PIVOT`. A gate closes the hole structurally: an entry with neither an explicit
Trino verdict nor an audit-verified `Native` is **named and fails**, never quietly counted as
covered — the same "untested `Native` is *unverified*, not *passing*" rule the `Emission` doc
comment already states.

Two consequences of Trino's shape are settled here rather than discovered later. Trino has no
`PIVOT`, which makes it the first backend with `supports_pivot: false` — so either the lowering
exists or the construct is refused at compile time with `UnsupportedOnBackend`, and never
emitted as SQL Trino would reject. And because Trino offers several built-ins only in the
opposite call position from the one an author may write, the `Restructure` path
(`docs/specs/multi_backend.md` §"Statement-level lowering") is exercised on a fourth dialect,
including its null-safe join spelling.

## Success criteria (checkable)

1. **The implicit-`Native` hole is closed by a gate, not by diligence.** A standing test
   enumerates every registry entry and, for `DialectId::Trino`, fails while naming any entry
   whose verdict is neither explicitly stated in `signatures/` nor verified by an audit probe.
   A newly-added built-in therefore cannot silently acquire a Trino claim. The gate reports
   *unverified* distinctly from *passing* and from *gap*.
2. **The known divergences carry explicit verdicts.** At minimum `^` (smelt's power operator,
   which Trino has no operator for), `//`, `::`, `[a,b]` array literals, trailing commas,
   `QUALIFY`, `LOG`/`DAYOFWEEK`-family renames and the `FIRST`/`LAST` family have stated
   `Rename`/`Template`/`Rewrite`/`Restructure`/`Unsupported` verdicts with the reason recorded,
   validated at registry construction (placeholder range, argument coverage, no variadic
   template, window-position call shape) by the existing `registry_coverage` checks.
3. **`PIVOT` is decided, not defaulted.** Trino's measured `supports_pivot: false` (T1
   criterion 5) is answered either by a lowering that produces SQL Trino executes to the same
   rows, or by a compile-time `UnsupportedOnBackend` refusal with a diagnostic and a fixture.
   Which, and why, is a decision-log entry. What does not happen is emitting `PIVOT` to Trino.
4. **The audit has a live Trino leg in both directions.** `cargo test -p smelt-db --test
   dialect_audit` gains Trino probes **derived from the registry, not authored against it**,
   run against the live coordinator: a schema leg (does the printed SQL run?) and a value leg
   (does it compute the same thing?) — the leg that catches an operator whose spelling survives
   but whose meaning changes. Coverage totality holds: an entry with no probe is *named*, never
   dropped.
5. **The ledger is two-sided for Trino.** `crates/smelt-db/tests/dialect_audit/ledger.rs` gains
   Trino rows, and the existing two-sided rule applies unchanged: an unregistered mismatch
   fails, **and so does** a registered row the engine now accepts, or a row naming a pair that
   is never probed. `.claude/dialect-gaps-baseline.txt` gains a Trino metric with a tracking
   issue, ratcheting **down only**.
6. **The published table tells the truth.** `docs/reference/dialect-coverage.md` regenerates
   with a Trino column via `SMELT_REGEN_DOCS=1`, and the doc-sync gate keeps it from drifting.
7. **Ownership is not diluted.** `cargo test -p smelt-dialect --test emission_ownership` stays
   green: the printer module holds no name-matched Trino spelling, no branch on
   `SqlDialect::Trino`, and no derivation of a call's position. A per-dialect spelling is
   `Signature::emission` data; a capability-shaped difference is a `BackendCapabilities` flag.
   Every `RewriteId` and `RestructureId` variant remains dispatched.
8. **Refusal happens at compile time, on the compile path.** `cargo test -p smelt-runtime
   --test dialect_seam` covers Trino: a construct the registry declares `Emission::Unsupported`
   on Trino fails at compile time with `UnsupportedOnBackend`, and no compile entry point
   reaches the printer without that check. Clause-level refusals (`QUALIFY`, and whatever else
   T1 measured as absent) refuse inside function bodies too, per §"Refusal covers function
   bodies".
9. **Projection stays source-derived across four dialects.** `cargo test -p smelt-runtime
   --test projection_dialect_invariance` compiles one model exercising every construct the
   printer lowers and asserts `output_columns` and the cast-wrap column names are
   **byte-identical** across DuckDB, Spark, BigQuery **and Trino** — the guard against
   re-parsing Trino's own printed lowering, the bug class recorded for `MEDIAN`.
10. **The statement-level restructure works on Trino.** Where Trino offers a built-in only in
    the opposite position, the `Restructure` path plans from the source CST (never from printed
    SQL), synthesises its join null-safely with Trino's own spelling, and — for a running-frame
    window over a built-in with no analytic form — refuses with `UnsupportedOnBackend` rather
    than emitting something plausible. Each is asserted against live Trino output.
11. **Types are conformant, or the divergence is registered.** The inferred output type of a
    Trino-targeted projection matches Trino's own reported schema exactly (integer width,
    `DECIMAL` precision/scale, `TIMESTAMP(p)` precision included), with the only blanket
    leniency the existing named string-family rule; every other tolerated difference is an
    explicit registered divergence and every `Unknown` inference matches a registered entry.
    **Or**: a decision-log entry defers the Trino type-oracle leg with a named reason and a
    tracking issue — deferral by explicit decision is acceptable, deferral by silence is not.
12. **Gates green.** `bash .claude/scripts/verify-phase.sh` passes; no ratchet lowered; the
    registry-migration and parser-gaps baselines are untouched (this outcome adds no
    hand-written `match` typing and no parser work).

## Out of scope

- **The backend, the client and the tier** — all T1's (`20260913-trino-target-spine`).
- **smelt's own bookkeeping state** (`20260913-trino-ledger`) and **the maintenance statement
  families** (`20260913-trino-incremental`). Emission here means *expression and clause*
  spelling inside a model's SQL; a `MERGE` statement's shape is the incremental outcome's.
- **Trino as a parser source dialect.** No `smelt-parser-compat` differential, external corpus
  or `.claude/parser-gaps-baseline.txt` work: smelt SQL is the source dialect, Trino is a target.
- **Lowering every registered gap.** A `Gap` row with a tracking issue is a legitimate landing
  state; the ratchet exists so gaps shrink over time. This outcome's job is that every gap is
  *registered and probed*, not that the count reaches zero.
- **Other connectors' emission columns** — the Iceberg profile is the one dialect column.
- **Retiring or reshaping another dialect's verdicts.** If a Trino probe reveals a mistake in
  the DuckDB, Spark or BigQuery column, it is recorded and handed on, not fixed here.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec delta: `multi_backend.md` gains Trino to §"Operator lowering", §"Clause-level dialect refusals", §"Cross-engine emission audit" (which legs run where, and Trino's per-PR-vs-nightly tier), and the §"Parity contract" statement for a fourth dialect | planned |
| 2 | The coverage gate first, red: a standing test naming every registry entry with no explicit Trino verdict and no audit verification, distinguishing *unverified* from *passing* from *gap* — landed before any verdict, so the hole is visible as a failure | pending |
| 3 | Explicit verdicts for the operator and clause divergences (`^`, `//`, `::`, `[a,b]`, trailing commas, `QUALIFY`) with registry-construction validation, plus the `BackendCapabilities` flags they pair with | pending |
| 4 | The `PIVOT` decision: lower it to SQL Trino executes to the same rows, or refuse it at compile time with a diagnostic and a fixture — recorded either way | pending |
| 5 | The live `dialect_audit` Trino leg, schema direction: registry-derived probes executed against the coordinator, coverage totality enforced (no silent drop), `report.rs` rendering Trino | pending |
| 6 | The value direction — the leg that catches a spelling that survives but changes meaning — plus the two-sided Trino `ledger.rs` rows and the `.claude/dialect-gaps-baseline.txt` Trino metric with its tracking issue | pending |
| 7 | `Restructure`/`Rewrite` on a fourth dialect: position-opposite lowering planned from the source CST, null-safe synthesised join in Trino's spelling, and the running-frame refusal — each asserted against live output | pending |
| 8 | Seams: `dialect_seam` Trino refusal coverage (incl. inside function bodies), `emission_ownership` still green with no printer Trino branch, `projection_dialect_invariance` widened to four dialects byte-identically | pending |
| 9 | The type-oracle question: land the Trino leg with its divergence registry and `Unknown` census, or record an explicit deferral decision with a tracking issue — never silence | pending |
| 10 | Close: regenerate `dialect-coverage.md` with the Trino column, doc-sync gate green, `verify-phase.sh` green, §Known Divergences rewritten to drop T1's implicit-`Native` divergence now that the gate closes it | pending |

## Decision log

- **2026-09-14 — hand-forward from `20260913-trino-target-spine` phase 11.** T1 closed leaving
  the implicit-`Native` emission hole open (`Signature::emission_at` claims every built-in is
  natively spelled on Trino with no probe backing that claim — `docs/specs/multi_backend.md`
  §Known Divergences). Measured grammar gaps this outcome should treat as its starting probe
  list: no `QUALIFY`, no `::` cast, no trailing commas, no `PIVOT` clause, no pipe syntax
  (`|>`); the `[a,b]` array-literal syntax *does* work on Trino (unlike Spark). No
  `AUDITED_DIALECTS` entry exists for Trino yet in `dialect_audit` — phase 5/6 above is the
  first Trino leg of that gate.
- **2026-09-14 — outcome activated, phase 1 planned.** No reshape: T1's hand-forward is
  already absorbed into the phase table (the measured gap list drives phases 3–5, and the
  positive finding that `[a, b]` array literals work on Trino is a stated `Native` verdict
  rather than a lowering). Phase 1 additionally owns reconciling the §Surface `SMELT_TRINO_URL`
  *skip* sentence with this outcome's never-skip-green discipline for the audit legs — the two
  statements would otherwise contradict each other in the same spec.

## Blocked
