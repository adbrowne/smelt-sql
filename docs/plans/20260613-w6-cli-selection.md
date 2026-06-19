# Plan: W6 — CLI & selection (D-cli)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — the **W6** wave of the spec-remediation backlog. Remediates the **D-cli** cluster of the 2026-06-13 spec review (Theme 7, decisions **D-36…D-41**): the CLI argument-resolution and model-selection surface. All six decisions are already committed to the specs (`cli.md`, `model_selection.md`) by the review; this wave lands the **code** to match the now-authoritative specs. Depends on **W1** (universal `smelt.<path>` addressing + `paths:`-strip scope derivation), which the resolution algorithm builds on. The autonomy loop works this sub-plan phase by phase and rolls up to the master only when it is exhausted.

**Date**: 2026-06-13
**Spec**: `docs/specs/cli.md` §"Argument resolution and `--scope`", §"No-op vs unresolvable selector", §"`--exclude` and inconsistent working sets", §"Argument resolution algorithm", §"Cwd-derived scope computation", §"`smelt test` isolation"; `docs/specs/model_selection.md` §"Selector syntax", §"Selection methods", §"Constraints & Invariants", §"Known Divergences" — these are the correctness oracle.
**Spec diff**: `bab08fb0^..bab08fb0` (`docs(spec): Track C theme 7 — cli & selection (D-36..D-41)`). The behaviour already landed in the specs; this wave is **code-only** (no further spec edits except the close-out KD retraction in P6).
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only. P6 retracts now-satisfied Known-Divergence notes in `model_selection.md`/`cli.md` and may touch `docs-site/` only if the reviewer flags a user-facing gap in the selection docs.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then the spec sections above — they are the correctness oracle; do not re-open the settled decisions (all D-36…D-41 resolved to option **A**). Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked` rows) using the per-phase routine below (pre-flight → red-green `/smelt:implement` with implementer + reviewer, spec as oracle → verification gates → set the row to `done` + date → commit + push with the phase's commit message). If that was the last `pending` phase, also flip this sub-plan's Status to `done (<today>)` in the master registry and commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record + continue, see §"Block conditions"), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>` (sub-plan exhausted), or `<<ALL_DONE>>`. A block is recorded and the loop continues — there is no hard-stop.

## Context (the six decisions, all resolved to A)

