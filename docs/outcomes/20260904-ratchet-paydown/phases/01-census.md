# Phase 01 — Census

Range: `39228307` (pre-burst, 2026-08-21 baseline write, confirmed pre-burst commit per outcome
source) `..HEAD`. Widened from the outcome's nominal `39228307..994e6f3f` per the 2026-09-06
decision log: two `smelt-cli println` sites land in `c7583387`, which is a descendant of
`994e6f3f`, and criterion 1 is a whole-tree count, so a burst-range-only census would leave it
unachievable by construction.

## 1. Baseline table

| Crate | Pattern | Pre-burst (`39228307`) | HEAD | Delta |
|---|---|---|---|---|
| `smelt-cli` | `println` | 161 | 174 | +13 |
| `smelt-cli` | `expect` | 41 | 42 | +1 |
| `smelt-db` | `unwrap` | 16 | 19 | +3 |

Both endpoints quoted directly:
- `git show 39228307:.claude/hardening-baseline.txt` → `smelt-cli expect 41`, `smelt-cli println 161`, `smelt-db unwrap 16`.
- `git show HEAD:.claude/hardening-baseline.txt` (== working tree `.claude/hardening-baseline.txt`) → `smelt-cli expect 42`, `smelt-cli println 174`, `smelt-db unwrap 19`.

All other crate/pattern rows are byte-identical between the two revisions (diffed directly; not
re-derived here). `bash .claude/scripts/hardening-budget.sh` reports OK against the current
baseline both before and after this phase (no production code touched).

## 2. Per-site census

### `smelt-db` `unwrap` (+3) — all in `crates/smelt-db/src/lib.rs`, all one commit

