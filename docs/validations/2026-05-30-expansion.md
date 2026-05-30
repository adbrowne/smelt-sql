## Drift Report: expansion

**Spec**: docs/specs/expansion.md (last_reviewed: 2026-05-13)
**Date**: 2026-05-30
**Phase**: B1 (feature sweep)

### Automated checks
- cargo fmt — PASS (`cargo fmt --all -- --check`)
- cargo clippy — PASS (`cargo clippy --all-targets`, zero warnings)
- cargo test — PASS (full suite green at pre-flight)
- example_diagnostics — PASS (87 passed, 1 ignored)
- example_workspaces (LSP) — PASS (27 passed)

### Surface drift
- ✅ **`FrameInfo` shape** — all spec fields present in `crates/smelt-types/src/signatures.rs:1619` (`function`, `param`, `bound_type`, `decl_path`, `decl_range`, `call_site_range`, `fn_id`, `element_index`, `column_origin`, `model_origin`, `source_origin`). Lives in `smelt-types` (Salsa-free, invariant #4 upheld).
- ✅ **Anonymous HOF frames** — `<map>`/`<filter>`/`<reduce>` bracket form asserted in `function_body_check.rs::tests` (lines 3475–3533); `fn_id = None`, `param = ""` form confirmed.
- ✅ **`<generator>` frame** — `make_generator_frame` + `stamp_generator_frame_onto` at `function_body_check.rs:357,389`; inline tests `generator_frame_*` pass; appended outermost.
- ✅ **`column_origin` / `model_origin` / `source_origin`** — producer-side fields present; renderer-side deferral matches Known Divergences.
- ⚠️ **CTE-collision diagnostic (§Surface, line 21)** — spec: "When a CTE name introduced inside a function body would collide with a CTE in the calling scope **at codegen time, the compiler emits a diagnostic** rather than alpha-renaming." **No such diagnostic exists.** `rg` for caller-vs-callee CTE collision detection across `crates/` finds nothing in the planner expansion path (`smelt-planner/src/logical_plan_rules.rs` has no CTE handling) nor in `smelt-db`. The only CTE diagnostic is `CteCycle` (within-body mutual recursion, `cte_splice.rs::cte_cycle_detected`). See **BUG-007** — this is a soundness gap, not merely a missing diagnostic: the collision produces silently-wrong data.
- ⚠️ **`make_generator_frame` signature (Known Divergences, line 142)** — spec documents the constructor as `make_generator_frame(path, body_range, file_text)` (3 args). Actual signature is `make_generator_frame(generator_file_path, body_range)` (2 args; `function_body_check.rs:357`). The `file_text` parameter was removed. Minor spec inaccuracy. See **BUG-008**.

### Semantics drift
- ✅ **Two senses of expansion** (§Semantics) — Tier 1 type-check-time (no CST mutation) in `function_body_check.rs` (`walk_body_with_ctx`); codegen-time CST splice in `smelt-planner` (`ExpandTransparentFunctionCalls`, `phase41_body_splice_tests.rs` green).
- ✅ **Provenance origin tags** — `Caller`/`Callee`/`Synthesized` (`ProvenanceTag`); `provenance_preserved_through_splice` asserts `Callee` tags survive splice.
- ✅ **Frame-stack invariants 1–6** — innermost-first ordering, one-frame-per-level, no defensive empty stack, HOF anonymous frames obey ordering — covered by `function_body_check.rs::tests` (single/multi-frame, `<map>` inner + `<generator>` outer).
- ❌ **Hygiene v1 rule #3 (CTE-name collisions emit a diagnostic)** — **NOT upheld**. Reproduced: a function whose body declares `WITH helper AS (...)`, called with a caller CTE also named `helper`, emits SQL `FROM ((WITH helper AS (SELECT 1 AS amount) SELECT amount*2 AS doubled FROM helper))` where the inner `FROM helper` resolves to the **body's** CTE rather than the substituted argument → the caller's data is silently dropped (result `2` instead of `20`). No diagnostic, no alpha-rename. See **BUG-007**.
- ✅ **Hygiene v1 rules #1 (parameters-first) & #2 (placeholder CST kinds)** — parameters-first resolution covered by scoping tests; placeholder-kind substitution exercised by `phase41_body_splice_tests.rs`.

### Invariant drift
- ✅ **#1 Expansion is AST-level** — codegen splice operates on `Plan`/CST nodes, not text.
- ✅ **#2 Origin tags conservative** — `provenance_preserved_through_splice` confirms tags propagate.
- ✅ **#3 Frame stack innermost-first** — pinned by snapshot-style frame tests.
- ✅ **#4 `FrameInfo` Salsa-free** — lives in `smelt-types`; no Salsa dep.
- ✅ **#5 Type-check expansion bounded by cycle pre-pass** — `FunctionCallCycle` guards.
- ⚠️ **CTE hygiene (Constraints lean on Hygiene v1 #3)** — the collision-diagnostic safety net the spec relies on to keep collisions in the "hygiene gap, not soundness gap" category is **absent**, so collisions are currently a soundness gap (BUG-007).

### Timeless-oracle drift
- ⚠️ Spec body contains phase vocabulary in `### Why this matters as a spec` adjacent prose? — checked: phase references appear only in §Known Divergences (line 143, with a `docs/plans/...` link) and §References → Plans (history). The §Surface/§Semantics/§Design bodies are clean. **No timeless-oracle drift** in normative sections. (Note: `signatures.rs` and `function_body_check.rs` docstrings carry "Phase N" labels, but those are code comments, not spec/user-doc body — out of scope for the timeless-oracle rule.)

### Freshness
- last_reviewed: 2026-05-13
- most recent code change to References → Code paths: the meta-language work (generator frames, `model_origin`/`source_origin`) post-dates 2026-05-13; spec body reflects it, but the `make_generator_frame` signature note (line 142) is stale.
- Verdict: **mostly fresh**; one stale Known-Divergence signature line (BUG-008).

### Summary
- Drift items: 3 — 2 surface (BUG-007 CTE-collision diagnostic absent + soundness; BUG-008 stale constructor signature), 1 semantics (Hygiene v1 #3, same root as BUG-007).
- Recommended next step: human review of BUG-007 (implement codegen-time CTE-collision detection vs. amend spec to demote Hygiene #3 to a Known Divergence). BUG-008 is a one-line spec correction for the human pass.