| Dec | One-line contract (the spec is authoritative) |
|-----|-----------------------------------------------|
| **D-36** | Entity arguments **accept and strip** a leading `smelt.` prefix, so any printed canonical `smelt.<path>` round-trips straight back into a command. `silver.events_parsed` and `smelt.silver.events_parsed` resolve identically. |
| **D-37** | An **entity-name** selector/argument that resolves to **no entity** is a **hard "not found" error** (non-zero). A **method** selector (`tag:`/`generator_file:`) that legitimately matches no models is a **valid empty selection** (exit `0`, "no models matched" no-op). |
| **D-38** | Leading/trailing `+` **graph operators are stripped before** entity resolution and **re-attached** to the resolved full path afterward (`+events_parsed` → resolve `events_parsed` → `+silver.events_parsed`). No `path:` selection method is added — the methods stay `ModelName`/`Tag`/`GeneratorFile` (the spec resolved the conflict by dropping the undefined `path:` example, not by adding the method). |
| **D-39** | `--exclude +model` removes the model **and its transitive upstreams**; if a removed upstream is still required by a **retained** model, smelt **refuses the inconsistent set** with a diagnostic naming the retained model and the missing upstream, rather than running a model against an absent input. |
| **D-40** | **No cwd-scope fall-through.** A scoped shorthand resolves **only** as `<scope>.<arg>`; if that exact path does not resolve, the command errors — it never silently retries the bare `<arg>`. (Resolved & spec'd 2026-06-13; this wave locks the code to it.) |
| **D-41** | `smelt test --select` uses the **full selector syntax** (the same methods + `+` operators as every other command), not a substring match on test names. |

These are settled; the spec text is the oracle. Do **not** re-litigate option A vs B for any of them.

## Where the code lives (orientation, not a contract)

- `crates/smelt-cli/src/argument_resolution.rs` — scope computation (`compute_scope`, `derive_scope_from_cwd`), `validate_scope_value`, the `strip_smelt_prefix` helper, and the per-arg resolution algorithm. **D-36, D-40** live here.
- `crates/smelt-core/src/selector.rs` (~319 lines) — selector parsing (`Selector`, `Method`, `+` operator parsing) and the engine entry points. **D-38, D-37** live here.
- `crates/smelt-core/src/graph.rs` — name/tag/upstream/downstream selection + the exclusion post-pass. **D-39** lives here (the inconsistent-set check after `--exclude +` upstream removal).
- `smelt test` command path (`crates/smelt-cli/src/commands/…` + `test_property.rs` dispatch) — **D-41**.
- Tests: `crates/smelt-cli/tests/scope_integration.rs` (existing arg-resolution/scope coverage — extend here) and a new `crates/smelt-cli/tests/selector_resolution.rs` for selector-layer behaviour; `crates/smelt-core/src/{selector,graph}.rs` unit tests for the pure layer.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. If red on this phase's own acceptance target (the test it exists to make green), proceed. If red on **unrelated** breakage, treat as a block (record + continue, §"Block conditions").
2. **Red-green `/smelt:implement`.** Write the phase's failing test(s) first, then the implementation, spec as oracle. Implementer pass, then reviewer pass (material findings only).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; the dual example gate `cargo test -p smelt-cli --test example_diagnostics` + `cargo test -p smelt-lsp --test example_workspaces`; scoped `scope_integration` + `selector_resolution` + `example_builds` for any selection fixtures.
4. **Record + commit.** Set the table row to `done` + date; commit + push tests + impl + table together with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or `<<MASTER_EXHAUSTED>>`/roll-up on the last phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue, no hard-stop)
Set the row to `blocked` with a one-line reason; append a dated entry to §"Blocked phases" (phase id, reason, candidate options); restore the tree to a clean committed state; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:
- The tree can't be returned to green after the phase (e.g. an example workspace relies on the old fall-through or substring-match behaviour and needs a fixture redesign larger than this phase).
- The spec is genuinely ambiguous for a real case the phase hits (record the question for a human; do not guess).
- Pre-flight red on unrelated breakage that this phase didn't introduce.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | Entity arguments accept & strip a leading `smelt.` prefix (canonical round-trip) | done | D-36 | feat(cli): entity arguments accept and strip a leading `smelt.` prefix (D-36) | 2026-06-20 |
| P2 | No cwd-scope fall-through: scoped shorthand resolves only `<scope>.<arg>` | done | D-40 | feat(cli): scoped shorthand resolves only `<scope>.<arg>`, no fall-through (D-40) | 2026-06-20 |
| P3 | Strip leading/trailing `+` graph operators before entity resolution; re-attach to full path | done | D-38 | feat(core): strip `+` graph operators before selector entity resolution (D-38) | 2026-06-20 |
| P4 | Unresolvable entity selector = hard "not found" error; empty method selector = exit 0 no-op | pending | D-37 | feat(cli): unresolvable entity-name selector is a hard not-found error (D-37) | |
| P5 | `--exclude +model` dropping a retained dependency → inconsistent-set error | pending | D-39 | feat(core): refuse `--exclude +model` that drops a retained model's upstream (D-39) | |
| P6 | `smelt test --select` uses full selector syntax (not substring) + close-out (KD retraction, registry, ROADMAP) | pending | D-41, D-cli close-out | feat(cli): `smelt test --select` uses full selector syntax; close out W6 (D-41) | |

**Status values**: `pending`, `done`, `blocked`. A phase is `done` only when its tests are red-green confirmed and all gates are green. A `blocked` phase has a dated §"Blocked phases" entry and returns to `pending` once a human resolves it.

---

### Phase P1: Entity arguments accept & strip a leading `smelt.` prefix (D-36)

**Goal.** Every CLI command that takes an entity identifier accepts both the bare form (`silver.events_parsed`) and the canonical printed form (`smelt.silver.events_parsed`), stripping a leading `smelt.` before resolution so any printed identifier copy-pastes straight back. (`cli.md` §"Argument resolution algorithm" step 0; §"Canonical-display rule".)

**Pre-conditions.** W1 addressing landed (it is — P1–P3 done).

**TDD tests to write first:**
- `crates/smelt-cli/tests/scope_integration.rs::arg_strips_leading_smelt_prefix` — `smelt.silver.events_parsed` and `silver.events_parsed` resolve to the same entity.
- `…::select_strips_leading_smelt_prefix` — `--select smelt.silver.events_parsed` round-trips a printed identifier into the same selection as `--select silver.events_parsed`.
- A unit test on the resolution step (`argument_resolution.rs`) asserting step-0 prefix strip happens before scope expansion (so `--scope silver` + `smelt.bronze.x` resolves `bronze.x` as a full path, not `silver.smelt.bronze.x`).

