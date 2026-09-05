# Phase 03 plan — `smelt-cli`: mark the added stdout sites and justify `migrate.rs:403`

## Objective

Give every `println!`/`eprintln!` site `smelt-cli` gained since `39228307` an in-source
`// stdout: <reason>` marker, and give `migrate.rs:403`'s `.expect(` an in-source infallibility
justification. This is the phase that turns phase 1's census verdicts into reviewable facts at the
call sites themselves, satisfying success criteria 2 (the one remaining added `expect`) and 3 (all
13 added stdout sites). Comment-only in production code — no behaviour change.

## Spec delta

None. No user-visible behaviour changes; the `// stdout:` / `// invariant:` markers are source
conventions already used elsewhere in the tree (`smelt-db`'s `// invariant:` RwLock comments), not
spec surface.

## Tests

New file `crates/smelt-cli/tests/stdout_markers.rs` (source-text gate, no warehouse needed). Write
it first; all three fail until the markers land.

- `migrate_command_stdout_sites_are_marked` — every `println!` in
  `crates/smelt-cli/src/commands/migrate.rs` (a wholly post-`39228307` file, so all 10 sites are
  added sites) is immediately preceded by a comment line starting `// stdout:`. Fails naming each
  unmarked line number.
- `state_mode_and_selector_stdout_sites_are_marked` — the three added sites in files that also hold
  pre-burst printlns are located by message substring, not line number, and each carries a
  `// stdout:` marker: `history.rs` + `status.rs` (`state.mode: stateless`) and the *second*
  `run.rs` occurrence of `smelt: no models matched the selector(s)` (the `--since-upstream`
  intersection path; the pre-burst first occurrence is deliberately not required to carry one).
- `migrate_json_expect_is_justified` — the `.expect("JSON serialization should not fail")` in
  `migrate.rs` is preceded by an `// invariant:` comment line.

## Tasks

1. Write `crates/smelt-cli/tests/stdout_markers.rs` with the three tests above; confirm all three
   fail (red).
2. Add `// stdout: <reason>` above each of the 10 `println!` sites in `migrate.rs` (lines 294, 307,
   311, 316, 322, 340, 351, 355, 357, 401 at HEAD). Reasons are per-site and specific — plan
   rendering for a human, `--json` structured dump for a script, spacer in the rendered plan — not
   a repeated boilerplate string.
3. Add `// stdout: <reason>` above `history.rs:37` and `status.rs:33` (the `state.mode: stateless`
   branch of a pre-existing user-facing "nothing to show" notice).
4. Add `// stdout: <reason>` above `run.rs:499` (second call site of the pre-existing
   "no models matched" notice, reached via the `--since-upstream`/selector intersection).
5. Add `// invariant: ...` above `migrate.rs:403` explaining why serialization cannot fail (the
   `json!` value is built only from `String`/`bool`/`Vec`/integer fields — no non-string map key,
   no `NaN`/`Infinity` float). Mirror the same comment onto the identical pre-existing
   `diff.rs:502` site so the two read consistently (free, not required by any criterion).
6. Re-run the three tests (green). Confirm `hardening-budget.sh` still reports OK with no baseline
   edit — this phase changes zero counts (`smelt-cli println` 174, `expect` 42 unchanged); phase 4
   owns the regeneration and the criterion-1 restatement.

## Verification

- `cargo test -p smelt-cli --test stdout_markers --quiet` — 3 passed.
- `bash .claude/scripts/hardening-budget.sh` — OK, and `git diff --stat .claude/hardening-baseline.txt`
  empty (no count moved).
- `bash .claude/scripts/verify-phase.sh` — all green.

## Commit message

`docs(cli): justify every stdout site and expect added since the burst — ratchet sign-off: 20260904-ratchet-paydown phase 3`
