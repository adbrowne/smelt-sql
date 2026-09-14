# Outcome: Trino takes Spark's state-residency posture — correctness structures unrealisable, every dependent cell recorded as downgraded

**Created:** 2026-09-13
**Status:** active
**Driver:** loop. Docker only, no credential, no human gate. Live-tier phases must emit
`<<PHASE_BLOCKED>>` when the coordinator is unreachable, never skip green.
**Depends on:** `20260913-trino-target-spine` (T1) for the backend and the tier.
Independent of `20260913-trino-emission` (T2), which owns expression spelling; this outcome owns
*bookkeeping* — the half the maintenance-plan invariant explicitly excludes from
`smelt-logical`'s single ownership ("ledger DDL/DML in `smelt-state` excluded as bookkeeping").
**Source:** T3 of the five-outcome Trino programme agreed 2026-09-13; rescoped the same day on
the ruling that **Trino's transaction support is equivalent to Spark's**, and that some
incremental features being unsupported on Trino for now is acceptable. Split from T4 at the seam
the maintenance-plan invariant draws. Pattern followed:
`crates/smelt-state/src/ddl_spark.rs`, whose header records every rule as *measured against a
live server rather than read from documentation* (`scripts/spark-probe-ddl.sh`).
**Spec anchors:** `docs/specs/state.md` §"Which dialects realise which structure" (the table this
outcome extends), §"The residency rule", §"The optionality rule", §"The degradation contract",
§"Declarations stay fail-loud", §Diagnostics; `docs/specs/schema_evolution.md`;
`docs/specs/run_state.md`; `docs/specs/multi_backend.md` §"Incremental & schema evolution per
backend"

## The outcome

Trino's column in `state.md` §"Which dialects realise which structure" reads like Spark (Delta)'s:
**no** for the transactional merge ledger, the reconciliation ledger, observed output deltas, the
fingerprint sidecar and the tombstone ledger. Not "not yet" — a permanent, reasoned absence, for
exactly Spark's reason. Iceberg gives per-table atomic commits and no cross-table transaction, so
a ledger write and its data write cannot be made atomic, and the additive fold's never-fold-twice
refusal has no sound realisation there. The residency rule is therefore satisfied by *declining to
claim* these structures rather than by realising them.

One thing has to be checked rather than inherited. Unlike Spark, Trino has explicit
`START TRANSACTION` / `COMMIT` syntax, so the absence of cross-table atomicity is a property of
the Iceberg connector rather than of the SQL surface, and a connector that did give it would make
"no" the wrong answer. A probe confirms the expected behaviour against the live coordinator and
records the server's own words. The expected answer is Spark's; the probe exists so the claim is
measured, as `ddl_spark.rs` measured every one of its rules.

Everything downstream then follows the degradation contract already specified, and the two
invariants that govern it: **a claim implies a builder** (so Trino claims none of these and no run
reaches an execution path expecting one), and **an absence implies a downgrade, never a refusal**
(so every dependent cell falls to the cheapest recompute-family technique preserving the
equivalence invariant, carries `MaintenanceStateDowngraded`, and is visible in `smelt explain`).
The one exception is a declaration whose semantics *are* a statement about state:
`contract.deferral` promises a ledger-measured lag, so on Trino it fails loudly with
`DeclaredContractRequiresState` rather than silently skipping.

What Trino does get is the other half of `smelt-state`: schema-evolution DDL. Every
`SchemaOperation` maps to a Trino/Iceberg statement or to an honest full refresh, and — exactly as
`ddl_spark.rs` did — **every rule in the table is established by running the form against a live
Iceberg table and recording what the server answered**, because a statement the deployed table
refuses is worse than a migration smelt declines to express. Iceberg is the more capable format
here, so this is where Trino is expected to diverge *upward* from Spark's Parquet column and to
land near or above Delta's.

## Success criteria (checkable)

1. **The posture is confirmed by measurement, then stated.** `scripts/trino-probe-state.sh` (in
   the shape of `scripts/spark-probe-ddl.sh`) runs the atomicity candidates against the live tier
   — a multi-statement `START TRANSACTION` … `COMMIT` spanning two Iceberg tables, the same with
   the second statement failing, and a single-statement write — and prints what Trino answered
   verbatim. The decision log records the verdict and the server's error text. If the probe finds
   genuine cross-table atomicity, that is a **finding that changes this outcome** and is escalated
   in the decision log rather than absorbed; the phases below assume Spark's answer.