**Implementation shape.** Add a step-0 leading-`smelt.` strip to the per-arg resolution entry point (reuse/extend the existing `strip_smelt_prefix` helper). Apply it on the **argument** path and on the **`ModelName` selector value** path (selectors flow through the same algorithm). Leave `--scope` validation rejecting `smelt.` as-is (that is a separate, intentional rule).

**Critical files.** `crates/smelt-cli/src/argument_resolution.rs`, the selector→arg-resolution bridge (`crates/smelt-core/src/selector.rs` / cli call sites).

**Review checklist:** bare and `smelt.`-prefixed forms resolve identically; strip happens before scope expansion; selectors and bare args both covered; `--scope smelt.x` still rejected.

**Commit.** `feat(cli): entity arguments accept and strip a leading `smelt.` prefix (D-36)`

---

### Phase P2: No cwd-scope fall-through (D-40)

**Goal.** When a scope is active, a shorthand argument resolves **only** as `<scope>.<arg>`. If that exact path does not resolve, the command errors — it never silently retries the bare `<arg>` or searches up the hierarchy. This is what keeps a passing command stable when a top-level entity is added later. (`cli.md` §"Argument resolution and `--scope`" → "No scope-expansion fall-through"; §"Argument resolution algorithm" step 2.)

**Pre-conditions.** P1 (prefix strip in place; the candidate-tuple build is the same code).

**TDD tests to write first:**
- `crates/smelt-cli/tests/scope_integration.rs::scoped_shorthand_no_fall_through` — scope `silver`, only a top-level `events_parsed` exists, `--scope silver events_parsed` → **errors** (does not fall through to top-level `events_parsed`).
- `…::scoped_shorthand_resolves_under_scope` — scope `silver`, `silver.events_parsed` exists, the same shorthand resolves to it.
- `…::full_path_honored_regardless_of_scope` — scope `silver`, `bronze.raw_events` (full path) still resolves (full paths bypass scope narrowing).

**Implementation shape.** In the candidate-tuple build (`argument_resolution.rs` step 2): when scope is `Some(s)`, the **only** candidate is `s ++ arg_segs` — remove any bare-`arg_segs` fall-through candidate. (If the code already does this — the spec describes the landed behaviour — the tests confirm-and-lock it; keep the phase, it pins the regression guard.)

**Critical files.** `crates/smelt-cli/src/argument_resolution.rs`.

