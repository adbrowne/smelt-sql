# Plan: `Unknown` reason-discriminant + `ColumnTypeUnresolved` live (unblocks W3 D-07)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — a follow-up to the **W3 — D-diag** wave (`docs/plans/20260613-w3-diagnostics.md`). W3 landed P1–P4 but **blocked P5 + P6** on a missing prerequisite: `DataType::Unknown` is an undiscriminated unit variant, so the code cannot distinguish a *compiler-resolvable* `Unknown` from a *genuinely-dynamic* one and `ColumnTypeUnresolved` would over-fire (W3 §"Blocked phases", 2026-06-14). The block's recommended unblock was **Option B** — *"a dedicated `D-types-unknown-reason` sub-plan that adds the discriminant and then unblocks P5/P6."* This is that sub-plan.

**Date**: 2026-06-20
**Spec** (correctness oracle — already landed, no spec edit except the P4 close-out KD retraction):
- `docs/specs/types.md` — owns the `Unknown` reason-discriminant. §"Strict-by-default doctrine": the closed three-way reason `Unresolved` / `Dynamic` / `Propagated` (table at types.md:132), the no-silent-`Unknown` invariant (types.md:128, :390), and the design rationale + rejected alternatives (types.md:369). The set is closed — adding a reason requires a spec edit (types.md:390).
- `docs/specs/function_schema_inference.md` — owns the `ColumnTypeUnresolved` schema-propagation rules: it fires at the projection producing an `Unknown` column whose reason is `Unresolved`; `Propagated` and `Dynamic` columns are diagnostic-free by construction (lines 32, 42, 68, 78, 85). The emission-gap Known Divergence (line 90) is retracted/refreshed at close-out.
- `docs/specs/diagnostics.md:64` — the `ColumnTypeUnresolved` catalogue row (Error).
**Spec diff**: none — code-catching-up-to-spec. The discriminant and `ColumnTypeUnresolved` were fully specified in the 2026-06-12/13 review (`docs/research/20260613-spec-remediation-decisions.md` D-07 = B, "mint live"). This sub-plan brings the **implementation** into line.
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only, except the P4 close-out retraction of the now-satisfied `types.md` / `function_schema_inference.md` Known-Divergence notes (a timeless spec edit, mirroring W3 P6).

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then the spec sections above — they are the correctness oracle; do not re-open the settled decisions (D-07 = B; reason set is the closed `Unresolved`/`Dynamic`/`Propagated`). Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked`) per the per-phase routine below. The standing gates every phase must keep green: the **unknown-census guard** `cargo test -p smelt-types --test unknown_census` (every `DataType::Unknown` construction site classified), the **catalogue coverage gate** `cargo test -p smelt-db --test diagnostics_catalogue` (every `DiagnosticCode` variant appears in `diagnostics.md`), and the dual example gates `cargo test -p smelt-cli --test example_diagnostics` + `cargo test -p smelt-lsp --test example_workspaces`. If that was the last `pending` phase, flip this sub-plan's Status to `done (<today>)` in the master registry and commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record + continue), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>`, or `<<ALL_DONE>>`.

## Goal

Make the `Unknown` reason-discriminant real in the type system, then mint `ColumnTypeUnresolved` live — closing the W3 D-07 block:

1. **Discriminant (the keystone).** Turn `DataType::Unknown` (unit variant) into `DataType::Unknown(UnknownReason)` with the closed set `Unresolved` / `Dynamic` / `Propagated`. Type *identity* stays reason-agnostic (two `Unknown`s are equal regardless of reason — the reason is diagnostic metadata, not part of the lattice bottom's identity), so LUB/compatibility/dedup are unaffected. The reason is readable (`unknown_reason()`) at the schema/diagnostic layer.
2. **Census-as-reason-map.** Every `DataType::Unknown` construction site already lives in `.claude/unknown-census.toml` (98 sites: 3 `error`, 95 `legitimate`). Extend the census + its guard to also record the **reason** at each site; the 3 `error` sites become `Unresolved`, the conservatism sites become `Dynamic`/`Propagated`. This makes the migration reviewable and prevents silent reason-drift.
3. **`ColumnTypeUnresolved` live.** Mint `DiagnosticCode::ColumnTypeUnresolved` and wire emission at the schema-layer projection that produces an `Unknown` column whose reason is `Unresolved` (origin-only; `Propagated`/`Dynamic` stay silent). This delivers the work blocked as **W3 P5**.
4. **Close out W3.** Retract/refresh the `types.md` and `function_schema_inference.md` Known-Divergence notes the discriminant now satisfies; flip W3 P5/P6 to `done` (delivered here) and the master registry. This delivers **W3 P6**.

## Design decisions (resolved — do not re-litigate)

- **D-07 = B (mint live).** From `docs/research/20260613-spec-remediation-decisions.md`. `ColumnTypeUnresolved` is a live catalogued code; the rule that emits it is normative. It fires for a **compiler-resolvable** `Unknown` (reason `Unresolved`), never for a genuinely-dynamic value (reason `Dynamic`) or a propagated one (reason `Propagated`).
- **Representation = payload on the variant: `DataType::Unknown(UnknownReason)`.** The spec attaches the reason to the `Unknown` *value*, not to a column side-table: `types.md:104` and `:307` degrade *expression* results (`NonPortableCollation`, `DecimalPrecisionOverflow`) to `Unknown` with reason `Unresolved`, and those are not column-schema entries. A "reason on `TypedColumn`" representation could not carry an expression-level reason, so it is rejected. The reason must ride on `DataType` itself.
- **Type identity is reason-agnostic.** `Unknown(Unresolved) == Unknown(Dynamic)`, and both hash identically. Rationale (types.md:369): the reason is *why* the inference gave up, not *what* the type is; the lattice bottom must compare/hash as one value or LUB, set-dedup, and schema-equality break. **Mechanism:** give `UnknownReason` a custom `PartialEq`/`Eq`/`Hash` whose `eq` is always `true` and whose `hash` is a no-op; `DataType` then keeps its existing `#[derive(PartialEq, Eq, Hash)]` and `Unknown(a) == Unknown(b)` holds for free. *Rejected alternative:* hand-writing `DataType`'s entire `PartialEq`/`Hash` over ~20 variants — verbose and error-prone for a one-variant carve-out.
- **Closed reason set.** `Unresolved` / `Dynamic` / `Propagated` only (types.md:390). Adding a reason is a spec edit; this plan does not add a fourth.
- **`ColumnTypeUnresolved` may have no user-reachable trigger today — that is expected, not a block.** `function_schema_inference.md:90` records that all *function-derived* inference gaps now resolve, so a well-formed signature does not currently contribute an `Unknown` column; the live `Unresolved`-producers (cross-family arithmetic, decimal overflow, non-portable collation — the 3 census `error` sites) already emit their own origin diagnostic (`TypeMismatch` / `DecimalPrecisionOverflow` / `NonPortableCollation`) at the operator, which makes the *column* reading that result `Propagated` (silent, no double-report). The deliverable is therefore the **wired rule** matching the normative spec — proven by a white-box unit test that synthesizes an origin `Unknown(Unresolved)` column — **not** a new diagnostic firing on the example workspaces (which must stay clean). The remaining silent-`Unknown` sources the spec still lists (generator-emitted schemas, `smelt.columns_of` reflection, meta-language HOF values in column position — types.md:419) are **out of scope**: they are meta-language-owned and tracked in `docs/plans/20260519-functions-meta-gaps.md`.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. Red on this phase's own target → proceed; unrelated red → block.
2. **Red-green `/smelt:implement`.** Failing test(s) first, then implementation, spec as oracle. Implementer then reviewer.
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; the **unknown-census guard** `cargo test -p smelt-types --test unknown_census`; the **catalogue gate** `cargo test -p smelt-db --test diagnostics_catalogue`; the dual gate `cargo test -p smelt-cli --test example_diagnostics` + `cargo test -p smelt-lsp --test example_workspaces`.
4. **Record + commit.** Row `done` + date; commit + push tests + impl + table with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or roll-up on the last phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)
Set the row `blocked` + one-line reason; append a dated §"Blocked phases" entry; restore a clean committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:
- A construction site whose correct reason is genuinely ambiguous from the census classification and the surrounding code (block rather than guess `Unresolved` and over-fire).
- The reason-agnostic identity carve-out interacts badly with an unforeseen `DataType` equality/hash consumer that the test suite reveals and that cannot be reconciled without a spec question.
- Pre-flight red on unrelated breakage; tree can't return to green.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | `UnknownReason` enum + `DataType::Unknown(UnknownReason)` w/ reason-agnostic identity; behavior-preserving workspace migration | done | discriminant | feat(types): discriminate DataType::Unknown by reason (Unresolved/Dynamic/Propagated), reason-agnostic identity | 2026-06-20 |
| P2 | Unknown-census guard records the reason per site | pending | census map | feat(types): unknown-census records each Unknown site's reason | |
| P3 | `DiagnosticCode::ColumnTypeUnresolved` minted + wired at schema-layer origin `Unknown(Unresolved)` columns | pending | D-07 (W3 P5) | feat(db): mint and emit ColumnTypeUnresolved for origin Unresolved columns (D-07) | |
| P4 | Close-out: KD retraction (types.md, function_schema_inference.md) + W3 P5/P6 → done + master registry + ROADMAP | pending | W3 P6 | docs(spec-impl): close out D-07 — Unknown discriminant + ColumnTypeUnresolved landed | |