2. **`state.md`'s realisability table gains a Trino column reading `no` five times**, with the
   reason stated as Spark's is (per-table atomicity, no cross-table transaction) and marked
   permanent rather than pending. `multi_backend.md` §"Incremental & schema evolution per backend"
   states the same for the Trino target.
3. **A claim implies a builder — vacuously, and provably.** A standing test asserts Trino claims
   no correctness structure and that no execution path on a Trino target can reach a builder
   expecting one. The failure this excludes is the one §"Constraints & Invariants" names: a claimed
   structure with no builder means resolution skips the downgrade, the run reaches the execution
   path, finds nothing, and *refuses* — losing exactly the graceful degradation the contract
   promises.
4. **An absence implies a downgrade, never a refusal.** Every cell that would have used a Trino
   correctness structure resolves to its recompute-family equivalent, carries
   `MaintenanceStateDowngraded`, and is rendered by `smelt explain` (text and `--json`). The
   downgrade is derived **once** by the pure availability resolver in `smelt-logical` — no
   consumer re-derives it at run time — and the *ideal* plan still exists as a derived object even
   though it cannot run, per the resolve-late requirement.
5. **The state-bearing declaration refuses by name.** `contract.deferral` on a Trino target fails
   with `DeclaredContractRequiresState`, because the bounded lag it promises is ledger-measured and
   Trino has no frontier to measure it against. `frozen_horizon` and `retain_departed` follow
   whatever the existing grain and posture rules already say — no new lattice point is defined.
6. **Schema-evolution DDL, measured not read.** Every `SchemaOperation` — add nullable column, add
   column with a `default:`, add `NOT NULL` column, drop column, widen a type, `DROP`/`SET NOT
   NULL`, add/drop a struct field, backfill — maps to a Trino/Iceberg statement or to an honest
   full refresh, with the mapping table in the module header and **each row established by
   executing the form against a live Iceberg table**. Trino's type spellings (`VARCHAR` unbounded,
   `TIMESTAMP(6)`, `DECIMAL(p,s)`, `ROW`/`ARRAY`/`MAP`) are derived from what the server accepted.
   `supports_struct_field_ddl`, `supports_nested_array_ddl`, `supports_alter_column_using`,
   `supports_column_mapping` and `supports_merge_schema_write` from T1's matrix are confirmed or
   corrected here, with the correction pushed back into the spec table in the same commit.
7. **`ddl_trino` is a sibling, split before it sprawls.** Trino's state layer lands as a module
   directory (`crates/smelt-state/src/ddl_trino/…`) the way `ddl_bigquery/` is, not as a single
   file — its peers are 1,883 and 1,708 lines and `.claude/large-file-baseline.txt` is a ratchet.
   Reachable only through the same abstract entry points its peers use; no consumer branches on
   Trino outside the state layer.
8. **The staged relation group works without temporary tables.** Trino has none, so
   `supports_staged_relation_group` — the temp-relation-backed statement group behind the
   merge-less conditional write — is realised over a real relation in a scratch schema, with a
   derived non-colliding name, an owned lifecycle, and proved cleanup: a run interrupted between
   the stage and the apply leaves **no** partially-applied data and no orphan relation a later run
   mistakes for its own. If T1 measured the flag `false`, the consuming path refuses by name
   instead.
9. **Locking and versioning are realised or refused by name.** `run_state.md`'s state locking and
   version checks either work on Trino — demonstrated by two concurrent runs where exactly one
   proceeds — or are refused with a diagnostic naming the backend and the missing capability. A
   lock that silently never locks is the failure this criterion excludes.
10. **`.smelt/` stays non-correctness-bearing.** A test deletes `.smelt/` between runs on the Trino
    target and asserts every maintained table equals what it equalled before — the residency rule's
    own falsifiable form, and the one that matters most on a backend whose correctness structures
    are all absent. Under `state.mode: stateless`, nothing is written under `.smelt/` and no
    maintained table's value changes.
11. **Gates green.** `verify-phase.sh` passes; `.claude/hardening-baseline.txt` and
    `.claude/large-file-baseline.txt` are respected rather than bumped; `state_docs_freshness` and
    the diagnostics catalogue gate stay green; every new `DiagnosticCode` has a fixture under
    `examples/broken/` and an entry in `docs/specs/diagnostics.md`.

