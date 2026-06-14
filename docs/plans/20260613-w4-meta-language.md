# Plan: W4 — Meta-language reflection & precedence (D-meta)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — the fourth wave of the spec-remediation implementation backlog. Remediates the **D-meta** cluster of the 2026-06-13 spec review: **D-15** (spread `...` outermost/lowest precedence), **D-16** (`ModelRef`/`SourceRef` `name`/`path` are string-literal data, carved out of the identifier lift), **D-17** (wide-reflection ordering by `path` then `name`), **D-18** (deferred `m.has(k)` stays Boolean → short-circuit governs), **D-20** (`ModelRef.path` = generator-file provenance; path-keyed ops use per-emission identity), **D-21** (`ColumnRef` head-constructor predicates + exact-structural `c.type` equality). No hard dependency on W1; W2 helps P3, and P5's per-emission identity builds on W1's address authority. The autonomy loop works this sub-plan phase by phase.

**Date**: 2026-06-13
**Spec**: `docs/specs/meta_language.md` §"Pipe" (D-15 precedence), §"Reflection: `smelt.columns_of`, `ColumnRef`, identifier lift" (D-21 ColumnRef fields + exact equality), §"Reflection: `smelt.models`, `smelt.sources`, `ModelRef`, `SourceRef`" rules 2/7/8 (D-17 ordering, D-16/D-20 lift carve-out + path provenance), §"Maps" rule 4 + §"Meta-world ternary" rule 4 (D-18 deferred `has`); `docs/specs/types.md` §"Type constraints" (the head-constructor families).
**Spec diff**: `e862ebec..HEAD` — **already landed**. Code-catching-up-to-spec; no spec edits except the P6 close-out retraction of any now-satisfied Known-Divergence note.
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only. Close-out updates the master registry + `docs/ROADMAP.md`.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then the spec sections above — they are the correctness oracle; do not re-open the settled decisions. Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked`) per the per-phase routine below. If that was the last `pending` phase, flip this sub-plan's Status to `done (<today>)` in the master registry and commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record + continue), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>`, or `<<ALL_DONE>>`.

**Scope fence — this wave does NOT touch evaluation-order interleaving.** The `meta_language.md` diff also carries the D-24 "combined fully-interleaved fixed-point with Python" text; *implementing* that interleaving is **W5b** (isolated), not here. W4 is reflection + precedence only (D-15/16/17/18/20/21). `HofNamedArgument` (D-19) is W3, already scaffolded — do not redo it.

## Goal

Bring the meta-language surface into line with the reviewed `meta_language.md`:

- **D-15 (critical).** Pipe `|>` is lowest in the expression grammar (`crates/smelt-parser/src/parser/expr.rs:136-175`); spread `...` is a prefix in comma positions (`expr.rs:936-942`, `select.rs:156`). The spec makes `...` the **outermost (lowest)** operator: `...smelt.columns_of(x) |> map(f)` must parse as `...(… |> map(f))` (build the list, transform via pipe, *then* spread) — no parens.
- **D-17 (critical, determinism).** `smelt.models.all` already sorts `path` then `name` (`project.rs:2361`), but `models_with_tag` (`:2458`), `sources_with_tag` (`:2509`), and `sources_all` (`:2524`) sort by `path` only. Co-emitted models share one generator `path`, so `path` alone is non-deterministic — add the `name` tiebreaker everywhere.
- **D-21 (minor).** `ColumnRef` needs head-constructor predicates `is_decimal`/`is_string`/`is_temporal`/`is_integer`/`is_boolean` (`signatures.rs:3350-3363` field set; `meta_eval.rs:380-389` materialisation), so a user can test "any `Decimal`" without spelling out precision/scale (since `c.type == T` is exact). `c.type` exact-structural equality is **normative-but-unlanded** — `c.type` stays `Unknown` (a bigger meta-`DataType` change); land only the predicates and keep the divergence note.
- **D-18 (major).** A deferred `m.has(k)` (non-static key, `record.rs:728-732`) must stay a `Boolean` meta-value, **not** collapse the ternary COND to `Unknown` (`ternary.rs:114-129`), so `if m.has(k) then m.get(k) else default` short-circuits for dynamic keys, not only static ones.
- **D-16 (critical).** `ModelRef`/`SourceRef` `name`/`path` are **data values** (already rendered as SQL string literals on the build path, `meta_eval.rs:478-489`) and must be **carved out** of the four-position identifier lift (column-ref / AS-alias / ORDER BY / GROUP BY) — they render as string literals there too, never bare identifiers. `ColumnRef.name` still lifts.
- **D-20 (minor).** `ModelRef.path` stays the **generator-file** path (provenance; N co-emitted models share it — `project.rs:2347-2358` already does this). Every **path-keyed** operation (collision/dedup/goto-def) must key on the model's **per-emission `smelt.<path>` address**, not the generator path; ordering uses `path` then `name` (D-17).

