# Phase 6 summary — `--markdown`, CI guide, dogfood workflow

Spec deltas Δ4-Δ6 + D2's Known Divergences bullet landed first in
`docs/specs/property_diff.md`. `MARKER`, `markdown_report`, and the promoted
`glyph`/`dimension_str`/`json_display`/`cause_str` primitives live in
`crates/smelt-logical/src/analysis/diff_render.rs`; `--markdown` on
`ExplainArgs` (`crates/smelt-cli/src/main.rs`) wires into
`crates/smelt-cli/src/commands/explain_diff.rs`'s render branch, placed
before the `--fail-on` early return.

**D1 — `examples/huge` timing.** 2000 models (1000 `.sql` + 1000 `.py`, as
`CLAUDE.md` states — my first pass counted only `*.sql` and wrongly
concluded the doc was stale; it is not), both sides derived, no edits:
**~2.4–3.0s wall clock** across five runs total (three in the original
pass, two re-confirmed after the count correction). Each run pointed
`--project-dir` at the whole `examples/huge` project, so both SQL and
Python models were derived in every timing; re-timing after the correction
changed nothing, as expected. **Verdict: acceptable, not pathological**
for 2000 models. This is a one-shot cost per workspace load (re-resolved
only on `HEAD`/ref changes per the spec), not a per-keystroke cost, so
Phase 7's editor integration is not blocked by this number. Worth watching
if the example grows further, but no action needed now.

**Tests and what they'd catch.** All 6 unit tests in `diff_render.rs` and 5
CLI tests (7–11) can each say how they fail against a broken impl — stated
inline in each test's doc comment, and two were hand-verified by sabotage:
test 10 (`markdown_body_is_printed_even_when_fail_on_exits_1`) was run
against a deliberately-reordered `explain_diff.rs` (fail_on before print)
and failed with an empty body, as R6 predicted, then reverted. Test 12
(`the_marker_literal_is_identical_in_code_docs_and_workflow`) was run
against a mutated `ci.md` marker and failed, then reverted. Tests 13/14
(doc content assertions) are straightforward "does substring X appear"
checks — I can say what content their absence would report, but they exert
less pressure than 10/12.

**Honesty per R3/D6.9 — NOT covered by cargo, observed by hand instead:**
built `smelt-cli --release --no-default-features --features duckdb` and ran
the render step's actual commands (not through `gh`, which needs a live PR
and credentials this session doesn't have) against `examples/timeseries`
and `examples/retail_analytics` at `HEAD~1` vs `HEAD`: both exited `0` with
the cleared `no models shifted` + marker body. Separately ran the same
binary against a synthetic clone of `examples/timeseries` with the
join-downgrade edit applied and confirmed the real Markdown body renders
correctly (open `<details>` for both the edited and downstream model, valid
table, marker last line) — this is the same fixture the CLI tests use, run
through the real release binary end to end. **Not run**: the `gh api`/`gh pr
comment` find-and-update pair and the fork guard — no live PR or
`GITHUB_TOKEN` available in this session. The workflow YAML's syntax was
not validated by an actions runner; only its text content (marker,
permissions, fork guard) is asserted by tests 12–14.

**Gate:** `cargo fmt --all -- --check` clean; `cargo check --workspace
--all-targets` clean; `clippy-gate.sh` clean on both feature sets;
`smelt-logical --lib` 839/839; `smelt-cli` property_diff_cli (16),
property_diff_ci_docs (3), cli_docs_coverage (3), explain_docs_freshness
(3), docs_front_door (6) — 31/31; `mkdocs build --strict` exit 0, no
warnings on the new `guide/ci.md` page. Hardening budget: unchanged, no new
`unwrap`/`expect`/`println!`.

**What Phase 7 needs:** the LSP renders the same `DiffReport` via lenses and
diagnostics, and per the spec's Surface-parity constraint must reuse
`diff_render`'s primitives (`glyph`, `dimension_str`, `json_display`,
`cause_str`, `change_line`) exactly as this phase's Markdown form does —
never re-deriving a spelling. The standing gate
`cargo test -p smelt-lsp --test property_diff_parity` should assert the
lens counts and `PropertyDowngrade` set equal the CLI JSON for the same
workspace/ref. D1's number (~2.4-3.0s for 2000 models) is not a blocker but should
inform how the editor caches the derivation across a workspace session.