| Line (HEAD) | Excerpt | Commit | Verdict |
|---|---|---|---|
| 696 | `let existing = self.deployed_schemas.read().unwrap().get(&key).copied();` | `a68a8268` feat(db): surface definition-change refusals ahead of a run via a deployed-schema Salsa input | `justify` — already carries `// invariant: same RwLock poisoning rationale as set_source_file.` immediately above, copying the pre-existing rationale for the identical pattern on `self.files`/`self.projects` a few lines up in the same file (pre-burst, uncontested). No action needed beyond confirming the comment is present. |
| 713 | `self.deployed_schemas.write().unwrap().insert(key, input);` | `a68a8268` | `justify` — same rationale, same comment (shared with the line-696 site's leading comment inside the `match` arm). |
| 730 | `.unwrap()` (in `self.deployed_schemas.read().unwrap().get(...)`) | `a68a8268` | `justify` — same rationale, own leading comment (`// invariant: same RwLock poisoning rationale as set_source_file.` at line 727). |

Reconciliation: `16 (pre) + 3 (added, a68a8268) − 0 (removed) = 19 (HEAD)`. Balances.

All three sites are **already justified in the source** — the introducing commit copied the exact
comment convention used by the pre-existing `self.files`/`self.projects` RwLock unwraps in the
same impl block (lines ~528, 538 etc., pre-burst). Phase 2 has no conversion work; it only needs
to confirm the comments read correctly and are not, e.g., stale copy-paste that no longer matches
the site (they do match — same `RwLock<HashMap<...>>` field, same single-threaded Salsa mutation
context argument).

### `smelt-cli` `println` (+13)

| File:Line (HEAD) | Excerpt | Commit | Verdict |
|---|---|---|---|
| `migrate.rs:294` | `println!("smelt migrate {model_name}: applied {} statement{}...")` | `0d5cb0e6` feat(migrate): persist plan approvals and add smelt migrate --apply/--json | `stdout` |
| `migrate.rs:307` | `println!("definition delta for {model_name}: eclipsed — nothing to do")` | `1c7bffea` feat(cli): derive and print the definition-delta migration plan via smelt migrate | `stdout` |
| `migrate.rs:311` | `println!("definition delta for {model_name} ({} column group{}...")` | `1c7bffea` | `stdout` |
| `migrate.rs:316` | `println!();` (spacer in plan rendering) | `1c7bffea` | `stdout` |
| `migrate.rs:322` | `println!("plan hash: {hash}   approve and execute with: smelt migrate {model_name} --apply");` | `1c7bffea` | `stdout` |
| `migrate.rs:340` | `println!("  {label:<18}{verdict_label:<20} {:?} ({} statement{})", ...)` | `1c7bffea` | `stdout` |
| `migrate.rs:351` | `println!("  {label:<18}{verdict_label:<20} no admissible technique");` | `1c7bffea` | `stdout` |
| `migrate.rs:355` | `println!("                    refused: {}", refusal.reason);` | `1c7bffea` | `stdout` |
| `migrate.rs:357` | `println!();` (spacer) | `1c7bffea` | `stdout` |
| `migrate.rs:401` | `println!("{}", serde_json::to_string_pretty(&output).expect(...))` (`--json` output) | `0d5cb0e6` | `stdout` |
| `history.rs:37` | `println!("No run history: target '{}' is running with state.mode: stateless, which ...", args.target)` | `c7583387` feat(state-residency): honour state.mode in execute_project — per-posture .smelt/ write set | `stdout` |
| `status.rs:33` | `println!("No state directory: target '{}' is running with state.mode: stateless, which ...", args.target)` | `c7583387` | `stdout` |
| `run.rs:499` | `eprintln!("smelt: no models matched the selector(s)");` (substring `println!` inside `eprintln!`, per the gate's counting rule) | `acb5e66d` feat(propagation): intersect the --since-upstream propagated plan with --select/--exclude | `stdout` — pre-existing identical message already present at `run.rs:314` (pre-burst); this is the same user-facing "nothing matched" notice reached from the `--since-upstream` intersection path. |

Reconciliation: `161 (pre) + 13 (added, across 4 commits/4 files) − 0 (removed) = 174 (HEAD)`. Balances.

All 10 `migrate.rs` sites are the entire user-facing rendering surface of the new `smelt migrate`
command (human-readable plan/apply summaries and the `--json` structured dump) — there is no
non-output logic to route through `tracing`; this command's job is to print a plan to stdout for a
human or script to read. The `history.rs`/`status.rs` pair are a new branch of an existing
user-facing "nothing to show" message, conditioned on `state.mode == Stateless`, sitting directly
beside the pre-burst unconditional message they now wrap. The `run.rs` site is a second call site
of a pre-existing verbatim error string reached via a new selector-intersection path. None of the
13 are diagnostic/progress chatter that belongs behind `RunReporter`/`tracing`.

### `smelt-cli` `expect` (net +1)

| File:Line (HEAD) | Excerpt | Commit | Verdict |
|---|---|---|---|
| `migrate.rs:403` | `serde_json::to_string_pretty(&output).expect("JSON serialization should not fail")` | `0d5cb0e6` | `justify` — identical pattern to the pre-existing `diff.rs:502` (`serde_json::to_string_pretty(&output).expect("JSON serialization should not fail")`, present at `39228307`): a `serde_json::Value` built purely from `String`/`bool`/`Vec`/primitive fields can only fail to serialize on a non-string map key or a `NaN`/`Infinity` float, neither of which this `json!{}` literal contains. Give it the same one-line comment `diff.rs` should also carry (pre-existing, out of this outcome's added-sites scope, but phase 3 can align them for free). |

**Correction to the phase-1 recon / outcome decision log**: `commands/rebuild.rs`'s 2 `.expect(`
sites (`try_get(...).expect("workspace not initialized")` and a second `.expect("project not
initialized")`, both at what is now `rebuild.rs:99`/`:102`) are **not new**. `rebuild.rs` did not
exist at `39228307`; it was created by `5ea4c9fb` ("rename smelt backbuild to smelt rebuild") as a
pure `git mv` of `commands/backbuild.rs`, which already carried these exact two lines verbatim at
`39228307` (confirmed: `git show 39228307:crates/smelt-cli/src/commands/backbuild.rs` contains
both). The file-level diff tool sees `backbuild.rs −2 / rebuild.rs +2` because it's a rename, not
because two `expect`s were introduced. Net crate effect is zero from this pair, and success
criterion 2 ("every production `unwrap`/`expect` **added** by commits in the range") does not
reach them — they were added before the range, under a different filename. **Phase 3's scope
shrinks to the single `migrate.rs:403` site**; the `rebuild.rs` pair needs no conversion or new
justification (whether they *already* deserve one is pre-existing debt outside this outcome, per
"Out of scope" — no ratchet ever counted them as new).

Reconciliation: `41 (pre, including backbuild.rs's 2) + 1 (migrate.rs:403, added) + 0 (net from
the backbuild→rebuild rename, since both sides are the same 2 sites under different filenames) −
0 (nothing else removed) = 42 (HEAD)`. Balances.

## 3. Criterion-1 verdict

**Criterion 1's `smelt-cli println ≤ 161` is not achievable by honest means.** All 13 added sites
are `stdout` verdicts — either the entire rendering surface of a genuinely new user-facing command
(`smelt migrate`, 10 sites) or additional branches/call-sites of pre-existing user-facing messages
(3 sites). None is diagnostic chatter miscategorized as println; there is nothing left to convert
or route. The lowest honest count achievable without deleting shipped, tested CLI output is **174
— unchanged from HEAD today.**

Recommended restatement of criterion 1, for the phase-4 plan step to apply verbatim to
`outcome.md`:

> `.claude/hardening-baseline.txt` records `smelt-db unwrap` ≤ 16 (reversed to the pre-burst
> value, since all 3 additions are pre-justified duplicate-pattern sites — see phase 2) and
> `smelt-cli println` = 174 with every one of the 13 sites added since `39228307` marked
> `// stdout: <reason>` at its call site (not just tallied in this census), and no other crate's
> count rises above its `39228307` value.

This keeps the *spirit* of criterion 1 (no unreviewed, unjustified growth) while not forcing a
regression in `smelt-cli`'s actual user-facing surface. `smelt-cli expect` should land at 42
(HEAD, unchanged) once `migrate.rs:403` carries its justification comment — criterion 1 as
originally written only names `println`/`unwrap`, so `expect`'s value is unconstrained by it but
is covered by criterion 2.

## Phase ownership after this census

- **Phase 2** (`smelt-db`): confirm the 3 existing justification comments at lines 696/713/730 are
  present and accurate. No conversions. Essentially a verification-only phase now.
- **Phase 3** (`smelt-cli`): mark all 13 `println!`/`eprintln!` sites above with `// stdout:
  <reason>` (per criterion 3, as restated), and add the one justification comment to
  `migrate.rs:403` (mirroring `diff.rs:502`'s equivalent pattern, optionally adding the same
  comment there too since it's free and keeps the two sites consistent — not required by any
  criterion since `diff.rs:502` predates the range). **Drop `rebuild.rs` from phase 3's scope
  entirely** — its 2 `expect`s are pre-existing debt outside the outcome, not added sites.
- **Phase 4**: regenerate the baseline (`smelt-cli println` stays 174, `smelt-cli expect` stays
  42, `smelt-db unwrap` stays 19 — the ratchet-paydown here is entirely in *justification*, not
  in *count*, since every added site survives review as legitimate); apply the criterion-1
  restatement above to `outcome.md`; record the sign-off note in the regeneration commit message.
