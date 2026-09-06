# Phase 1 — Provision the dogfood project and open a scoped credential path

**Outcome:** `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`
**Serves criteria:** 1 (provisioned), 2 (reachable, deliberately)
**Driver:** human-executed, with the repo-side change made by Claude
**Spec delta:** none — no user-visible smelt behaviour changes in this phase.

## Objective

A dedicated GCP project exists, holding a dataset whose tables do **not** expire, under a
budget alert; and a session can reach that project — and only that project — with BigQuery
from this worktree. The test project `smelt-bq-test-20260816` is left exactly as it is.

## Two decisions to settle before executing

**D1 — the identity behind ADC.** Plain `gcloud auth application-default login` is the
recorded choice and is fine. One alternative, offered as a preference rather than a
correction, since it costs nothing and keeps the blast radius to one project:

```
gcloud auth application-default login --impersonate-service-account=smelt-dogfood@<PROJECT>.iam.gserviceaccount.com
```

Same ergonomics, no key material either way; the difference is only whether the effective
identity reaches your other GCP projects. Take whichever you prefer and record which in
the decision log so a later reader knows what the credential actually was.

**D2 — `bq` may be unusable anyway.** `scripts/bigquery-provision.sh` documents that `bq`
imports pyOpenSSL and dies on some installs (`module 'lib' has no attribute 'GEN_EMAIL'`),
which is why every existing script speaks BigQuery REST over `curl` with a
`gcloud auth print-access-token` bearer. Decide whether the dogfood path follows that same
convention (recommended — one way of talking to BigQuery, already proven here) or whether
this phase additionally makes `bq` work. If REST, the permission narrowing only has to
admit `gcloud`, which is a smaller hole.

Answerable cheaply once the guard admits it: `bq version` needs no credentials and either
prints a version or dies on the pyOpenSSL import. Run it as the first check of task 5.

**D3 — the SDK comes from mise, pinned, as a task.** Google Cloud SDK 580.0.0 (`gcloud`,
`bq` 2.1.36) is already present at `~/google-cloud-sdk/bin/`, off `PATH`, so this machine
needs nothing. The next one does, so the install is pinned in `mise.toml` (human decision
of 2026-09-06 — reproducibility beats reusing the ad hoc path).

It goes in as a **task plus an env pin, not a `[tools]` entry.** Every workflow runs
`jdx/mise-action@v2` with no arguments, which installs everything in `[tools]`; a `gcloud`
pin there would pull the ~200MB SDK into all seven CI jobs, none of which use it. The repo
already has the precedent for a heavy, non-universal dependency — `[tasks.setup-duckdb]`
with `scripts/mise-setup-duckdb.sh`, and a computed `[env]` var so consumers need no path
knowledge. Mirror it exactly:

- `[tasks.setup-gcloud]` → `scripts/mise-setup-gcloud.sh`, installing **580.0.0** (match
  what is on this box so it is a no-op here), idempotent, skipping if already present.
- an `[env]` entry resolving the SDK's `bin` — preferring a `PATH` hit, falling back to
  `~/google-cloud-sdk/bin` — so an existing install is adopted rather than duplicated,
  the way `scripts/mise-duckdb-lib-dir.sh` prefers `/usr/local/lib`.
- a `CLAUDE.md` line under the mise setup block, next to `mise run setup-duckdb`.

The existing scripts' own fallback
(`GCLOUD="$(command -v gcloud || echo "$HOME/google-cloud-sdk/bin/gcloud")"`) keeps
working either way and is not touched.

## The permission change

`.claude/settings.json` denies `Bash(gcloud)`, `Bash(gcloud *)`, `Bash(bq)`, `Bash(bq *)`.
**Deny beats allow**, so no `settings.local.json` entry can re-open them — the checked-in
list must change.

Remove those four entries. Session access to the dogfood project is deliberate (human
decision of 2026-09-06), so there is nothing left for a command-scoping hook to add: an
earlier draft of this plan proposed a PreToolUse guard admitting only project-scoped
invocations, and it is dropped. Text matching over a command line was never containment —
a dynamically built string defeats it — so it would have bought ceremony rather than
safety.

These two stay, and they are the parts that are real boundaries rather than pattern
matching:

- `Read(//home/andrew/.config/gcloud-smelt-bq/**)` — the test project's credentials live
  in a separate `CLOUDSDK_CONFIG`, so isolation rests on not being able to read that
  directory, which is enforced rather than inferred from command text.
- the `scripts/bigquery-*.sh` denials — those scripts self-target the test project.

Worth knowing while editing the list: the deny patterns match the **start** of the command
string, so `~/google-cloud-sdk/bin/bq …` — the SDK's real location on this box — was never
matched by `Bash(bq *)` in the first place. It is not a hole worth patching now, but it
does mean the old denies were narrower in practice than they read, which is a good reason
not to reintroduce the same shape elsewhere.

## Tasks

Human (you), in a shell — task 8 lands first, after which the rest is Claude-executable
too; these are listed as yours because they spend money and create identities:

1. Create the project (name it in the decision log), link billing, enable
   `bigquery.googleapis.com` and `billingbudgets.googleapis.com`.
2. Create the dataset with **no** `defaultTableExpirationMs` — the one deliberate
   departure from `bigquery-provision.sh`'s `ensure_dataset`, whose 24h expiry is fatal to
   a pipeline meant to accumulate history. Record the location.
3. Budget alert plus a documented monthly cap. `bigquery-provision.sh`'s
   `gcloud billing budgets create` invocation is the template; US$5 is the test project's
   figure and is almost certainly too low here — pick a real number and write down why.
4. Per D1: run the ADC login, impersonating `smelt-dogfood@…` or not, as you prefer.
5. Set the default / quota project so ordinary invocations need no flag.

Claude, in the repo:

6. Per D3: `scripts/mise-setup-gcloud.sh` + `[tasks.setup-gcloud]` and the `[env]` bin
   resolution in `mise.toml`, mirroring the DuckDB pair; a `CLAUDE.md` line beside
   `mise run setup-duckdb`. Verify it is a no-op on this box (SDK 580.0.0 already present)
   before trusting it on a clean one.
7. Remove the four `gcloud`/`bq` deny entries from `.claude/settings.json`, leaving the
   `scripts/bigquery-*.sh` denials and the config-dir `Read` deny intact.
8. Confirm the reach end to end, from this worktree.
9. Record the project id, dataset, location, cap, and the D1/D2/D3 answers in the
   outcome's decision log.

## Tests

Nothing here is a testable unit: the phase provisions cloud resources and deletes four
lines of configuration. Verification is by observation, and the phase is done only when
each observation is real rather than inferred from a create call having succeeded.

## Verification gate

- `mise run setup-gcloud` is a no-op on this box, and its script is plain enough that a
  clean machine's behaviour is evident from reading it. A genuinely clean-machine run is
  not reproducible here — say so rather than implying it was tested.
- `bq version` runs. This is also D2's answer, whichever way it goes: a version string, or
  the pyOpenSSL import failure that sends the dogfood path to REST like every other script
  in this repo.
- The dogfood dataset lists, under the credential D1 chose.
- The dataset's `defaultTableExpirationMs` is confirmed **absent**, read back from the API.
  This is the one detail whose silent failure destroys the pipeline's history a day later,
  and a successful create call is not evidence that it is unset.
- The budget alert exists and names the dogfood project.
- `bash .claude/scripts/verify-phase.sh` green — no Rust changes here, so this only checks
  that the `mise.toml` edit broke no other tooling.

## Commit message

```
feat(dogfood): provision the BigQuery dogfood project and open session access

Creates the dedicated dogfood project's dataset (no table expiry, unlike the
test project's 24h) under a budget alert, and opens a credential path to it.

.claude/settings.json's blanket Bash(gcloud *) / Bash(bq *) denies are
removed — session access to the dogfood project is deliberate.
smelt-bq-test-20260816's isolation is unchanged and rests where it actually
holds: its credentials live in a separate CLOUDSDK_CONFIG that stays
Read-denied, and its scripts stay denied. No command-text guard is added;
matching command strings was never containment, and IAM is the real
boundary.

The Cloud SDK is pinned via mise (task + env, not [tools], so the ~200MB
download stays out of seven CI jobs that never use it), mirroring the
existing setup-duckdb pair.

Verified rather than assumed: the dataset's defaultTableExpirationMs is
absent when read back from the API — a successful create call does not prove
it, and a wrong value here silently destroys the pipeline's history a day
later.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
```