**Status values**: `pending`, `done`, `blocked`.

---

### Phase P1: `UnknownReason` + payload on `DataType::Unknown`, reason-agnostic identity

**Goal.** `DataType::Unknown` carries a closed reason (`Unresolved`/`Dynamic`/`Propagated`); type identity ignores the reason; the whole workspace compiles and is **behavior-preserving** (no new diagnostics — `ColumnTypeUnresolved` is not wired until P3).

**Pre-conditions.** None (the keystone). This is an atomic big-bang variant change: the enum signature changes, so all construction/match sites migrate in one commit.

**TDD tests to write first** (`crates/smelt-types/src/lib.rs::tests` / `crates/smelt-types/tests/`):
- `unknown_identity_is_reason_agnostic` — `DataType::Unknown(Unresolved) == DataType::Unknown(Dynamic)`, and both produce the same hash (insert into a `HashSet`, assert len 1).
- `unknown_reason_is_readable` — `DataType::Unknown(Unresolved).unknown_reason() == Some(UnknownReason::Unresolved)`; a non-Unknown type returns `None`.
- `lub_and_dedup_unaffected_by_reason` — an LUB / set-dedup over a column list containing two differently-reasoned `Unknown`s behaves exactly as with the old unit variant (collapses to one).
- `is_unknown_matches_any_reason` — `DataType::Unknown(Dynamic).is_unknown()` is `true`; `DataType::Integer.is_unknown()` is `false`.

**Implementation shape.**
- Add `pub enum UnknownReason { Unresolved, Dynamic, Propagated }` (`#[derive(Debug, Clone, Copy)]`) with custom `PartialEq`/`Eq`/`Hash` that treat all reasons as equal (always-`true` `eq`, no-op `hash`) — so `DataType` keeps `#[derive(PartialEq, Eq, Hash)]` and `Unknown(a) == Unknown(b)`.
- Change `DataType::Unknown` → `DataType::Unknown(UnknownReason)` (`lib.rs:88`).
- Add helpers on `DataType`: `is_unknown(&self) -> bool` (`matches!(self, DataType::Unknown(_))`), `unknown_reason(&self) -> Option<UnknownReason>`, and constructors `unknown_unresolved()` / `unknown_dynamic()` / `unknown_propagated()` to keep call-sites legible.
- **Migration (default = behavior-preserving):** the 3 census `error` sites (`binary.rs:404`, `dispatch.rs:732`, `collation.rs:90`) → `Unresolved`; **every other** site → `Dynamic` unless the code obviously produces the `Unknown` *because an input was already `Unknown`* (those → `Propagated`). Defaulting unclear conservatism to `Dynamic` guarantees no new diagnostic can fire from the migration alone. Update the ~6 production match/eq sites (`== DataType::Unknown`, `DataType::Unknown =>`) to `is_unknown()` / `DataType::Unknown(_)`.

