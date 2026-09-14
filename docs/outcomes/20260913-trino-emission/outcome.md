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
| 1 | Spec delta: `multi_backend.md` gains Trino to §"Operator lowering", §"Clause-level dialect refusals", §"Cross-engine emission audit" (which legs run where, and Trino's per-PR-vs-nightly tier), and the §"Parity contract" statement for a fourth dialect | done |
| 2 | The coverage gate first, red: a standing test naming every registry entry with no explicit Trino verdict and no audit verification, distinguishing *unverified* from *passing* from *gap* — landed before any verdict, so the hole is visible as a failure | done |
| 3 | Explicit verdicts for the operator and clause divergences (`^`, `//`, `::`, `[a,b]`, trailing commas, `QUALIFY`) with registry-construction validation, plus the `BackendCapabilities` flags they pair with | done |
| 4 | The `PIVOT` decision: lower it to SQL Trino executes to the same rows, or refuse it at compile time with a diagnostic and a fixture — recorded either way | done |
| 5 | The live `dialect_audit` Trino leg, schema direction: registry-derived probes executed against the coordinator, coverage totality enforced (no silent drop), `report.rs` rendering Trino | done |
| 6 | The value direction — the leg that catches a spelling that survives but changes meaning — plus the two-sided Trino `ledger.rs` rows and the `.claude/dialect-gaps-baseline.txt` Trino metric with its tracking issue; phase 2's coverage census reaches zero here and the file is deleted, not grandfathered | done |
| 7 | `Restructure`/`Rewrite` on a fourth dialect: position-opposite lowering planned from the source CST, null-safe synthesised join in Trino's spelling, and the running-frame refusal — each asserted against live output | done |
| 8 | Seams: `dialect_seam` Trino refusal coverage (incl. inside function bodies), `emission_ownership` still green with no printer Trino branch, `projection_dialect_invariance` widened to four dialects byte-identically | planned |
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
- **2026-09-14 — phase 1 done.** `multi_backend.md` now states Trino's emission surface in all
  six places the plan named. The `unverified`/`passing`/`gap` three-way rule (§"Cross-engine
  emission audit") is written as a general rule over every dialect, which phase 2's gate should
  reuse verbatim. Flagged for phase 4: the §Surface capability matrix's `supports_pivot` row
  still shows Trino as `✓`, contradicting the measured `false` — left uncorrected here since the
  PIVOT lowering-vs-refusal call is phase 4's, not phase 1's.

- **2026-09-14 — phase 2 planned; no phase-table reshape beyond one clarification.** Phase 1's
  summary surfaced no work needing a new row (the `supports_pivot` matrix cell it flagged is
  already phase 4's). One mechanism decision the outcome could not have anticipated: the spec
  says an `unverified` pair is a *failure*, but phase 2 lands the gate before any Trino verdict
  exists, so every pair is unverified and a plain failure would leave `verify-phase.sh` red for
  four phases and stall the loop. Resolved the way this repo already resolves it elsewhere — a
  two-sided, shrink-only census file (`.claude/trino-emission-census.txt`) naming every
  outstanding pair, modelled on `.claude/dialect-gaps-baseline.txt`: a pair *not* in the census
  fails immediately, which is exactly criterion 1's structural claim (a newly-added built-in
  cannot silently acquire a Trino claim), and the census is deleted at zero rather than
  grandfathered. Phase 2 carries the one-paragraph spec delta stating this; phase 6's row is
  amended to own the deletion.
- **2026-09-14 — phase 2 done.** The coverage gate landed: `Signature::stated_emission_at`
  distinguishes a stated verdict from the implicit `Native` default (`emission_at` now
  delegates to it); `crates/smelt-db/tests/dialect_audit/census.rs` classifies every
  `(entry, position)` pair for a dialect into `Stated`/`Verified`/`Gap`/`Unverified`, with a
  ledger `Gap`/`Divergent` row checked ahead of the registry's own verdict (a pair the registry
  states `Native` for that a live sweep found broken is `Gap`, not `Stated`). 237 pairs are
  `Unverified` for Trino today, all named line-for-line in `.claude/trino-emission-census.txt`
  (generated via `SMELT_REGEN_TRINO_CENSUS=1`); phases 3–6 drive that count to zero and delete
  the file. `report::applicable_positions`/`position_label` were widened to `pub(crate)` so the
  census module reuses the coverage table's own position axis rather than re-deriving it.

- **2026-09-14 — phase 3 planned; no phase-table reshape.** Phase 2's summary named no work
  requiring a new row: the census is the ratchet phases 3–6 already own, and its regeneration
  step is folded into each of their task lists rather than becoming a phase. One spec correction
  surfaced while reading the code phase 3 touches, and it is phase 3's own spec delta rather than
  a new row: §"Operator lowering" (written by phase 1) claims Trino's `//` integral arm lowers to
  `DIV(a, b)` "as GoogleSQL and Spark SQL" do, but Trino has no `DIV` function and its `/` already
  truncates toward zero on integral operands and divides plainly on floating/decimal ones — the
  same class-sensitivity DuckDB's `//` has. So Trino's `//` is a single unconditional
  `Template("{0} / {1}")`, not a `Conditional`, and the spec sentence is corrected in phase 3's
  commit. The claim is to be settled against a live coordinator first; if the tier cannot be
  brought up, phase 3 states the verdict and hands the four unverified answers to phase 6's value
  leg rather than blocking — the never-skip-green discipline binds the audit legs (phases 5–6),
  not this offline registry phase.
- **2026-09-14 — phase 3 done.** Live tier settled before writing anything: `bash
  scripts/trino-up.sh` + `source scripts/trino-env.sh`, then `SELECT 7/2` (`3`), `SELECT -7/2`
  (`-3`), `SELECT 7.5/2.0` (`3.750000`), `SELECT DIV(7,2)` (`FUNCTION_NOT_FOUND: Function 'div'
  not registered`) — confirms the plan's analysis exactly, so `//` on Trino is one unconditional
  `Template("{0} / {1}")`, not a `Conditional`; `docs/specs/multi_backend.md` §"Operator lowering"
  corrected accordingly. Registry rows landed for all five infix operators (`%` → `Native`, `^`/
  `**` → `Template("POWER({0}, {1})")`, `//` → `Template("{0} / {1}")`, `||` → `Native`), no
  printer change needed (`emission_ownership` stays green). The four clause divergences (`QUALIFY`,
  `::`, trailing commas, `[a,b]`) needed no registry or printer work either — `trino_iceberg()`'s
  capability flags already drive the existing generic dispatch; `trino_clause_lowering.rs`'s four
  tests only prove it. Census regenerated: 237 → 232 rows, exactly the five operators. Unrelated
  finding: `verify-phase.sh`'s workspace test leg was red on `large_file_ratchet` for six files
  (`resume.rs`, `link_c_harness.rs`, `execute/project/mod.rs`, `contract_deferral_skip_e2e.rs`,
  `key_addressed_model_edge_lowering.rs`, `repair_lowering.rs`) already 1 line over baseline at
  HEAD, from commits before this phase touched anything (`.claude/large-file-baseline.txt` last
  regenerated at `a642eb91a`, itself unrelated to this outcome). Resynced via
  `large-file-check.sh --update` rather than blocking phase 3 on unrelated pre-existing drift —
  flagged for the next planner in case it recurs and warrants investigation.

- **2026-09-14 — phase 4 planned; no phase-table reshape.** Phase 3's summary named no work
  needing a new row (its one hand-forward, the pre-existing six-file large-file baseline drift,
  is advisory and serves no success criterion — it stays out of the table). The decision
  criterion 3 demands is taken here rather than left to the implement step: **`PIVOT`/`UNPIVOT`
  on Trino is refused at compile time, not lowered.** Reason: smelt already refuses both for
  *every* target at the diagnostic layer (`check_unsupported_constructs`,
  `UnsupportedConstruct` — output columns depend on data values), and a Trino lowering would
  have to enumerate the `IN`-list values to name its own output columns, which is exactly the
  projection smelt declines to derive and must not recover from printed SQL
  (`architecture.md` §"Source-derived projection"). So no lowering is admissible, and the real
  gap is narrower than the phase row assumed: the *dialect* layer would print `PIVOT` verbatim
  to Trino, because `compile_with_sql` runs no diagnostics query. Phase 4 closes that with a
  clause-level refusal keyed on a new `SqlDialect::supports_pivot`, in the same place
  §"Clause-level dialect refusals" already puts the aggregate-`FILTER` and `INTERVAL`-frame
  refusals — no printer branch on `SqlDialect::Trino`, so `emission_ownership` stays green.
  Also surfaced while reading: the spec contradicts itself on this flag — §Surface's matrix row
  and its Spark-prior paragraph say Trino accepts `PIVOT`, while §"Operator lowering" says it
  was measured `false`. Phase 4 settles it against a live coordinator first (phase 3's
  precedent), blocks rather than guesses if the tier cannot be brought up, and if the probe
  measures `true` states a `Native` verdict and corrects the other sentence instead.
- **2026-09-14 — phase 4 done; live measurement split the plan's binary assumption.**
  Against a live coordinator: `PIVOT (COUNT(id) FOR cat IN ('a'))` executes cleanly, but
  `UNPIVOT (val FOR name IN (a,b,c))` fails to parse (`mismatched input 'UNPIVOT'`) — the
  plan expected both to measure `false` together under one flag; they diverge. Ruling:
  `BackendCapabilities::trino_iceberg().supports_pivot` stays `true` (already was — no
  code or matrix change), `PIVOT` gets no refusal and no lowering (it's Native). A new,
  separate dialect fact `SqlDialect::supports_unpivot()` (`false` for Trino only) gates a
  new clause-level refusal in `emission_check.rs`, fired only for `UNPIVOT`. The universal
  diagnostic-layer refusal of both constructs, for every backend, is unchanged and remains
  the primary gate; this phase's refusal is the `compile_with_sql` backstop, narrowed to
  the one construct Trino's grammar genuinely lacks. Census unchanged (232 rows — neither
  construct is a registry entry). One unrelated ratchet needed a sign-off: the two new
  `compile.rs` tests pushed that file from 4233 to 4306 lines (`SqlCompiler` test helpers
  are `pub(crate)`, so they can't live in a separate integration-test file); resynced via
  `large-file-check.sh --update`. See `phases/04-summary.md`.

- **2026-09-14 — phase 5 planned; no phase-table reshape.** Phase 4's summary explicitly deferred
  nothing. One mechanism decision the outcome could not have anticipated, settled in the plan rather
  than left to the implement step: `census::classify` returns `Verified` for any `AUDITED_DIALECTS`
  member, so adding Trino to that list — which phase 5 must do, since it is what drives the offline
  totality gates, the fixture gate and the print-for-every-dialect gate — would flip all 232 census
  rows to `Verified` on the strength of a *schema* leg alone, asserting exactly the "unverified =
  passing" equivalence this outcome exists to deny. Resolved by making `Verified` leg-aware: Trino
  joins `AUDITED_DIALECTS` in phase 5 but not the both-legs-live set the census consults, so the
  census stays at 232 rows until phase 6's value leg drives it to zero and deletes it. Phase 6's row
  keeps ownership of the Trino ledger *rows* and the ratchet; phase 5 lands only the
  `dialect_gaps_trino` baseline metric that `baseline_names_exactly_the_audited_dialects` forces the
  moment Trino is audited, plus any `Leg::Schema` row for a pair the live coordinator actually
  rejects. Second decision: the Trino oracle reads the coordinator's reported column metadata and
  decodes no row data, so the recorded `array(...)` Arrow-decode divergence cannot masquerade as a
  rejected probe. Unlike phase 3, this phase has no offline fallback — the legs are the
  never-skip-green ones, so an unreachable tier is `<<PHASE_BLOCKED>>`, not a stated verdict.

- **2026-09-14 — phase 5 done.** The live schema leg found 57 gaps (53 unregistered builtins, 4
  type-leg mismatches), tracked in bulk under #209, plus one entry (`LOG`'s one-argument form)
  that got a real `Emission::Conditional` registry fix rather than a ledger row, since a single
  ledger row cannot express "gap on this arm, pass on the other" when one of the two probes
  carries no arm of its own. `census.rs`'s `Verified` classification now consults a `BOTH_LEGS_LIVE`
  set distinct from `AUDITED_DIALECTS`, exactly as planned — Trino joined the latter only, so the
  232-row (now 217-row, after phases 3/4's own verdicts) census is unaffected. Confirmed (not
  anticipated in the plan): running the full-workspace `cargo test` with the Trino tier exported
  causes namespace/transaction collisions in `smelt-backend-trino`'s own test suite under
  concurrent scheduling — pre-existing, reproduced with and without this phase's changes, avoided
  by running targeted live commands first and `verify-phase.sh` with the tier unexported, matching
  phases 3/4's own pattern. See `phases/05-summary.md`.

- **2026-09-14 — phase 6 planned; no phase-table reshape.** Phase 5's summary hands forward one
  advisory item — the full-workspace-`cargo test`-with-the-Trino-tier-exported concurrency
  collision on the shared Iceberg REST catalog. It serves no success criterion (every phase
  already avoids it by running targeted live commands and then `verify-phase.sh` with the tier
  unexported, and phase 6's plan repeats that discipline), so it stays out of the phase table
  rather than becoming a row. Two mechanism decisions taken in the plan rather than left to the
  implement step. First, the value oracle decodes **raw `/v1/statement` JSON** against the
  coordinator's reported type strings, not `arrow_convert::trino_type_to_arrow`: that converter
  has no array/varbinary/interval arm, and a decode error inside the oracle would be
  indistinguishable from the engine rejecting the probe — the exact confusion phase 5 designed
  out for the schema leg. Second, deleting the census removes the only dialect that could be
  classified `Unverified`, so `census::classify` gains the both-legs set as an explicit parameter
  and keeps a synthetic red-proof; the standing criterion-1 gate becomes
  `no_dialect_has_unverified_pairs` over `DialectId::ALL`, with the audit's own coverage-totality
  and two-sided ledger gates carrying the "a new built-in cannot silently acquire a Trino claim"
  claim from here on. The `.gitignore` whitelist line for the census file is deleted with it.

- **2026-09-14 — phase 6 done.** `TrinoClient::execute_json` + `cell_from_trino_json` decode
  Trino's raw JSON cells (including NaN/Infinity-as-string and nested `array(...)`) without
  `arrow_convert`; `value_leg_trino` compares 125 probes against DuckDB. 8 findings: 7 permanent
  `Divergent` rows (NULL-vs-NaN on degenerate `CORR`/`REGR_SLOPE` variance, `GREATEST`/`LEAST`
  NULL propagation, `DATE_TRUNC` millisecond rendering, `JSON_ARRAY_LENGTH` strict-array
  semantics, `SPLIT_PART` NULL-vs-empty-string) and 1 `Gap` (`JSON_ARRAY`'s missing `NULL ON NULL`
  lowering, #209; `dialect_gaps_trino` 57 → 58). Trino joined `census::BOTH_LEGS_LIVE`; the
  file-backed census and `.claude/trino-emission-census.txt` are deleted, replaced by the standing
  `no_dialect_has_unverified_pairs` gate over `DialectId::ALL`. See `phases/06-summary.md`.

- **2026-09-14 — phase 7 planned; no phase-table reshape.** Phase 6's summary deferred nothing
  out of its own scope; its two hand-forwards are the `JSON_ARRAY` `NULL ON NULL` gap (already a
  registered `Gap` under #209, and "lowering every registered gap" is explicitly out of scope)
  and the shared-catalog concurrency collision (advisory; every phase already avoids it), so
  neither becomes a row. One scope judgement taken in the plan rather than left to the implement
  step: the candidates for Trino's position-opposite lowering (`ARG_MAX`/`ARG_MIN`,
  `APPROX_COUNT_DISTINCT`, `PERCENTILE_CONT`/`PERCENTILE_DISC`) all currently sit in the ledger
  as `#209` *aggregate-position* gaps, so phase 7 necessarily states their aggregate spelling
  too. That is not gap-grinding: a `Restructure(WindowToCte)` verdict emits the aggregate form
  inside the synthesised CTE, so the aggregate verdict is load-bearing for the window one and
  cannot be deferred. `dialect_gaps_trino` therefore ratchets down in this phase. Second: the
  `AnalyticToCte` shape may have no Trino instance at all — the plan requires that answer be
  *measured and recorded* (spec sentence + summary), never left as silence, since the outcome's
  criterion 10 is conditional on "where Trino offers a built-in only in the opposite position".
  As with phases 5/6 and unlike phase 3, there is no offline fallback here: an unreachable
  coordinator is `<<PHASE_BLOCKED>>`.

- **2026-09-14 — phase 7 done.** The measurement landed the negative result phase 6's plan
  anticipated: Trino accepts `MAX_BY`/`MIN_BY`/`approx_distinct` as window functions in every
  position (unlike GoogleSQL), so none of this phase's candidates needed
  `Emission::Restructure`. `ARG_MAX`/`ARG_MIN`/`APPROX_COUNT_DISTINCT` closed to a single
  `Position::Any` `Rename` each, matching their Spark shape; `dialect_gaps_trino` ratcheted
  58 → 55. `percentile_cont`/`percentile_disc` stay `Gap` rows — confirmed live to be
  `FUNCTION_NOT_FOUND` on Trino, not a `WITHIN GROUP` shape mismatch. `AnalyticToCte` is
  recorded in `multi_backend.md` §"Statement-level lowering" as not currently exercised on
  Trino, with the coverage gate (not the prose enumeration) named as what protects a future
  candidate from landing unverified — `registry_coverage::trino_restructure_pairs_with_a_window_refusal`
  is the concrete forward-guard, vacuously green today by design. See
  `phases/07-summary.md` for the full measurement log and gate results.

- **2026-09-14 — phase 8 planned; no phase-table reshape.** Phase 7's summary deferred nothing
  into phase 8's scope (its one follow-up, the misleading `percentile_cont` ledger *reason text*,
  is prose on a row whose `Gap` disposition is unaffected and serves no success criterion — it
  stays out of the table, and the ledger row itself is already registered and probed as criterion 5
  requires). One finding surfaced while reading the code this phase touches, and it is phase 8's
  own work rather than a new row: `emission_ownership::the_printer_branches_on_no_dialect_variant`
  restates a hardcoded three-name list (`DuckDB`/`SparkSQL`/`BigQuery`) and therefore does **not**
  currently catch a `SqlDialect::Trino` branch in the printer — criterion 7's claim is literally
  untrue today. The gate is rewritten to parse the `SqlDialect` enum from `dialect.rs`, the same
  "parsed out of the module, not restated" shape the `RewriteId`/`RestructureId` gates already use,
  so a fifth dialect is covered the day it exists. Second decision taken in the plan: this phase is
  offline — the seams are compile-path gates with no live leg — so an unreachable coordinator is
  irrelevant here, and `verify-phase.sh` runs with the tier unexported per phases 3–7's discipline.

## Blocked
