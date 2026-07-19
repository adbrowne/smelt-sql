# Crates-publishing posture for v0.5

Date: 2026-07-20
Inputs: `crates/*/Cargo.toml` (21 workspace crates), the crates.io API (queried live for
each crate name on 2026-07-20), `docs/research/20260719-production-release-review.md`.

## Verified crates.io state

The production-release review's claim of "3/21 published" is **out of date**. A live query
against `https://crates.io/api/v1/crates/<name>` for all 21 workspace crate names found
**6 already published**, not 3:

| Crate | Published versions | Latest | Workspace version | Drift |
|---|---|---|---|---|
| `smelt-cli` | 0.1.0 (2026-01-27) | 0.1.0 | 0.3.2 | stale, 2 minors behind |
| `smelt-core` | 0.1.0 (2026-01-27) | 0.1.0 | 0.3.2 | stale, 2 minors behind |
| `smelt-runtime` | 0.1.0, 0.1.1 (2026-07-16) | 0.1.1 | 0.3.2 | stale, published 4 days before this brief |
| `smelt-parser` | 0.1.0, 0.2.0, 0.3.0, 0.3.1 | 0.3.1 | 0.3.2 | one patch behind, actively kept in sync until now |
| `smelt-types` | 0.1.0, 0.2.0, 0.3.0, 0.3.1 | 0.3.1 | 0.3.2 | one patch behind |
| `smelt-dialect` | 0.1.0, 0.2.0, 0.3.0, 0.3.1 | 0.3.1 | 0.3.2 | one patch behind |

No versions are yanked. The other 15 crates (including `smelt-logical` and
`smelt-fingerprint`, the two of the five "missing `publish`" crates that had no explicit
value) have never been published — `crates.io` returns `does not exist` for all of them.

**Important finding not in the original review:** `smelt-cli`, `smelt-core`, and
`smelt-runtime` already carry `publish = false` in their `Cargo.toml` *today*, yet each has
a live crates.io listing — `smelt-runtime` as recently as four days before this brief. This
means `publish = false` was added to those three *after* an earlier publish, not before one.
crates.io has no delete operation (only yank), so these stale 0.1.x listings are permanent;
setting `publish = false` only prevents *future* publishes of new versions, it does not
retract what's already live. Anyone who `cargo install`s `smelt-cli` or `smelt-runtime` from
crates.io today gets a build from January/mid-July that predates most of the v0.3 work.

## Recommendation

**`publish = false` on all 21 workspace crates for the v0.5 cut.** Distribution for v0.5 is
wheels (PyPI), standalone binaries, Docker, and (pending) Homebrew — not `cargo install` or
`cargo add`. None of the currently-published crates (`smelt-parser`, `smelt-types`,
`smelt-dialect`, `smelt-cli`, `smelt-core`, `smelt-runtime`) has semver commitments the
project is prepared to stand behind yet: the workspace is pre-1.0, internal crate
boundaries move as `smelt-logical` / `smelt-planner` layering settles (see CLAUDE.md
"Layered single-ownership"), and `smelt-parser`/`smelt-types`/`smelt-dialect` in particular
are exactly the crates most likely to gain a real external consumer if opened — which is a
reason to do that deliberately post-0.5, not as a byproduct of "some crates already had no
`publish` line."

This phase therefore adds `publish = false` to the five crates that had no explicit value
(`smelt-parser`, `smelt-types`, `smelt-logical`, `smelt-dialect`, `smelt-fingerprint`),
bringing all 21 crates to an explicit, uniform posture. No crate is published, unpublished,
or yanked as part of this work — those are irreversible or human-gated actions
(`cargo publish`, `cargo yank`) outside this plan's scope.

## Deferred (post-0.5, human-gated)

- **Yanking or otherwise flagging the stale `smelt-cli 0.1.0`, `smelt-core 0.1.0`, and
  `smelt-runtime 0.1.1` listings** so a `cargo install smelt-cli` doesn't silently hand out
  a nine-month-stale build. Yanking is a one-way crates.io action requiring the account that
  owns the crate; Andrew owns that decision.
- **Opening `smelt-parser`, `smelt-types`, `smelt-dialect`, `smelt-logical`,
  `smelt-fingerprint` deliberately** as a supported external dependency, with real semver
  policy, once the parser/type-system layering is stable enough to commit to. Until then
  they stay `publish = false` alongside the rest of the workspace.

## Verification

- `for c in crates/*/Cargo.toml; do rg -q '^publish' $c || echo "MISSING: $c"; done` → prints
  nothing (all 21 crates now carry an explicit `publish` line).
- `cargo metadata --format-version 1 | jq '[.packages[] | select(.publish == null)] | length'`
  → `0` (this brief's conclusion: zero crates publishable by default).
