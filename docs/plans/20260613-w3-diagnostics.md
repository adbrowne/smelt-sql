# Plan: W3 — Diagnostics codes, ownership & severities (D-diag)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — the third wave of the spec-remediation implementation backlog. Remediates the **D-diag** cluster of the 2026-06-13 spec review: **D-07** (mint `ColumnTypeUnresolved` live — *risk-flagged*: you chose B, declare-it-firing-now), **D-14** (drop the malformed-frontmatter trigger from `BackendsWideningNotAllowed`), **D-19** (`HofNamedArgument`), **D-30** (function name-uniqueness directory-scoped), **D-31** (unknown frontmatter key → Error), and the **D-08/D-09** cleanup (no `UnknownSmeltPath` code; bare unresolved `smelt.<path>` → `UndefinedModelRef`; no `smelt.source()` call-form). Depends on W1 (addressing) for D-30. The autonomy loop works this sub-plan phase by phase.

**Date**: 2026-06-13
**Spec**: `docs/specs/diagnostics.md` (the Models/core table rows for `ColumnTypeUnresolved`, `UndefinedModelRef`/`UndefinedSource` tie-break; the Functions rows for `DuplicateFunctionDefinition` directory-scoping, `BackendsWideningNotAllowed`, `FrontmatterParseError`; the meta-language `HofNamedArgument` row); `docs/specs/functions.md` Constraint 4 (directory-scoped uniqueness) + Constraint 6 (unknown-key → Error); `docs/specs/function_schema_inference.md` + `docs/specs/types.md` (`ColumnTypeUnresolved` trigger/anchor + the `Unknown` reason-discriminant); `docs/specs/meta_language.md` §HOF (named-argument rule).
**Spec diff**: `e862ebec..HEAD` — **already landed**. Code-catching-up-to-spec; no spec edits except the P6 close-out retraction of any now-satisfied Known-Divergence note.
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only. Close-out updates the master registry + `docs/ROADMAP.md`.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then the spec sections above — they are the correctness oracle; do not re-open the settled decisions. Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked`) per the per-phase routine below. The **catalogue coverage gate** `cargo test -p smelt-db --test diagnostics_catalogue` (every `DiagnosticCode` variant must appear in `diagnostics.md`, and the spec rows already exist) is a verification gate every phase that adds a variant must keep green. If that was the last `pending` phase, flip this sub-plan's Status to `done (<today>)` in the master registry and commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record + continue), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>`, or `<<ALL_DONE>>`.

## Goal

Bring the diagnostics surface into line with the reviewed catalogue and ownership rules:

- **D-19 (major).** A HOF call (`map`/`filter`/`reduce`) passing arguments by name currently has no dedicated diagnostic, and a named arg that *is* a lambda produces **no** error (the `HofExpects*` codes only cover wrong-kind). Add `HofNamedArgument` (Error), firing **before** the kind check — mirroring the existing `ReducerNamedArgument` (`crates/smelt-db/src/type_inference/hof.rs:303-366`).
- **D-14 (minor).** `BackendsWideningNotAllowed` fires on backend-widening **or** malformed frontmatter (`function_diagnostics.rs:550,573`). Remove the malformed-frontmatter branch; route malformation to `FrontmatterParseError`. A code named for backend widening should not fire on unrelated malformation.
- **D-31 (major).** The doctrine is fail-loud: an unknown frontmatter key is a typo (`deterministc:`) to surface as an **Error**. Unknown-key is **already** Error (`crates/smelt-core/src/frontmatter.rs:9-10,167`); the spec now says `FrontmatterParseError` is **Error in all cases** — confirm and remove any residual `FrontmatterParseError`-as-Warning path (`function_diagnostics.rs:845-861`).
- **D-30 (major).** `DuplicateFunctionDefinition` is currently **workspace-wide** (`function_diagnostics.rs:37-100`, one `HashMap<name, path>`). The reviewed rule (functions.md Constraint 4 + architecture.md addressing) is **directory-scoped**: two `smelt.define`s in the same directory sharing a name collide; a define clashing with a built-in routes to `ExternCollidesWithBuiltin` (already exists, `function_diagnostics.rs:58-73`). Encodes the W1 addressing model → depends on W1.
- **D-08/D-09 (cleanup).** `UnknownSmeltPath` has **zero** code references (nothing to remove). Bare unresolved `smelt.<path>` already routes `smelt.sources.*` → `UndefinedSource`, else → `UndefinedModelRef` (`crates/smelt-db/src/lib.rs:1746-1794`) — matching the tie-break. The remaining work is the user's note: `smelt.source()` as a call-form should no longer exist — audit the runtime references (`compile.rs:361,474,…`) and remove any live `smelt.source()` call-form handling (or block if load-bearing).
- **D-07 (major, *risk-flagged*).** Mint `ColumnTypeUnresolved` **live** (you chose B). A column whose type degrades to `Unknown` for a **compiler-resolvable** reason (e.g. a `smelt.functions.*`-derived / struct-spread column the schema rules cannot type) must emit `ColumnTypeUnresolved` (Error) at the projection that produced it — distinct from a *genuinely dynamic* `Unknown` and from the absent-type `CannotInferType` (`check_types.rs:110-150`).