## Out of scope

- **Building any correctness-structure realisation for Trino.** The merge ledger, reconciliation
  ledger, observed output deltas, fingerprint sidecar and tombstone ledger are declined, for
  Spark's reason. Revisiting that needs a connector that offers cross-table atomicity, which is a
  research question, not a phase.
- **The incremental features the absence costs.** The additive keyed fold's never-fold-twice
  refusal, the succession patch's window-forward route, `contract.deferral`, and the
  sidecar-dependent key-addressed per-group route are **accepted as unsupported on Trino for now**
  (ruling of 2026-09-13). Each takes its specified downgrade or by-name refusal; none is rebuilt
  on a different mechanism here.
- **A two-phase commit, a compensating transaction, or any mechanism that makes a non-atomic
  ledger look atomic.** Worse than no ledger: the degradation path is correct and false trust is
  not.
- **Weakening the residency rule.** A weaker rule is a spec change needing its own research, never
  a workaround discovered mid-phase.
- **The maintenance statements themselves** — the per-family emitters, `statement_parity`'s legs
  and `maintenance_conformance` on Trino are `20260913-trino-incremental`'s. The seam is the
  invariant's own: bookkeeping here, maintenance statements there.
- **Expression and clause emission** (`20260913-trino-emission`).
- **Reworking another backend's state layer.** A defect this reveals in `ddl_duckdb`, `ddl_spark`
  or `ddl_bigquery` is recorded and handed on. In particular, tightening Spark's column is not
  this outcome's business even though Trino now shares it.
- **Iceberg table maintenance** (`expire_snapshots`, `rewrite_data_files`, snapshot retention) —
  operating concerns, not correctness.
- **State in a second catalog** or any store outside the target backend: the trust boundary forbids
  it being correctness-bearing, so there is nothing to build.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Confirm the posture: `scripts/trino-probe-state.sh` runs the cross-table `START TRANSACTION` candidates (including a failing second statement) against the live tier and prints Trino's answers verbatim; escalate in the decision log if genuine cross-table atomicity is found, since that would change this outcome | done |
| 2 | Spec delta: `state.md`'s realisability table gains a Trino column reading `no` five times with Spark's reason stated as permanent, `multi_backend.md` §"Incremental & schema evolution per backend" states it for the target, and the diagnostics the degradation needs are named | done |
| 3 | Schema-evolution DDL: `ddl_trino/` as a module directory with the measured `SchemaOperation` → Trino/Iceberg mapping table in its header, type spellings derived from what the server accepted, and T1's five schema-related capability cells confirmed or corrected back into the spec table | done |
| 4 | Wire the absence: Trino claims no correctness structure, availability resolution downgrades every dependent cell to its recompute equivalent with `MaintenanceStateDowngraded`, derived once by the pure resolver with the ideal plan still materialised | done |
| 5 | The two invariants as standing tests: no execution path on Trino reaches a builder for an unclaimed structure (claim ⇒ builder), and no absence produces a refusal where the contract specifies a downgrade (absence ⇒ downgrade) | planned |
| 6 | `contract.deferral` refuses on Trino with `DeclaredContractRequiresState`; `frozen_horizon` and `retain_departed` follow the existing grain and posture rules with no new lattice point | pending |
| 7 | The staged relation group without temp tables: scratch-schema relation with a derived non-colliding name, owned lifecycle, proved cleanup after an interruption between stage and apply — or a by-name refusal if T1 measured the flag `false` | pending |
| 8 | Locking and versioning: two concurrent runs where exactly one proceeds, or a refusal naming the backend and the missing capability — never a lock that never locks | pending |
| 9 | `.smelt/` is not correctness-bearing on Trino: delete-between-runs equality, and `state.mode: stateless` writing nothing while changing no maintained table's value | pending |
| 10 | Surface and close: `smelt explain` rendering Trino's downgrades (text + `--json`), diagnostics catalogue and `examples/broken/` fixtures, `docs-site/` state page updated with what Trino costs and why, `verify-phase.sh` green with no baseline bumped | pending |

## Decision log

