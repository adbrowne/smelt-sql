# Phase 03 summary — mark stdout sites, justify the added `expect`

**Shipped:**
- `crates/smelt-cli/tests/stdout_markers.rs` — new source-text gate, 3 tests, no warehouse
  needed. Walks the raw source lines of the target files and checks that a `// stdout:`/
  `// invariant:` comment precedes each call, tolerating multi-line `println!(...)` statements
  by anchoring on the statement's opening line rather than the literal line a substring lands on.
- `// stdout: <reason>` markers on all 10 `println!` sites in `migrate.rs` (lines 294, 307, 311,
  316, 322, 340, 351, 355, 357, 401 at pre-edit HEAD), each reason specific to that call (plan
  header, spacer, per-group technique/refusal line, `--json` dump, apply confirmation).
- `// stdout: <reason>` markers on `history.rs:37` and `status.rs:33` (the stateless-mode "nothing
  to show" branch) and on the *second* `run.rs` "no models matched the selector(s)" site (the
  `--since-upstream` intersection path at line 499); the pre-burst first occurrence at line 314 is
  left unmarked per the plan.
- `// invariant: ...` comment above `migrate.rs`'s `.expect("JSON serialization should not
  fail")`, and the identical comment mirrored onto the pre-existing `diff.rs:502` site for
  read-consistency (not required by any criterion, called out as free work in the plan).

**Decisions:**
- Anchored marker lookup on the *start* of the `println!`/`eprintln!` call rather than the line
  containing the target substring/`.expect(`, since several sites are multi-line macro
  invocations — the naive "line immediately above the matched line" check would have missed
  markers stacked above the call's opening line. This only affects the test's own robustness, not
  source layout.

**For the next planner:**
- Phase 3 changes zero hardening-budget counts by design (`smelt-cli println` 174, `expect` 42
  unchanged) — confirmed via `hardening-budget.sh` and an empty `git diff --stat` on the baseline
  file. Phase 4 still owns the full regeneration, the criterion-1 `println` clause restatement,
  and the `docs/TODO.md` file-split deferral note — none of that was touched here.
- No new follow-up work surfaced; the phase was mechanical once the census (phase 1) had already
  resolved the `rebuild.rs`-is-out-of-scope question.

**Gates:**
- `cargo test -p smelt-cli --test stdout_markers --quiet` — 3 passed (red before edits, green
  after).
- `bash .claude/scripts/hardening-budget.sh` — OK; `git diff --stat .claude/hardening-baseline.txt`
  empty.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test, example_diagnostics).