**Review checklist:** no fall-through candidate when scope active; full-path args still honored; ambiguity/no-scope paths unchanged; the stability invariant (adding a top-level entity can't retarget a scoped command) is testable.

**Commit.** `feat(cli): scoped shorthand resolves only `<scope>.<arg>`, no fall-through (D-40)`

---

### Phase P3: Strip `+` graph operators before entity resolution (D-38)

**Goal.** A `ModelName` selector may carry leading and/or trailing `+` graph operators. The `+` markers are removed **before** the bare identifier is resolved and **re-attached** to the resolved full path afterward: `+events_parsed` resolves `events_parsed` → `silver.events_parsed` → yields selector `+silver.events_parsed`. The `+` operators never participate in entity resolution. No `path:` method is added (methods stay `ModelName`/`Tag`/`GeneratorFile`). (`cli.md` §"Argument resolution algorithm" → "Graph operators are stripped before resolution"; `model_selection.md` §"Selection methods".)

**Pre-conditions.** P1 (the arg-resolution entry the stripped name is fed to).

**TDD tests to write first:**
- `crates/smelt-cli/tests/selector_resolution.rs::plus_prefix_resolves_then_reattaches` — `+events_parsed` (scope `silver`) selects `silver.events_parsed` **and its upstreams** (operator preserved through resolution).
- `…::plus_suffix_and_both` — `events_parsed+` and `+events_parsed+` resolve the leaf and preserve downstream / both-direction traversal.
- `crates/smelt-core/src/selector.rs` unit test: parsing `+name+` yields a `ModelName` method with leading+trailing operators and a bare `name` value (no `+` in the resolved identifier).
- `…::no_path_method` — a selector `path:models/silver` is **not** a recognized method (parse error or unresolved per the grammar), confirming `path:` was not added.

**Implementation shape.** Ensure the selector parser separates the `+` operator flags from the `ModelName` value, the resolution bridge strips/re-attaches around `argument_resolution`, and the graph traversal reads the operators from the resolved selector. Confirm the methods enum has no `path:` variant.

**Critical files.** `crates/smelt-core/src/selector.rs`, `crates/smelt-core/src/graph.rs`, the cli selector→resolution bridge.

**Review checklist:** `+` stripped before resolution and re-attached; upstream/downstream/both traversal intact; `tag:`/`generator_file:` pass through unchanged (not entity-resolved); no `path:` method.

**Commit.** `feat(core): strip `+` graph operators before selector entity resolution (D-38)`

---

### Phase P4: Unresolvable entity selector = hard error; empty method selector = exit 0 (D-37)

**Goal.** Distinguish the two empty-output cases by exit code. An **entity-name** selector/argument that resolves to no entity of any kind is a **hard "not found" diagnostic** (non-zero) — a typo fails loudly. A **method** selector (`tag:`/`generator_file:`) that legitimately matches no models, or a valid selector whose result set is empty/up-to-date, is a quiet no-op (`exit 0`, "no models matched the selector(s)" / "nothing to rebuild" to stderr). (`cli.md` §"No-op vs unresolvable selector"; `model_selection.md` Constraint 4.)

**Pre-conditions.** P3 (operator-stripped entity names reach resolution cleanly).

**TDD tests to write first:**
- `crates/smelt-cli/tests/selector_resolution.rs::unresolvable_entity_select_is_hard_error` — `--select typo_name` → non-zero exit + "not found" diagnostic (with `did you mean` when exactly one leaf matches).
- `…::empty_tag_selection_is_noop_exit_0` — `--select tag:nonexistent` → exit `0`, "no models matched" to stderr.
- `…::generator_file_no_match_is_noop` — `generator_file:` pointing at a non-generator/missing path → exit `0`, empty set, no error.
- `…::bare_leaf_ambiguous_is_error` — bare `events_parsed`, no scope, two `*.events_parsed` models → non-zero ambiguity diagnostic listing both.

**Implementation shape.** Route `ModelName` selector resolution failure to the same hard "not found" path bare command args use (`argument_resolution` step 4/5), while `tag:`/`generator_file:` non-matches return an empty set that the command surfaces as the no-op message with exit 0. Pin the exit-code contract in the command layer.

> **Note on the existing implementation divergence** (`cli.md` §"No-op vs unresolvable selector" implementation note): the current no-op message is logged via `info!()` (only visible under `RUST_LOG=info`). Bringing the no-op message to stderr unconditionally is in scope for this phase if it falls out cleanly; if it needs a larger logging-surface change, land the exit-code + hard-error contract here and leave the message-visibility fix as a recorded follow-up (do not block the phase on it).

**Critical files.** `crates/smelt-cli/src/argument_resolution.rs` (not-found/ambiguous diagnostics), `crates/smelt-core/src/selector.rs` / `graph.rs` (method non-match → empty set), the command exit-code paths (`run`/`build`/`explain`/`diff`).

**Review checklist:** entity typo → non-zero; empty method selector → exit 0 no-op; ambiguity → non-zero; the asymmetry matches Constraint 4; `did you mean` hint present when exactly one leaf matches.

**Commit.** `feat(cli): unresolvable entity-name selector is a hard not-found error (D-37)`

---

### Phase P5: `--exclude +model` inconsistent-set refusal (D-39)

**Goal.** `--exclude +model` removes the model and its transitive upstreams (after all `--select` expansions). If any removed upstream is still required by a model that **remains** in the working set, smelt refuses to run the inconsistent set: it emits a diagnostic naming the retained model and the missing upstream dependency rather than executing a model against an absent input. (`cli.md` §"`--exclude` and inconsistent working sets"; `model_selection.md` §"Known Divergences" → "`--exclude` with `+` traversal".)

**Pre-conditions.** P3 (`+` operators resolve on `--exclude` selectors too).

**TDD tests to write first:**
- `crates/smelt-cli/tests/selector_resolution.rs::exclude_upstream_needed_by_retained_is_error` — select `{A, B}` where both depend on shared upstream `U`; `--exclude +A` (removing `A` and `U`) while `B` is retained and needs `U` → **error** naming `B` and `U`.
- `…::exclude_upstream_not_needed_ok` — `--exclude +A` where the removed upstreams are not needed by any retained model → succeeds, consistent set.
- `…::exclude_bare_model_only` — `--exclude A` (no `+`) removes only `A`, leaves upstreams (no inconsistency check tripped).

**Implementation shape.** After the exclusion post-pass in the selection engine (`graph.rs`), validate set-consistency: for every retained model, every direct dependency must still be in the working set (or be a non-built input like a source/seed per existing rules); a missing built dependency caused by `--exclude +` is the diagnostic. Reuse the existing dependency graph; this is a post-selection validation, not a new traversal.

**Critical files.** `crates/smelt-core/src/graph.rs` (consistency check after exclusion), the cli command path that surfaces the diagnostic + non-zero exit.

**Review checklist:** retained-model-needs-removed-upstream → error naming both; consistent exclusions still pass; bare `--exclude model` unaffected; sources/seeds not falsely flagged as missing built deps.

**Commit.** `feat(core): refuse `--exclude +model` that drops a retained model's upstream (D-39)`

---

### Phase P6: `smelt test --select` full selector syntax + close-out (D-41)

**Goal.** `smelt test --select` uses the **full selector syntax** — the same methods (`ModelName`/`tag:`/`generator_file:`) and `+` graph operators as every other command — instead of a substring match on test names. Then close out the wave. (`cli.md` §"`smelt test` isolation"; `model_selection.md` §"Known Divergences" → "`--select` on `smelt test` selector-syntax rollout".)

**Pre-conditions.** P3–P4 (selector parsing + entity resolution + not-found semantics, which `smelt test --select` now reuses).

**TDD tests to write first:**
- `crates/smelt-cli/tests/selector_resolution.rs::test_select_uses_selector_syntax` — `smelt test --select tag:X` selects tests of models tagged `X` (not a substring match on test names); `--select +model` selects model + upstream tests.
- `…::test_select_unresolvable_is_hard_error` — `smelt test --select typo_name` → non-zero not-found (same fail-loud as other commands).

**Implementation shape.** Replace the substring filter in the `smelt test` command path with the shared selection engine (the same `--select`/scope/resolution flow `run`/`build` use), mapping the resolved model set to its tests. Remove the old substring code path.

**Close-out (same commit):**
- Retract the now-satisfied Known-Divergence notes: `model_selection.md` "§Known Divergences → `--select` on `smelt test`" and the `cli.md` no-op `info!()` implementation note **iff** P4 landed the stderr message (otherwise leave it as a recorded follow-up).
- Single canonical-display round-trip audit: confirm every command that prints an entity prints the full `smelt.<path>` form and that printed form copy-pastes back (D-36 invariant) — add one round-trip integration test if not already covered.
- Flip this sub-plan's registry row in `docs/plans/20260613-spec-impl.md` to `done (<today>)`; add a `docs/ROADMAP.md` line.

**Critical files.** `crates/smelt-cli/src/commands/…` (test command), `test_property.rs` dispatch, `docs/specs/model_selection.md` + `docs/specs/cli.md` (KD retraction only), `docs/plans/20260613-spec-impl.md`, `docs/ROADMAP.md`.

**Review checklist:** `smelt test --select` uses selector syntax end-to-end; substring path removed; unresolvable test selector fails loudly; KD retractions genuinely satisfied (don't retract the `info!()` note if P4 deferred it); round-trip audit passes; registry + ROADMAP updated.

**Commit.** `feat(cli): `smelt test --select` uses full selector syntax; close out W6 (D-41)`

---

## Deferred during implementation

(Append-only.)

- If P4 cannot bring the no-op message to stderr without a larger logging-surface change, the message-visibility fix is recorded here as a follow-up; the exit-code + hard-error contract still lands in P4.

## Blocked phases

Append-only log. None yet.

## Verification

- `cargo test` green; `cargo test -p smelt-cli --test scope_integration` + `--test selector_resolution` green; the dual example gate (`example_diagnostics` + `example_workspaces`) green; scoped `example_builds` for any selection fixtures.
- Manual smoke: a printed `smelt.silver.events_parsed` pastes back into `--select` and resolves; `--select typo` exits non-zero; `--select tag:nonexistent` exits 0 with a no-op message; `--exclude +shared_upstream` while a dependent is retained errors naming both; `smelt test --select tag:X` runs the tagged models' tests.
- `/smelt:validate cli` and `/smelt:validate model_selection` report no behavioural drift on the argument-resolution and selection surfaces.