## Design decisions (resolved — do not re-litigate; from `docs/research/20260613-spec-remediation-decisions.md` Theme 2 + D-30/D-31)

- **D-07 = B (mint live).** `ColumnTypeUnresolved` is a live catalogued code; the rule that emits it is normative. Owned by `function_schema_inference.md` (schema-propagation trigger/anchor) and `types.md` (the `Unknown` reason-discriminant). It fires for a **compiler-resolvable** `Unknown` (the schema layer *could* type it but the current rules don't), **not** for a genuinely dynamic value. **Risk / block condition:** if distinguishing compiler-resolvable from genuinely-dynamic `Unknown` requires a reason-discriminant that W2 has not yet landed, or the boundary is ambiguous for a given projection, **block** rather than over-fire (a false `ColumnTypeUnresolved` on a legitimately dynamic column is worse than a deferred mint). Sequence this phase last so it can build on W2's `Unknown`-reason work.
- **D-14 = A.** Remove "or the frontmatter itself is malformed" from `BackendsWideningNotAllowed`; route all malformation to `FrontmatterParseError`.
- **D-19 = A.** Dedicated `HofNamedArgument`, fires before the kind check; reserve `HofExpects*` for wrong-kind. Mirrors `ReducerNamedArgument`.
- **D-30 = A.** Directory-scoped uniqueness for `smelt.define` (matches `smelt.<path>` identity); workspace-wide flat namespace only for externs/built-ins. `DuplicateFunctionDefinition` fires for two defines in the **same directory** sharing a name.
- **D-31 = A.** Doctrine wins: unknown key → Error; `FrontmatterParseError` is Error in all cases. (Inapplicable-known-key severity is a *separate* concern — if it currently emits `FrontmatterParseError`-Warning, confirm against the spec whether "Error in all cases" subsumes it; **block** for a call if genuinely unclear rather than silently tightening a Warning to Error.)
- **D-08/D-09 = A.** Bare unresolved `smelt.<path>` → `UndefinedModelRef` (default); `UndefinedSource` reserved for the source-resolution case. No `UnknownSmeltPath`, no `smelt.source()` call-form.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. Red on this phase's own target → proceed; unrelated red → block.
2. **Red-green `/smelt:implement`.** Failing test(s) first, then implementation, spec as oracle. Implementer then reviewer.
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; the **catalogue gate** `cargo test -p smelt-db --test diagnostics_catalogue`; the dual gate `cargo test -p smelt-cli --test example_diagnostics` + `cargo test -p smelt-lsp --test example_workspaces`.
4. **Record + commit.** Row `done` + date; commit + push tests + impl + table with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or roll-up on the last phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)
Set the row `blocked` + one-line reason; append a dated §"Blocked phases" entry; restore a clean committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:
- A design decision not answered by this plan or the spec — notably **D-07's** compiler-resolvable-vs-dynamic `Unknown` boundary (block rather than over-fire); or **D-31's** inapplicable-key severity question.
- Pre-flight red on unrelated breakage; tree can't return to green.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | `HofNamedArgument` — dedicated code, fires before kind check | done | D-19 | feat(db): HofNamedArgument for named args to map/filter/reduce (D-19) | 2026-06-14 |
| P2 | Frontmatter diagnostics: `BackendsWideningNotAllowed` malformed-branch → `FrontmatterParseError`; `FrontmatterParseError` Error in all cases | done | D-14, D-31 | fix(db): route malformed function frontmatter to FrontmatterParseError; Error in all cases (D-14, D-31) | 2026-06-14 |
| P3 | `DuplicateFunctionDefinition` directory-scoped | done | D-30 | fix(db): scope DuplicateFunctionDefinition to a directory, not the workspace (D-30) | 2026-06-14 |
| P4 | D-08/D-09 cleanup: confirm bare-path → `UndefinedModelRef`; retire `smelt.source()` call-form | done | D-08, D-09 | refactor(runtime): drop legacy smelt.source() call-form; lock bare-path → UndefinedModelRef (D-08, D-09) | 2026-06-14 |
| P5 | `ColumnTypeUnresolved` minted live (risk-flagged) | pending | D-07 | feat(db): emit ColumnTypeUnresolved for compiler-resolvable Unknown columns (D-07) | |
| P6 | Close-out: catalogue gate + registry + ROADMAP | pending | — | docs(spec-impl): close out W3 — diagnostics fixes landed; registry + roadmap | |