**Critical files.** `crates/smelt-types/src/lib.rs` (enum + helpers + custom impls); every crate with a construction site (smelt-db, smelt-core, smelt-planner, smelt-runtime, smelt-cli — see `.claude/unknown-census.toml` for the exhaustive list).

**Review checklist:** identity/hash reason-agnostic (LUB/dedup unaffected); reason readable; reason set closed (no fourth variant); migration behavior-preserving (census `error`→`Unresolved`, conservatism→`Dynamic`, input-driven→`Propagated`); no `== DataType::Unknown` left; census guard green; dual example gates green (no new diagnostics).

**Commit.** `feat(types): discriminate DataType::Unknown by reason (Unresolved/Dynamic/Propagated), reason-agnostic identity`

---

### Phase P2: Unknown-census records the reason per site

**Goal.** The census is the authoritative reason-map: each `DataType::Unknown` site declares its reason, and the guard fails if a site is missing a reason or constructs a reason inconsistent with its classification.

**Pre-conditions.** P1 done.

**TDD tests to write first** (`crates/smelt-types/tests/unknown_census.rs`):
- `every_site_declares_a_reason` — every entry in `.claude/unknown-census.toml` has a `reason` field that is one of `unresolved`/`dynamic`/`propagated`; a site without one fails.
- `error_classification_implies_unresolved` — any `classification = "error"` site must declare reason `unresolved` (an `error` site is by definition a compiler-resolvable gap).
- `new_unclassified_site_fails` (extend the existing guard) — a construction site absent from the census still fails (unchanged), and now also fails if present-but-reasonless.

**Implementation shape.** Extend the census TOML schema with `reason = "unresolved" | "dynamic" | "propagated"` per entry; populate all 98 entries to match what P1 chose. Extend the guard's TOML parser (`load_allowlist`) + assertions to require and validate the reason. Keep `.claude/scripts/unknown-census.sh` emitting the construction-site list (no reason inference needed — the reason is human-classified in the TOML).

**Critical files.** `.claude/unknown-census.toml`, `crates/smelt-types/tests/unknown_census.rs`.

**Review checklist:** every site has a valid reason; `error`→`unresolved` enforced; guard rejects reasonless/unknown-reason entries; reasons match P1's migration.

**Commit.** `feat(types): unknown-census records each Unknown site's reason`

---

### Phase P3: `ColumnTypeUnresolved` minted and wired (delivers W3 P5)

**Goal.** A schema-layer projection that produces an `Unknown` column whose reason is `Unresolved` — and whose origin is that projection (not propagated, not dynamic) — emits `ColumnTypeUnresolved` (Error). `Propagated`/`Dynamic` columns stay silent. Example workspaces stay clean.

**Pre-conditions.** P1 + P2 done. **Risk-flagged (inherited from W3 P5):** do not over-fire on `Dynamic`/`Propagated`.

**TDD tests to write first** (`crates/smelt-db/...`):
- `origin_unresolved_column_emits_column_type_unresolved` — a white-box test that drives a schema-layer column resolving to `Unknown(Unresolved)` at its origin projection → exactly one `ColumnTypeUnresolved` at the projection span, with a message naming the column and the unresolved source.
- `propagated_unknown_column_is_silent` — a column that is `Unknown(Propagated)` (its input was already `Unknown`) emits **no** `ColumnTypeUnresolved` (origin-only, no cascade).
- `dynamic_unknown_column_is_silent` — a column that is `Unknown(Dynamic)` (legitimately dynamic) emits **no** `ColumnTypeUnresolved`; existing `CannotInferType` cases unchanged.
- Catalogue + dual gates green; example workspaces produce no new diagnostics.