- **2026-09-14 — phase 5 planning: no reshape; the existing "claim ⇒ builder" gate is a restated
  table, and phase 5 re-derives it from the builders themselves.** Nothing moved in or out of the
  phase table: both invariants already have homes (`smelt-logical`'s
  `tests/maintenance_availability/`, `smelt-runtime`'s `tests/availability_seam/`) and phase 5
  widens them rather than opening new ground. Three facts recorded while planning. (1)
  `realisation.rs::every_claimed_structure_has_a_builder` is two-sided but *both* sides are
  hand-maintained tables — `realisable_state_structures` against a local `has_emitters` that
  restates the same belief; the phase re-keys the right-hand side to calling the real
  `smelt_state::{ledger,observed_delta,tombstone}` entry points, so a builder that exists and a
  claim that does not (or vice versa) is caught by the code rather than by two authors agreeing.
  `FingerprintSidecar` is the one row with no `smelt-state` builder — its realisation gate is
  `BackendCapabilities::supports_fingerprint_sidecar`, which `maintenance_driver/sidecar.rs`
  checks at four sites, so that row keeps a table entry and gains a capability check instead. (2)
  `the_sidecar_claim_matches_the_backend_capability` loops over four `BackendCapabilities`
  constructors and omits `trino_iceberg()`, which has existed since T1 — Trino's sidecar row is
  therefore currently unchecked against the flag the run layer actually gates on. (3) Every
  production call site of a state builder is already confined to
  `smelt-runtime/src/maintenance_driver/`, `execute/project/ledger_reset.rs`,
  `execute/key_addressed.rs`, `smelt-backend-bigquery/src/` and `smelt-state` itself, so the
  reachability half of "claim ⇒ builder" is provable as an allowlist plus the pure
  post-resolution assertion `required_state_structure(cell) == None`, without needing a live
  coordinator.

- **2026-09-14 — phase 4 planning: no reshape; phase 4 removes the refusal, phase 10 keeps the
  rendering.** Two design calls recorded before implementation. (1) **No
  `MaintenanceDialect::Trino` variant is added here.** The enum has ~40 exhaustive arms across
  `smelt-logical/src/maintenance/emit/{fingerprint,succession,probes,partition_bucket,hash,
  merge,bootstrap}.rs`, and every arm is a *statement spelling* — the half
  `20260913-trino-incremental` (T4) owns by the maintenance-plan invariant's own seam, and the
  half this outcome's "Out of scope" explicitly hands over. Phase 4 therefore makes the plan,
  the downgrade and the report independent of the maintenance-statement dialect rather than
  inventing spellings T4 will measure. (2) The seam between phase 4 and phase 10 is
  abort-vs-render: `smelt explain` on a Trino target currently `?`-returns at
  `commands/explain.rs:~556` and produces nothing at all, so phase 4 removes that abort (phase
  10 cannot render a report the command refuses to build); the text/`--json` layout, the
  diagnostics catalogue and the docs-site page stay phase 10's. Three refusal sites and one
  silent fallback found while planning, all inside phase 4: `commands/explain.rs:~556` (aborts
  the command), `smelt-runtime/src/profile.rs:~201` (drops the model into `out.failures`, so
  property-diff reports nothing for a Trino model), `execute/project/dry_run.rs:~244` (silent
  `continue` — a skipped statement indistinguishable from an absent one, a fail-loud
  violation), and `smelt-db`'s `backend_dialect_for("trino")` returning `None`, which reaches
  the *right* availability answer (`vec![]`) only through `unwrap_or_default()` — correct by
  accident, and wrong the moment Trino realises anything. Confirmed already-correct and not
  touched: the pure resolver (`resolve_availability`, `recompute_equivalent`),
  `realisable_state_structures(Trino) == vec![]`, and `maintenance_plan_diagnostics`'
  dialect-generic availability loop.

- **2026-09-14 — phase 4 done: all four named seams fixed; probe-plan and per-cell statement
  preview widened to `Option`/`Result` rather than substituted, never re-derived elsewhere.**
  `backend_dialect_for("trino")` now maps to `SqlDialect::Trino`. `smelt_backend::
  maintenance_dialect`'s `Err` is threaded as a `Result` (not `?`-unwrapped) through
  `commands/explain.rs`, `smelt-runtime::profile.rs`, and `smelt-ui::build.rs`'s diagnostics
  endpoint — none of the three abort or drop the model into `failures` now.
  `smelt_runtime::diagnostics::build_model_diagnostics` and `probe_plan::probe_plan_for_model`
  take `Result<MaintenanceDialect, UnsupportedMaintenanceDialect>` / `Option<MaintenanceDialect>`
  respectively; a new `unavailable_plan_cell_diagnostics` helper (`diagnostics/preview.rs`)
  renders every technique-preview entry `NotApplicable` naming the missing dialect verbatim
  rather than skipping cells or substituting DuckDB's spelling — `build_admitted_statement_group`
  was re-keyed from scanning for `Admissibility::Admitted` to matching `admitted_technique`
  directly so it still finds the (now-`NotApplicable`) entry and surfaces its named reason
  instead of a generic "no Admitted entry" message. `execute/project/dry_run.rs`'s silent
  `continue` now calls `reporter.maintenance_warning` — which `CliReporter` did not previously
  override at all (a pre-existing gap: retention-downgrade warnings from `execute/project/mod.rs`
  were already silently swallowed by the terminal reporter), so this phase added the first
  `eprintln!`-backed override, incidentally making those warnings visible for the first time too.
  Three ratchets bumped with sign-off notes: `.claude/large-file-baseline.txt` (three files grew
  7–14 lines from the `Result`/`Option` threading and one new unit test, all in-place, no new
  abstraction) and `.claude/hardening-baseline.txt` (`smelt-cli println` 189→190, the
  `eprintln!`-matches-`println!`-substring quirk noted 2026-09-08). `docs/specs/multi_backend.md`
  §"Parity contract"'s Trino paragraph now states the settled posture instead of the phase-4
  forward-pointer. `verify-phase.sh`, the plan's five named test targets, and
  `smelt-lsp --test example_workspaces` are all green.

- **2026-09-14 — phase 3 done: `ddl_trino` measured and wired; struct-field drop and nested
  widening are DDL on Trino, unlike Spark/BigQuery.** All five T1 capability cells confirmed,
  no corrections. Discovered a live coordinator quirk (`ALTER COLUMN "c" DROP NOT NULL` with a
  quoted column name fails to resolve the column on `trinodb/trino:483`; every other ALTER
  COLUMN form accepts quoting) — worked around by emitting that one statement's column name
  unquoted, documented in the generator, spec, and module header. `RewriteColumn` and a
  struct-field `default:` are always refused (no safe in-place form). See
  `phases/03-summary.md`.
- **2026-09-14 — phase 3 planning: no reshape; probe and generator stay one phase.** Considered
  splitting phase 3 into a probe row and a generator row (the phase-1/phase-2 rhythm), and
  rejected it: renumbering rows 4-10 would break the decision-log references to "phase 4", and
  criterion 6 deliberately binds the measurement to the mapping table it produces — a probe whose
  output nothing consumes for a phase is the "read from documentation" failure with an extra step.
  Two facts recorded while planning. (1) T1 already measured the five schema-related capability
  cells against a live coordinator (`crates/smelt-backend-trino/tests/capability_probes.rs`, each
  with the server's words in `trino_iceberg()`), so criterion 6's second half is mostly a
  *confirmation* at `SchemaOperation` granularity rather than a fresh measurement — the new probe
  covers the forms those cells do not: add `NOT NULL` column, add with `default:`, `SET`/`DROP NOT
  NULL`, drop struct field, backfill, and the type spellings. (2) `ddl_backend_for_dialect`
  currently returns `UnsupportedDdlDialect` for `SqlDialect::Trino` with a doc comment naming Trino
  as the live case; phase 3 inverts that and rewords the comment, keeping the `Result` signature so
  callers are untouched. The live tier was down at planning time (`curl :18080/v1/info` → no
  connection), so task 2 brings it up first and blocks rather than skipping green if it cannot.

- **2026-09-14 — phase 2 result: spec text lands; one pre-existing gate needed the same tokens
  kept.** `state.md`'s realisability table gained the `Trino (Iceberg)` column (five `**no**`
  cells) plus a paragraph citing the measured autocommit refusal
  (`phases/01-summary.md`) and stating the absence as permanent. `multi_backend.md` gained a
  Trino paragraph under §"Incremental & schema evolution per backend" and the §"Parity contract"
  sentence describing Trino maintenance as "not yet reachable" was corrected to the
  permanent-absence-plus-downgrade framing. One wrinkle: `crates/smelt-cli/tests/
  trino_emission_spec_freshness.rs::parity_contract_states_trino_scope` (pre-existing, from
  `20260913-trino-emission`) asserts §"Parity contract" still contains the literal tokens
  `maintenance_dialect` and `SqlDialect::Trino` — the first reword dropped both and went red, so
  the corrected paragraph keeps both tokens while adding the permanence/downgrade language and a
  forward pointer to phase 4's fix. `docs/specs/state.md` §Diagnostics also gained one sentence
  naming which of the two existing diagnostic codes applies on a no-structure backend (Spark,
  Trino) — no new diagnostic code. New gate: `crates/smelt-logical/tests/
  state_realisability_docs.rs`, parsing both spec tables against `realisable_state_structures`
  and the corrected prose so the two cannot drift again. Doc comments in `state_structure.rs` and
  the `realisation.rs` test's `has_emitters` table, which both said `20260913-trino-ledger`
  "revisits" the absence, were reworded to record it as settled by measurement — no behaviour
  change. `cargo test -p smelt-logical --test state_realisability_docs`,
  `--test maintenance_availability`, `cargo test -p smelt-core --test trino_docs_freshness`,
  `cargo test -p smelt-cli --test trino_emission_spec_freshness`, and
  `bash .claude/scripts/verify-phase.sh` are all green; no baseline file touched.
  **For phase 4:** `maintenance_dialect` still returns `Err` for `SqlDialect::Trino` — the spec
  now describes the downgrade it should perform instead, so phase 4's job is to make the code
  match, not to decide the shape.

- **2026-09-14 — phase 2 planning: no reshape; one observation handed to phase 4.** The spec
  delta is text plus a new drift gate, so nothing moves in or out of the phase table. Recorded for
  phase 4's planner: the code already claims the right thing —
  `realisable_state_structures(SqlDialect::Trino)` is `vec![]` and
  `maintenance_availability/realisation.rs`'s two-sided `has_emitters` gate already covers Trino —
  but `maintenance_dialect` currently returns `Err` for `SqlDialect::Trino`
  (`docs/specs/multi_backend.md` line ~171), which is a **refusal where the degradation contract
  specifies a downgrade**. That is exactly the failure criterion 4 excludes and criterion 5's
  second invariant test asserts against; phase 4 owns removing it, and phase 2 only corrects the
  spec wording that describes it as "not yet reachable".

- **2026-09-14 — phase 1 result: measured, not assumed — Iceberg refuses ALL writes inside an
  explicit transaction, not just cross-table ones. Criteria 2–5's Spark-shaped assumption holds;
  no escalation.** `scripts/trino-probe-state.sh` run against the live tier (`trinodb/trino:483`,
  `apache/iceberg-rest-fixture:1.10.1`), full output in `phases/01-summary.md`. Two protocol facts
  had to be discovered before the atomicity question was even askable, both now baked into the
  script: (1) Trino refuses `START TRANSACTION` outright with `Client does not support
  transactions` unless **every** request, including the first transaction-less one, carries
  `X-Trino-Transaction-Id: NONE` — a client that never sends the header at all is assumed
  incapable of reading back `X-Trino-Started-Transaction-Id` and threading it forward, so the
  server won't open a transaction it can never learn was committed; (2) the target schema does not
  pre-exist and must be created (`CREATE SCHEMA IF NOT EXISTS iceberg.smelt_dev`) before any
  case — smelt's own backend does this itself (`crates/smelt-backend-trino/src/backend.rs:315`).
  With both fixed, every case's verdict:
  - **A (baseline, autocommit)** — ACCEPTED. `CREATE TABLE` + `INSERT` outside any transaction
    land normally; row count 1.
  - **B (bare `START TRANSACTION`/`COMMIT`)** — both ACCEPTED, confirming the SQL syntax itself
    parses and a transaction can be opened and closed — the refusal below is connector-level, not
    a grammar rejection.
  - **C (cross-table happy path)** — `START TRANSACTION` ACCEPTED; the first `INSERT` REFUSED
    verbatim `Catalog only supports writes using autocommit: iceberg`; the second `INSERT` and the
    `COMMIT` REFUSED because the transaction was already aborted by the first failure. Both
    tables' row counts: 0.
  - **D (cross-table, second statement fails)** — identical shape to C: the *first* write already
    refuses, so the type-mismatch second statement never even runs. t1's row count: 0 — it never
    landed, because it was never accepted in the first place.
  - **E (explicit rollback)** — `START TRANSACTION` ACCEPTED, `INSERT` REFUSED (same autocommit
    message), `ROLLBACK` ACCEPTED (rolling back nothing). t1 row count: 0.
  - **F (DDL in a transaction — the cell T1 measured through the wrong client)** — `START
    TRANSACTION` ACCEPTED, `CREATE TABLE` REFUSED with the *same* autocommit message (DDL is a
    write too, not a special case), `ROLLBACK` ACCEPTED, table does not exist afterward. This
    directly answers the hand-forward's flagged cell: `supports_transactional_ddl = false` is
    confirmed correct even measured through a client that *does* hold a real session — T1's
    verdict was right, for a reason one layer deeper than "the client had no session": the
    connector itself refuses DDL as a transactional write, full stop.
  - **G (same-table, two writes)** — same autocommit refusal on the *first* `INSERT`. This settles
    the question the case was designed to separate: the connector's restriction is not "no
    *cross*-table transaction" but "no multi-statement transactional write at all, same table or
    not." Iceberg's per-table atomic commit model has no place to hang a second pending write
    while a first is uncommitted, so the connector refuses the second write's *precondition*
    (being inside an open transaction) rather than attempting and rolling back a torn commit.
  **Verdict:** Spark's shape is confirmed and, if anything, understated — Trino/Iceberg has no
  transactional write capability whatsoever (not even single-table), only autocommit writes and a
  syntactically-real but write-inert `START TRANSACTION`/`COMMIT`/`ROLLBACK` pair (useful only for
  read-only statements, e.g. consistent multi-query snapshots — not probed here, out of scope for
  this outcome). Criteria 2–5's assumption — Trino claims none of the five correctness structures
  for exactly Spark's reason — holds and is now measured rather than inherited. No genuine
  cross-table atomicity was found; nothing here escalates or changes the outcome's scope.

- **2026-09-14 — phase 1 planning: the probe must carry its own transaction session.** T1's
  `supports_transactional_ddl = false` measured smelt's stateless `/v1/statement` client, not the
  Iceberg connector, so a probe built on smelt's backend client would re-measure the client and
  say nothing about cross-table atomicity. `scripts/trino-probe-state.sh` therefore speaks the
  statement protocol directly, threading `X-Trino-Started-Transaction-Id` back as
  `X-Trino-Transaction-Id` (CLI-in-container as fallback), and asserts observed row counts after
  each case rather than trusting a statement's own success. The phase table is unchanged: nothing
  in the hand-forward moved work in or out.

- **2026-09-14 — hand-forward from `20260913-trino-target-spine` phase 11.** Measured, for this
  outcome to act on: `supports_transactional_ddl = false` is smelt's stateless `/v1/statement`
  HTTP client having no session continuity across `START TRANSACTION`/DDL/`ROLLBACK` — client
  design, not a Trino grammar rejection (`docs/specs/multi_backend.md` §Known Divergences); the
  staged-relation-group pattern (phase 7 above) has no temp-table primitive to build it from —
  Trino/Iceberg has no session-scoped temp tables; `null_safe_equality` is `IS NOT DISTINCT FROM`
  (matching DuckDB/BigQuery), not Spark's `<=>`; and the Delta-shaped residency prior this
  outcome's scaffold decision assumed held for schema-evolution DDL on most cells but not all —
  see T1's measured seven-flag divergence list in `docs/specs/multi_backend.md` §"Capability
  matrix" before assuming Spark(Delta) parity on an unconfirmed cell.

- 2026-09-13 (scaffold, before phase 1): **Trino's transaction support is taken as equivalent to
  Spark's, and some incremental features are accepted as unsupported on Trino for now.** Ruling by
  the programme owner. `state.md` §"Which dialects realise which structure" already records
  Spark's absence as permanent and reasoned — "Delta provides per-table atomicity and no
  cross-table transaction, so a ledger write and its data write cannot be made atomic, and the
  additive fold's never-fold-twice refusal has no sound realisation there" — and Iceberg has the
  same shape. So this outcome starts from Spark's column rather than treating residency as an open
  binary, which is why phase 1 is a confirmation probe rather than an investigation. The one fact
  not inheritable is that Trino, unlike Spark, has `START TRANSACTION`/`COMMIT` syntax: the
  absence of cross-table atomicity is a property of the Iceberg connector, not of the SQL surface,
  so it is measured rather than assumed, and a probe finding otherwise escalates instead of being
  absorbed.

## Blocked