**Status values**: `pending`, `done`, `blocked`.

---

### Phase P1: `HofNamedArgument`

**Goal.** A HOF call (`map`/`filter`/`reduce`) that passes any argument by name fires `HofNamedArgument` (Error), before the lambda/kind check.

**Pre-conditions.** None.

**TDD tests to write first:**
- `crates/smelt-db/src/type_inference/hof.rs::tests::hof_named_arg_is_error` — `map(xs, fn => …)` written with a named arg (e.g. `map(list: xs, …)`) → `HofNamedArgument`, and a named arg that *is* a lambda still errors (not silently accepted).
- `...::hof_positional_args_ok` — the normal positional form produces no `HofNamedArgument`.
- Catalogue gate green with the new variant.

**Implementation shape.** Add `DiagnosticCode::HofNamedArgument` (`diagnostics_types.rs`). In `hof.rs` (the second-arg handling ~`hof.rs:2147-2227`), check for `NAMED_PARAM` children **before** `extract_lambda_from_expr`, mirroring `ReducerNamedArgument` (`hof.rs:359-366`); emit at the named-arg span.

**Critical files.** `crates/smelt-db/src/diagnostics_types.rs`, `crates/smelt-db/src/type_inference/hof.rs`.

**Review checklist:** fires before the kind check; named-lambda no longer silently accepted; `HofExpects*` reserved for wrong-kind; catalogue gate green.

**Commit.** `feat(db): HofNamedArgument for named args to map/filter/reduce (D-19)`

---

### Phase P2: Frontmatter diagnostics (D-14 + D-31)

**Goal.** Malformed function frontmatter routes to `FrontmatterParseError` (not `BackendsWideningNotAllowed`), and `FrontmatterParseError` is Error in all cases.

**Pre-conditions.** None.

**TDD tests to write first:**
- `crates/smelt-db/src/queries/function_diagnostics.rs::tests::malformed_frontmatter_is_frontmatter_parse_error` — a `smelt.define` with malformed frontmatter → `FrontmatterParseError` (Error), **not** `BackendsWideningNotAllowed`.
- `...::backends_widening_still_fires_on_real_widening` — a genuine `backends:` widening still → `BackendsWideningNotAllowed` (no regression).
- `crates/smelt-core/src/frontmatter.rs::tests::unknown_key_is_error` — already present; assert/lock it (unknown key → Error).

