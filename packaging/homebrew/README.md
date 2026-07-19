# Homebrew packaging

`Formula/smelt.rb` installs the standalone `smelt` + `smelt-lsp` +
`smelt-datagen` binaries built by `.github/workflows/release.yml`'s
"Standalone" job, for macOS Apple Silicon and both Linux architectures
(Windows and Intel Mac are not covered — Intel Mac users install via
`pip install smelt-sql`, which builds from the sdist).

## Tap layout: in-repo vs dedicated tap repo

Homebrew requires formulas to live in a repo named `homebrew-<tap>` for
`brew tap <user>/<tap>` to work without a full `--formula` URL. Two options:

1. **Dedicated tap repo** (recommended): create `adbrowne/homebrew-smelt`
   containing just `Formula/smelt.rb`, generated/copied from this file on
   each release. Standard Homebrew UX: `brew tap adbrowne/smelt && brew
   install smelt`.
2. **In-repo only**: keep the formula here and document
   `brew install --formula https://raw.githubusercontent.com/adbrowne/smelt-sql/main/packaging/homebrew/Formula/smelt.rb`.
   Works without a second repo but is a worse install experience and doesn't
   support `brew upgrade` cleanly.

Recommendation: dedicated tap repo, generated from this file. Creating and
pushing `adbrowne/homebrew-smelt` is human-gated (external repo, outside
this codebase) — not done as part of this change.

## Bootstrap (once the tap repo exists)

```bash
brew tap adbrowne/smelt
brew install smelt
```

## Per-release update

After a tag's `release.yml` run completes and the GitHub release assets are
published:

```bash
scripts/update-homebrew-formula.sh X.Y.Z
```

This fetches the three standalone tarballs (`smelt-macos-aarch64.tar.gz`,
`smelt-linux-x86_64.tar.gz`, `smelt-linux-aarch64.tar.gz`) from the matching
GitHub release, computes their sha256, and rewrites `Formula/smelt.rb` in
place. Copy the updated formula into the `homebrew-smelt` tap repo and push
it there — this script only updates the in-repo copy.

## Local formula check

```bash
brew ruby -c packaging/homebrew/Formula/smelt.rb   # if brew is available
ruby -c packaging/homebrew/Formula/smelt.rb          # syntax-only fallback
```
