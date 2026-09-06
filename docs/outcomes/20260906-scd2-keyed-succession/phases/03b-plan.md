# Phase 3b plan — the test-file blind spot in three structural gates

## Objective

Three standing gates are red because this branch's large-file splits turned
`#[cfg(test)] mod tests { … }` *blocks* into whole *files* that no gate's
`#[cfg(test)]`-span scan can see, so test-only code is scanned as production:
`join_context_reach::every_production_join_context_new_is_tagged`,
`walk_coverage::admission_paths_have_no_raw_text_scans`, and
`hardening_budget::gate_detects_regression` (`smelt-logical` `expect` reads 14
against baseline 1; 13 of those are in `maintenance/choice/{write_variant,
region_write_variant}_tests.rs`). Fix the *file selection* in all three — one
shared rule, not a tag on any call site — so the blind spot closes for the next
split too. Advances criterion 10 ("all standing gates green").

## The rule

A file is test-only when its parent module declares it under `#[cfg(test)]`:
for `<dir>/<stem>.rs`, the parent module source (`<dir>/mod.rs`, else
`<dir>.rs`) contains `mod <stem>;` whose own line or the immediately preceding
non-blank line is `#[cfg(test)]`. Applied transitively up the directory chain
(a file inside a `#[cfg(test)]`-declared module directory is test-only too).
This is derived from the declaration, not from a `*_tests.rs` name convention,
so a differently-named split file is still classified correctly.

## Spec delta

None — no user-visible feature behaviour changes. The gates' own module docs
carry the rule.

## Tests

- `test_only_files::declared_under_cfg_test_is_test_only` — `#[cfg(test)]` on the
  line above `mod tests;` classifies `tests.rs` as test-only.
- `test_only_files::plain_mod_declaration_is_production` — a bare `mod real;`
  leaves `real.rs` production (guards against over-narrowing).
- `test_only_files::same_line_cfg_test_is_test_only` — `#[cfg(test)] mod tests;`
  on one line.
- `test_only_files::nested_under_test_only_module_is_test_only` — transitivity.
- `test_only_files::undeclared_file_is_production` — no parent declaration found
  (missing/unreadable parent module) ⇒ production, i.e. fail loud, never skip.
- `join_context_reach::gate_scans_production_walk_sources` — the scanned set
  excludes `analysis/walk/tests.rs` and still includes `analysis/walk/mod.rs`.
- `join_context_reach::every_production_join_context_new_is_tagged` — green.
- `walk_coverage::gate_scans_production_choice_sources` — the scanned set
  excludes `maintenance/choice/tests.rs` and `choice/write_suppression_tests.rs`
  and still includes `maintenance/choice/mod.rs`.
- `walk_coverage::admission_paths_have_no_raw_text_scans` — green.
- `hardening_budget::cfg_test_declared_module_files_are_not_counted` (new Test C
  in the existing fake-root harness): a fake crate with
  `src/m/mod.rs` declaring `#[cfg(test)] mod helper_tests;` and `mod real;` —
  an `.unwrap()` in `helper_tests.rs` does not count, one in `real.rs` does
  (baseline 0 ⇒ the fake tree must still fail on `real.rs`, and must pass when
  only `helper_tests.rs` has it).
- `hardening_budget::gate_detects_regression` — green on the committed tree.

## Tasks

1. Add `crates/smelt-logical/tests/support/test_only_files.rs`: a pure
   `is_test_only(repo_root, rel_path) -> bool` plus a pure
   `declared_cfg_test(parent_src: &str, stem: &str) -> bool` it is built on,
   with the five unit tests above and a module doc stating the rule. (Cargo does
   not auto-discover `tests/support/*.rs` as a target; each gate includes it via
   `#[path = "support/test_only_files.rs"] mod test_only_files;`.)
2. `join_context_reach.rs`: include the shared module; drop test-only files in
   `scanned_files`; keep the existing `cfg_test_spans` span exclusion for
   inline blocks; add the scanned-set test; note the rule in the module doc.
3. `walk_coverage.rs`: same inclusion and same file-level filter in its own
   file collection; add its scanned-set test.
4. `.claude/scripts/hardening-budget.sh`: add `_is_test_only_file()` implementing
   the same rule (awk over the parent module file, walking the directory chain),
   call it from `_count_crate` alongside the existing `tests.rs`/`tests/` skips;
   document it in the script header comment.
5. Add hardening_budget Test C.
6. Run `.claude/scripts/hardening-budget.sh`; the two-sided baseline will report
   *falls* for crates whose split test files were being over-counted. Run
   `--update` and record, in the commit message, the reviewer sign-off note the
   fail-loud discipline requires: the baseline tightens because the scan was
   over-counting test-only files, no production `unwrap`/`expect` was removed
   or reclassified. Verify `git diff .claude/hardening-baseline.txt` shows only
   decreases (and confirm `.claude/hardening-baseline.txt` is still tracked).
7. Re-run the three gates and `verify-phase.sh`; record in the summary any gate
   still red (phase 3c owns the path-drift class).

## Verification

- `cargo test -p smelt-logical --test join_context_reach --test walk_coverage`
- `cargo test -p smelt-core --test hardening_budget`
- `bash .claude/scripts/verify-phase.sh`
- `git diff --stat .claude/hardening-baseline.txt` reviewed by hand.

## Commit message

`fix(gates): classify #[cfg(test)]-declared module files as test-only in three structural gates`