**Implementation shape.** In `function_diagnostics.rs:550,573` split the branches: malformed-frontmatter → `FrontmatterParseError`; keep widening → `BackendsWideningNotAllowed`. Ensure the `FrontmatterParseError` severity mapping (`function_diagnostics.rs:845-861`) yields Error for the unknown-key/malformed cases. If an inapplicable-known-key currently emits `FrontmatterParseError`-Warning, confirm against the spec (block if unclear — see Design decisions D-31).

**Critical files.** `crates/smelt-db/src/queries/function_diagnostics.rs`, `crates/smelt-core/src/frontmatter.rs` (only if the severity origin lives there).

**Review checklist:** `BackendsWideningNotAllowed` no longer fires on malformation; `FrontmatterParseError` Error for unknown-key/malformed; widening unaffected; catalogue/dual gates green.

**Commit.** `fix(db): route malformed function frontmatter to FrontmatterParseError; Error in all cases (D-14, D-31)`

---

### Phase P3: `DuplicateFunctionDefinition` directory-scoped

**Goal.** Two `smelt.define`s collide only when they share a name **in the same directory**; same name in different directories is allowed.

**Pre-conditions.** P depends on **W1** (directory/address model in place).

**TDD tests to write first:**
- `crates/smelt-db/src/queries/function_diagnostics.rs::tests::same_name_different_dirs_ok` — `a/util.sql` and `b/util.sql` each declaring `smelt.define helper` → **no** `DuplicateFunctionDefinition`.
- `...::same_name_same_dir_collides` — two `smelt.define helper` in the same directory (or one file) → `DuplicateFunctionDefinition` at the second name span.
- `...::define_vs_builtin_routes_to_extern_collides` — a define clashing with a built-in → `ExternCollidesWithBuiltin` (unchanged).

**Implementation shape.** In `workspace_function_diagnostics` (`function_diagnostics.rs:37-100`), key the `seen` map by `(directory, name)` instead of `name` alone — derive the directory from the file's path/address (consistent with W1's address derivation). Externs/built-ins keep the flat workspace namespace.

**Critical files.** `crates/smelt-db/src/queries/function_diagnostics.rs`.

**Review checklist:** uniqueness is per-directory; cross-dir same-name allowed; extern/built-in collision unchanged; matches W1 addressing; dual gate green.

**Commit.** `fix(db): scope DuplicateFunctionDefinition to a directory, not the workspace (D-30)`

---

### Phase P4: D-08/D-09 cleanup

**Goal.** Lock that a bare unresolved `smelt.<path>` defaults to `UndefinedModelRef` (with `UndefinedSource` reserved for the source case), and retire the legacy `smelt.source()` call-form.

**Pre-conditions.** None.

**TDD tests to write first:**
- `crates/smelt-db/...::bare_unresolved_path_is_undefined_model_ref` — `smelt.does.not.exist` in value position → `UndefinedModelRef`; `smelt.sources.missing` → `UndefinedSource` (lock the `lib.rs:1746-1794` routing).
- A test asserting `smelt.source(...)` call-form is no longer a recognised surface (parse/resolve error or removed) — or, if the audit shows the refs are dead comments only, a note + no behavior change.

**Implementation shape.** `rg "UnknownSmeltPath"` → confirm zero (no-op). `rg "smelt.source\(|smelt_source|source\("` in `crates/smelt-runtime/src/compile.rs` (lines 361,474,510,597,…) — determine whether `smelt.source()` is a live call-form or dead comments; remove live handling (the single `smelt.<path>` namespace supersedes it). If removal is load-bearing (a real surface still used by fixtures), **block** for a decision rather than break builds.

**Critical files.** `crates/smelt-runtime/src/compile.rs` (legacy call-form removal), tests in `smelt-db`.

**Review checklist:** bare-path routing locked; `smelt.source()` call-form gone or proven dead; no `UnknownSmeltPath` anywhere; dual gate green.

**Commit.** `refactor(runtime): drop legacy smelt.source() call-form; lock bare-path → UndefinedModelRef (D-08, D-09)`

---

### Phase P5: `ColumnTypeUnresolved` minted live (risk-flagged)