## Design decisions (resolved — do not re-litigate; from `docs/research/20260613-spec-remediation-decisions.md` Theme 3)

- **D-15 = A.** `...` is the outermost/lowest operator; it applies after the pipe chain. Parse the spread operand through the pipe-level expression parser so `...a |> map(f)` ≡ `...(a |> map(f))`.
- **D-16 = B.** `ModelRef`/`SourceRef` `name`/`path` are data values (string literals), **not** subject to the identifier lift — carve them out of the lift table. A model name is rarely an in-scope column; treating it as data avoids accidental identifier injection.
- **D-17 = A.** Canonical wide-reflection order is `path` then `name` **everywhere** (byte-lexicographic). Determinism restored for co-emitted models.
- **D-18 = A.** A deferred boolean `has` does **not** collapse COND to `Unknown` — it stays a Boolean meta-value whose resolution defers to expansion; the short-circuit rule governs. `Unknown`-collapse is reserved for COND evaluations that *fail* (a surfaced diagnostic), not ones that merely *defer*.
- **D-20 = A.** `path` = generator file (provenance); path-keyed operations use the per-emission `smelt.<path>` address (derived per `architecture.md` §"Resolution" — i.e. W1's address authority); goto-def on an emitted `ModelRef` resolves through that address.
- **D-21 = A.** Exact structural equality for `c.type` (incl. type parameters) is the *normative* rule, but it is **unlanded** — `c.type` returns `Unknown` today (a Phase-D-scale meta-`DataType` change, out of scope). This wave lands only the head-constructor predicates and keeps the "`c.type` returns `Unknown`" Known Divergence. **Do not attempt to land `c.type` equality** — that is a separate, larger change.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. Red on this phase's own target → proceed; unrelated red → block.
2. **Red-green `/smelt:implement`.** Failing test(s) first, then implementation, spec as oracle. Implementer then reviewer.
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; `cargo test -p smelt-parser` (P1) / `cargo test -p smelt-runtime` (meta_eval) / `cargo test -p smelt-db` (type inference) as relevant; the dual gate `cargo test -p smelt-cli --test example_diagnostics` + `cargo test -p smelt-lsp --test example_workspaces`.
4. **Record + commit.** Row `done` + date; commit + push tests + impl + table with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or roll-up on the last phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)
Set the row `blocked` + one-line reason; append a dated §"Blocked phases" entry; restore a clean committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:
- A design decision not answered by this plan or the spec — e.g. the identifier-lift site (D-16) turns out to be entangled such that carving out `ModelRef`/`SourceRef` risks `ColumnRef.name`'s lift; or D-20's per-emission goto-def needs a target the per-emission address authority doesn't yet expose.
- Pre-flight red on unrelated breakage; tree can't return to green.
- **Do not** treat `c.type`-equality-not-working as a bug to fix here (it's the documented unlanded divergence — D-21).

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | Spread `...` outermost: parses after the pipe chain | done | D-15 | feat(parser): spread is the outermost operator; applies after pipe chains (D-15) | 2026-06-14 |
| P2 | Wide-reflection ordering by `path` then `name` everywhere | done | D-17 | fix(db): sort with_tag/all wide reflection by path then name (D-17) | 2026-06-14 |
| P3 | `ColumnRef` head-constructor predicates (`is_decimal`/`is_string`/`is_temporal`/`is_integer`/`is_boolean`) | pending | D-21 | feat(meta): ColumnRef head-constructor predicates for family tests (D-21) | |
| P4 | Deferred `m.has(k)` stays Boolean → short-circuit governs | pending | D-18 | fix(db): deferred map has() stays Boolean, never collapses ternary to Unknown (D-18) | |
| P5 | `ModelRef`/`SourceRef` `name`/`path` carved out of identifier lift; per-emission identity for path-keyed ops | pending | D-16, D-20 | fix(meta): ModelRef/SourceRef name/path render as string literals, not lifted identifiers (D-16, D-20) | |
| P6 | Close-out: registry + ROADMAP | pending | — | docs(spec-impl): close out W4 — meta-language reflection/precedence landed; registry + roadmap | |

**Status values**: `pending`, `done`, `blocked`.

---

### Phase P1: Spread `...` outermost

**Goal.** `...expr |> map(f)` parses as `...(expr |> map(f))` — the spread wraps the entire pipe chain, no parentheses.

**Pre-conditions.** None.

**TDD tests to write first:**
- `crates/smelt-parser/src/parser/tests.rs::spread_wraps_pipe_chain` — `...smelt.columns_of(x) |> map(fn c => c.name)` produces a `LIST_SPREAD` whose operand is the `PIPE_EXPR` (not a `PIPE_EXPR` whose LHS is the spread).
- `...::spread_of_plain_value_unchanged` — `...xs` (no pipe) still parses as a spread of `xs` (no regression).
- A `smelt-runtime` meta_eval test that the spread-of-piped-reflection expands correctly end-to-end (the spec's flagship build-path example).

**Implementation shape.** In the comma-position spread parse (`parse_list_spread`, `expr.rs:936-942`, used from `select.rs:156` and other list positions), parse the spread operand through the **pipe-level** expression entry (`parse_pipe_expr`) so the pipe binds inside the spread. Confirm the precedence comment (`syntax_kind.rs:273`) and update it to "spread is outermost; pipe is next".

**Critical files.** `crates/smelt-parser/src/parser/expr.rs`, `crates/smelt-parser/src/parser/select.rs` (and any other list-position spread call site), `crates/smelt-parser/src/syntax_kind.rs` (comment).

**Review checklist:** spread wraps the pipe chain in every list position (SELECT, GROUP BY, ORDER BY, list literal, args); plain `...xs` unchanged; meta_eval expands the flagship example; parser tests green.

**Commit.** `feat(parser): spread is the outermost operator; applies after pipe chains (D-15)`

---

### Phase P2: Wide-reflection ordering `path` then `name`

**Goal.** `with_tag`/`all` for both models and sources sort ascending by `path`, then `name` as tiebreaker — a total, byte-equal order.

**Pre-conditions.** None.

**TDD tests to write first:**
- `crates/smelt-db/src/queries/project.rs::tests::with_tag_orders_by_path_then_name` — two co-emitted models sharing a generator `path` come back in `name` order; a `models_with_tag` / `sources_with_tag` / `sources_all` result is `path`-then-`name`.
- A `smelt-runtime` test that `reduce(union_all)` over a wide-reflection result follows `path`-then-`name` row order.

**Implementation shape.** Add `.then_with(|| a.name.cmp(&b.name))` to the sort at `project.rs:2458` (`models_with_tag`), `:2509` (`sources_with_tag`), `:2524` (`sources_all`). `models_all` (`:2361`) already does this — leave it.

**Critical files.** `crates/smelt-db/src/queries/project.rs`.

**Review checklist:** all four wide-reflection accessors sort `path` then `name`; determinism observable in `reduce(union_all)`; dual gate green.

**Commit.** `fix(db): sort with_tag/all wide reflection by path then name (D-17)`

---

### Phase P3: `ColumnRef` head-constructor predicates

**Goal.** `ColumnRef` exposes `is_decimal`, `is_string`, `is_temporal`, `is_integer`, `is_boolean` (each `Boolean`, true iff `c.type`'s head constructor is in that family), so family tests don't need exact `==`.

**Pre-conditions.** W2 helps (the string/temporal family definitions align with W2's `normalize()`), not a hard dep.

**TDD tests to write first:**
- `crates/smelt-db/src/type_inference/tests.rs::column_ref_head_predicates` — `c.is_decimal` true for `Decimal(p,s)` of any p/s; `c.is_string` true for Text/Varchar/Char; `c.is_temporal` for Date/Timestamp/TimestampTz/Time; `c.is_integer` for any integer width; `c.is_boolean` for Boolean; each false otherwise.
- `...::column_ref_field_unknown_lists_new_fields` — `c.bogus` → `ColumnRefFieldUnknown` whose message lists `name, type, is_numeric, is_decimal, is_string, is_temporal, is_integer, is_boolean`.
- A `smelt-runtime` meta_eval test materialising the predicates from a real column's `DataType`.

**Implementation shape.** Add the five fields to `COLUMN_REF_FIELDS` (`smelt-types/src/signatures.rs:3350-3363`, type `Expr<Boolean>`); compute them in `meta_eval.rs:380-389` from `ColumnRefMeta`'s `DataType` head constructor (add `is_*` flags to `ColumnRefMeta` if needed). Update the `ColumnRefFieldUnknown` message + LSP completion list. **Leave `c.type` returning `Unknown`** (the unlanded structural-equality divergence — do not change it; the spec's Known Divergence stays).

**Critical files.** `crates/smelt-types/src/signatures.rs`, `crates/smelt-runtime/src/meta_eval.rs`, the `ColumnRefFieldUnknown` message + completion site.

**Review checklist:** five predicates resolve correctly over the head constructor irrespective of params; closed-field message/completion updated; `c.type` still `Unknown` (divergence preserved); tests green.

**Commit.** `feat(meta): ColumnRef head-constructor predicates for family tests (D-21)`

---

### Phase P4: Deferred `m.has(k)` stays Boolean

**Goal.** A deferred `m.has(k)` (non-static key) is a `Boolean` meta-value, so `if m.has(k) then m.get(k) else default` short-circuits for dynamic keys instead of collapsing to `Unknown`.

**Pre-conditions.** None.

**TDD tests to write first:**
- `crates/smelt-db/src/type_inference/record.rs::tests::deferred_has_is_boolean_not_unknown` — `m.has(k)` with a non-static key synthesises `Boolean` (not `Unknown`).
- `crates/smelt-db/src/type_inference/ternary.rs::tests::deferred_has_cond_short_circuits` — `if m.has(k) then m.get(k) else default` with a dynamic `k` does **not** collapse the ternary to `Unknown`, and `MapGetMissingKey` is **not** spuriously emitted on the `m.get(k)` branch.
- `...::failed_cond_still_collapses` — a COND that genuinely *fails* (e.g. `ConfigVarNotFound`) still collapses to `Unknown` (no over-correction).

**Implementation shape.** In `infer_map_method_call` (`record.rs:728-732`), make a deferred `has` resolve to `Boolean` (keep `Deferred` resolution semantics but type it `Boolean`, not `Unknown`). In `infer_ternary_type` (`ternary.rs:114-129`), ensure a `Boolean`-but-deferred COND drives the short-circuit path (rule 3), and reserve `Unknown`-collapse for a COND that surfaced a diagnostic.

**Critical files.** `crates/smelt-db/src/type_inference/record.rs`, `crates/smelt-db/src/type_inference/ternary.rs`.

**Review checklist:** deferred `has` is Boolean; dynamic-key defaulting short-circuits without spurious `MapGetMissingKey`; failed CONDs still collapse to `Unknown`; tests green.

**Commit.** `fix(db): deferred map has() stays Boolean, never collapses ternary to Unknown (D-18)`

---

### Phase P5: `ModelRef`/`SourceRef` name/path carve-out + per-emission identity

**Goal.** `m.name`/`m.path`/`s.name`/`s.path` render as SQL **string literals** in all positions (carved out of the identifier lift); path-keyed operations use the per-emission `smelt.<path>` address while `ModelRef.path` stays generator-file provenance.

**Pre-conditions.** Benefits from **W1** (per-emission address authority).

**TDD tests to write first:**
- `crates/smelt-runtime/src/meta_eval.rs::tests::model_ref_name_renders_as_string_literal` — `m.name`/`m.path` in column-ref / AS-alias / ORDER BY / GROUP BY position emit a quoted string literal, not a bare identifier.
- `...::column_ref_name_still_lifts` — `ColumnRef.name` in those positions still lifts to an identifier (the carve-out is specific to ModelRef/SourceRef).
- A goto-def test (smelt-lsp or smelt-db) that goto-def on an emitted `ModelRef` resolves through its per-emission `smelt.<path>` address, while `ModelRef.path` still reports the generator-file path.

**Implementation shape.** Locate the four-position identifier-lift site (printer/transformer or the splice path that turns meta-`Text` into identifiers) and exclude `ModelRef`/`SourceRef` `name`/`path`-sourced values (render as string literal — the build path already does this at `meta_eval.rs:478-489`, so the fix is ensuring the lift doesn't re-promote them). Confirm `make_model_ref_value_from_emitted` (`project.rs:2347-2358`) keeps `path` = generator file; ensure goto-def / dedup key on the per-emission address (W1 authority), not the generator path.

**Critical files.** the identifier-lift site (printer/transformer), `crates/smelt-runtime/src/meta_eval.rs`, `crates/smelt-db/src/queries/project.rs`, the goto-def resolution site for emitted `ModelRef`s.

**Review checklist:** ModelRef/SourceRef name/path never lift to identifiers; ColumnRef.name still lifts; `path` stays generator-file provenance; path-keyed ops use per-emission address; goto-def resolves correctly; dual gate green.

**Commit.** `fix(meta): ModelRef/SourceRef name/path render as string literals, not lifted identifiers (D-16, D-20)`

---

### Phase P6: Close-out

**Goal.** Retract any now-satisfied Known-Divergence note, roll up.

**Pre-conditions.** P1–P5 done.

**TDD tests to write first:** none new — runs the gates.

**Implementation shape.** Retract any meta_language.md Known-Divergence note this wave satisfies (e.g. a wide-reflection ordering-non-determinism note, or a spread-precedence note) — timeless edit. **Keep** the "`ColumnRef.type` projection returns `Unknown`" divergence (still unlanded by design). Flip the master registry W4 row to `done (2026-06-13)`; add a `docs/ROADMAP.md` line.

**Critical files.** `docs/specs/meta_language.md` (KD retraction only), `docs/plans/20260613-spec-impl.md`, `docs/ROADMAP.md`.

**Review checklist:** KD retractions genuinely satisfied + timeless; `c.type`-Unknown divergence retained; registry row `done`; ROADMAP updated.

**Commit.** `docs(spec-impl): close out W4 — meta-language reflection/precedence landed; registry + roadmap`

---

## Deferred during implementation

(Append-only.)

- `c.type` exact-structural-equality (the meta-`DataType` comparison surface) is **normative-but-unlanded** — `c.type` stays `Unknown`; landing it is a separate, larger meta-`DataType` change, not in W4.
- Python↔SQL evaluation-order interleaving (the D-24 text in the meta_language.md diff) is implemented in **W5b**, not here.

## Blocked phases

Append-only log. None yet.

## Verification

- `cargo test -p smelt-parser`, `cargo test -p smelt-runtime`, `cargo test -p smelt-db`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test example_workspaces` green.
- Manual smoke: `...smelt.columns_of(x) |> map(fn c => c.name)` expands without parens; `c.is_decimal` tests any `Decimal`; `if m.has(dynamic_k) then m.get(dynamic_k) else d` short-circuits; `m.name` in an AS-alias renders as a string literal; `smelt.models.all` over co-emitted models is `path`-then-`name`.
- `/smelt:validate meta_language` reports no behavioural drift on these surfaces (the `c.type`-Unknown divergence remains, by design).
