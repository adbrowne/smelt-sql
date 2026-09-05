# Continuous Integration

This page documents a GitHub Actions job that posts a property-diff comment on every pull
request: it derives each model's property profile at the PR's base commit and at the head
commit, reports every model whose profile shifted, and keeps one comment on the PR up to date
across pushes rather than stacking a new one each time. See [`smelt explain` §Property
diff](../reference/smelt-explain.md#property-diff) for the underlying command and
`docs/specs/property_diff.md` for the full normative surface.

## The job

The job is plain shell composed around two tools: the `smelt` CLI and the `gh` CLI. Its only
smelt-specific knowledge is the `<!-- smelt-property-diff -->` marker every `--markdown` body
ends with, which it uses to find a comment it posted earlier and update it in place instead of
adding a new one.

```yaml
name: property-diff

on:
  pull_request:

permissions:
  contents: read
  pull-requests: write

jobs:
  property-diff:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Build smelt
        run: cargo build --release -p smelt-cli --no-default-features --features duckdb

      - name: Render the property diff
        env:
          BASE_SHA: ${{ github.event.pull_request.base.sha }}
        run: |
          BIN=./target/release/smelt
          {
            "$BIN" explain --diff "$BASE_SHA" --markdown --project-dir path/to/your/project
          } > body.md
          cat body.md >> "$GITHUB_STEP_SUMMARY"

      - name: Post or update the PR comment
        if: github.event.pull_request.head.repo.full_name == github.repository
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          PR: ${{ github.event.pull_request.number }}
        run: |
          id=$(gh api "repos/$GITHUB_REPOSITORY/issues/comments" --paginate \
                --jq '[.[] | select(.user.login=="github-actions[bot]")
                           | select(.body | contains("<!-- smelt-property-diff -->"))] | last | .id // empty')
          if [ -n "$id" ]; then
            gh api -X PATCH "repos/$GITHUB_REPOSITORY/issues/comments/$id" -F body=@body.md
          else
            gh pr comment "$PR" --body-file body.md
          fi
```

A multi-project workspace runs the render step once per project, concatenating each project's
`body.md` before posting — `smelt explain --diff` diffs one `--project-dir` per invocation
(`docs/specs/property_diff.md` §Limitations).

## Why the job always writes the job summary

A `pull_request` event triggered from a fork gets a read-only `GITHUB_TOKEN`: both `gh` calls in
the comment step would fail with a permissions error on a fork PR. The render step therefore
always appends the body to `$GITHUB_STEP_SUMMARY`, which needs no write permission and is visible
on every PR including forks; only the comment step is guarded to run solely when the PR's head
repository is the same as the base repository. Do **not** switch this job to the
`pull_request_target` event to work around the fork restriction — that event grants a write token
to a workflow that must not check out and execute untrusted head-branch code, and `smelt build`
does execute the project's own SQL and (optionally) Python models.

## `fetch-depth: 0`

`smelt explain --diff "$BASE_SHA"` resolves `$BASE_SHA` with `git rev-parse`, which needs the
commit to actually be present in the checkout. `actions/checkout`'s default `fetch-depth: 1`
fetches only the PR's merge commit, so `$BASE_SHA` would not resolve and the job would fail with
`PropertyDiffBaselineUnavailable` (exit `2`) — a smelt-shaped error for what is really a shallow
checkout. Passing the base SHA explicitly, rather than relying on `--diff`'s own default
merge-base resolution, also avoids a subtlety of the checkout action's merge commit: its `main`
parent is not necessarily the PR's actual base commit once other commits have landed on the base
branch since the PR opened.

## Gating the build: an opt-in, not this repository's default

`smelt explain --diff --markdown` never gates the build by itself — the job above only comments.
Add `--fail-on downgrade` (or `--fail-on any`) to the render step to make a genuine property
downgrade fail the job:

```bash
"$BIN" explain --diff "$BASE_SHA" --fail-on downgrade --markdown --project-dir path/to/your/project
```

Whether this is the right default for your project is a real tradeoff. A repository whose
tracked models shift property profiles routinely as ordinary maintenance work lands (new joins,
changed grains, technique tuning) will find a default-on `--fail-on` gate goes red on legitimate
work — and a gate that goes red on legitimate work gets bypassed, which destroys the one signal
it exists to protect: an *unintended* downgrade slipping through unreviewed. This repository's
own dogfood job over its `examples/` projects runs without `--fail-on` for exactly that reason:
its build-failing assertion is narrower but real — `smelt explain --diff --markdown` must exit
`0`, so a broken renderer or an example that no longer derives a profile fails the job, while a
legitimate property shift does not. Choose `--fail-on` deliberately for your own project rather
than copying this repository's choice.