**Goal.** A column whose type degrades to `Unknown` for a compiler-resolvable reason emits `ColumnTypeUnresolved` (Error) at the projection that produced it — distinct from genuinely-dynamic `Unknown` and from `CannotInferType`.

**Pre-conditions.** P5 last; benefits from W2's `Unknown`-reason work. **Risk-flagged** — see the block condition.

**TDD tests to write first:**
- `crates/smelt-db/...::function_derived_unknown_column_is_column_type_unresolved` — a `smelt.functions.*` struct-spread / `TableExpr`-derived column the schema rules cannot type → `ColumnTypeUnresolved` at the spread/projection span.
- `...::genuinely_dynamic_unknown_does_not_fire` — a legitimately dynamic `Unknown` (no compiler-resolvable origin) does **not** fire `ColumnTypeUnresolved` (no over-fire); existing `CannotInferType` cases unchanged.
- Catalogue + dual gates green; example workspaces still clean (no spurious new Errors).

**Implementation shape.** Add `DiagnosticCode::ColumnTypeUnresolved` (`diagnostics_types.rs`). In the schema/type-check layer (`check_types.rs:110-150`, with the anchor from the struct-spread site `schema.rs:1473-1562`), distinguish a compiler-resolvable `Unknown` (use the `Unknown` reason-discriminant if W2 landed one) and emit `ColumnTypeUnresolved` there instead of `CannotInferType`. **If the reason-discriminant needed to separate resolvable-from-dynamic is absent or the boundary is ambiguous, block** (record the gap + candidate options) rather than over-fire.

**Critical files.** `crates/smelt-db/src/diagnostics_types.rs`, `crates/smelt-db/src/queries/check_types.rs`, `crates/smelt-db/src/queries/schema.rs` (anchor).

**Review checklist:** fires only for compiler-resolvable `Unknown`, anchored at the projection; no over-fire on dynamic `Unknown`; `CannotInferType` semantics preserved; example workspaces stay clean; catalogue gate green.

**Commit.** `feat(db): emit ColumnTypeUnresolved for compiler-resolvable Unknown columns (D-07)`

---

### Phase P6: Close-out

**Goal.** Confirm the catalogue gate is green with all new variants, retract any now-satisfied Known-Divergence note, roll up.

**Pre-conditions.** P1–P5 done.

**TDD tests to write first:** none new — runs the gates.

**Implementation shape.** `cargo test -p smelt-db --test diagnostics_catalogue` green (all of `HofNamedArgument`, `ColumnTypeUnresolved` present and catalogued). Retract any diagnostics.md/functions.md/types.md Known-Divergence note this wave satisfies (e.g. the `ColumnTypeUnresolved` "reserved/not-yet-minted" remnant, the unknown-key-Warning "divergent" note) — timeless edit. Flip the master registry W3 row to `done (2026-06-13)`; add a `docs/ROADMAP.md` line.

**Critical files.** relevant spec (KD retraction only), `docs/plans/20260613-spec-impl.md`, `docs/ROADMAP.md`.

**Review checklist:** catalogue gate green; KD retractions genuinely satisfied + timeless; registry row `done`; ROADMAP updated.

**Commit.** `docs(spec-impl): close out W3 — diagnostics fixes landed; registry + roadmap`

---

## Deferred during implementation

(Append-only.)

- `DuplicateEmittedName` and `UnknownTestInput` enum variants are added by W1 (P4) and W8-testing respectively, not here — W3 adds only `HofNamedArgument` and `ColumnTypeUnresolved`.

## Blocked phases

Append-only log. None yet.

## Verification

- `cargo test -p smelt-db --test diagnostics_catalogue`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test example_workspaces` green.
- Manual smoke: `map(list: xs, fn => x)` errors `HofNamedArgument`; a malformed `smelt.define` frontmatter errors `FrontmatterParseError` (not `BackendsWideningNotAllowed`); `a/util.sql` + `b/util.sql` both defining `helper` is clean; a function-derived untypeable column errors `ColumnTypeUnresolved`.
- `/smelt:validate diagnostics`, `/smelt:validate functions`, `/smelt:validate meta_language` report no behavioural drift on these surfaces.