**Implementation shape.** Add `DiagnosticCode::ColumnTypeUnresolved` (the variant does **not** exist yet) wherever `DiagnosticCode` is defined in `smelt-db`; the `diagnostics.md:64` catalogue row already exists, so the catalogue gate stays green. In the schema/type-check layer (W3 P5 pointed at `check_types.rs:110-150` with the projection anchor from the struct-spread/projection site), inspect a resolved column's `unknown_reason()`: emit `ColumnTypeUnresolved` only for `Unresolved`; leave `Propagated`/`Dynamic` silent and preserve existing `CannotInferType` semantics. Anchor at the projection (SELECT item / FROM entry) that produced the column.

**Critical files.** `smelt-db` `DiagnosticCode` definition, `crates/smelt-db/src/queries/check_types.rs`, `crates/smelt-db/src/queries/schema.rs` (projection anchor).

**Review checklist:** fires only for origin `Unresolved`; silent for `Propagated`/`Dynamic`; anchored at the projection; `CannotInferType` preserved; example workspaces clean; catalogue + census gates green.

**Commit.** `feat(db): mint and emit ColumnTypeUnresolved for origin Unresolved columns (D-07)`

---

### Phase P4: Close-out (delivers W3 P6)

**Goal.** Retract the Known-Divergence notes the discriminant now satisfies, mark the W3 block resolved, and roll up.

**Pre-conditions.** P1–P3 done.

**TDD tests to write first:** none new — runs the gates.

**Implementation shape.**
- **KD retraction (timeless edits):** `docs/specs/types.md:419` — drop "the `Unknown` value is currently undiscriminated in places" (it is now discriminated); keep only the genuinely-remaining meta-language silent-`Unknown` sources, repointed at `docs/plans/20260519-functions-meta-gaps.md`. `docs/specs/function_schema_inference.md:90` — refresh the emission-gap note to: the rule is now wired; no in-scope user input currently reaches it because all function-derived gaps resolve (state it as behavior, no phase vocabulary).
- **Unblock W3:** in `docs/plans/20260613-w3-diagnostics.md`, flip P5 and P6 from `blocked` to `done` with a note "delivered via `docs/plans/20260620-unknown-reason-discriminant.md`"; the §"Blocked phases" entries stay (append-only history).
- **Master registry:** flip the W3 row **and** this sub-plan's row in `docs/plans/20260613-spec-impl.md` to `done (2026-06-20)`.
- `docs/ROADMAP.md` line.

**Critical files.** `docs/specs/types.md`, `docs/specs/function_schema_inference.md`, `docs/plans/20260613-w3-diagnostics.md`, `docs/plans/20260613-spec-impl.md`, `docs/ROADMAP.md`.

**Review checklist:** KD retractions genuinely satisfied + timeless (no phase vocabulary); W3 P5/P6 `done` with pointer; both registry rows `done`; ROADMAP updated; catalogue + census + dual gates green.

**Commit.** `docs(spec-impl): close out D-07 — Unknown discriminant + ColumnTypeUnresolved landed`

---

## Deferred during implementation

(Append-only.)

- The remaining silent-`Unknown` sources the no-silent-`Unknown` doctrine still wants closed — generator-emitted model schemas, `smelt.columns_of` reflection, and meta-language HOF values in SQL column position (`types.md:419`) — are **out of scope** here. They are meta-language-owned and tracked in `docs/plans/20260519-functions-meta-gaps.md`. This plan delivers the discriminant they depend on.

## Blocked phases

Append-only log. None yet.

## Verification

- `cargo test -p smelt-types --test unknown_census`, `cargo test -p smelt-db --test diagnostics_catalogue`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test example_workspaces` green.
- Manual smoke: two differently-reasoned `Unknown`s are type-equal (LUB/dedup unaffected); a synthesized origin `Unknown(Unresolved)` column errors `ColumnTypeUnresolved`; a propagated/dynamic `Unknown` column is silent; the example workspaces produce no new diagnostics.
- `/smelt:validate types`, `/smelt:validate function_schema_inference`, `/smelt:validate diagnostics` report no behavioural drift on these surfaces.
