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

**D1 — the identity behind ADC.** The outcome records "plain ADC". Executed literally,
`gcloud auth application-default login` writes *your own* credentials, which reach every
GCP project you own, not just the dogfood one. Recommended refinement, same ergonomics:

```
gcloud auth application-default login --impersonate-service-account=smelt-dogfood@<PROJECT>.iam.gserviceaccount.com
```

ADC still "just works" for every client library, no key material is minted or stored, and
the effective identity is a service account holding only dataset-scoped roles on the
dogfood project. This is strictly better than both options originally offered and needs
your yes before phase 1 runs. If you decline, plain ADC stands and the IAM half of
criterion 2 is weaker — record that in the decision log rather than leaving it implied.

**D2 — `bq` may be unusable anyway.** `scripts/bigquery-provision.sh` documents that `bq`
imports pyOpenSSL and dies on some installs (`module 'lib' has no attribute 'GEN_EMAIL'`),
which is why every existing script speaks BigQuery REST over `curl` with a
`gcloud auth print-access-token` bearer. Decide whether the dogfood path follows that same
convention (recommended — one way of talking to BigQuery, already proven here) or whether
this phase additionally makes `bq` work. If REST, the permission narrowing only has to
admit `gcloud`, which is a smaller hole.

## The permission problem, and the shape of the fix

`.claude/settings.json` denies `Bash(gcloud)`, `Bash(gcloud *)`, `Bash(bq)`, `Bash(bq *)`.
**Deny beats allow**, so no `settings.local.json` entry can re-open them — the checked-in
list must change. A blanket removal would also re-open the test project, which criterion 2
forbids.

Fix: replace the blanket denies with a **PreToolUse Bash hook** that denies any
`gcloud`/`bq` invocation not scoped to the dogfood project. The repo already runs a
PreToolUse Bash hook (the `pgrep -f` guard in `.claude/settings.json`), so the mechanism
and its `permissionDecision: "deny"` JSON shape are proven here. The hook denies when:

- the command invokes `gcloud` or `bq`, **and** carries no `--project=<DOGFOOD>` /
  `--project_id=<DOGFOOD>`; or
- it references `gcloud-smelt-bq`, or sets `CLOUDSDK_CONFIG` to the test config dir; or
- it is destructive at project scope (`projects delete`, `billing` mutations,
  `iam ... delete`) — phase 1 is the only phase that needs those, and it is human-run.

These stay untouched: the `scripts/bigquery-*.sh` denials and
`Read(//home/andrew/.config/gcloud-smelt-bq/**)`.

**Honesty about what this is.** A hook matching command text is a guardrail against
mistakes, not a security boundary — a determined agent can build the string dynamically.
The actual boundary is IAM, which is why D1 matters more than the hook does. Say this in
the commit rather than implying the hook is containment.

## Tasks

Human (you), in a shell — none of this is Claude-executable before the hook lands:

1. Create the project (name it in the decision log), link billing, enable
   `bigquery.googleapis.com` and `billingbudgets.googleapis.com`.
2. Create the dataset with **no** `defaultTableExpirationMs` — the one deliberate
   departure from `bigquery-provision.sh`'s `ensure_dataset`, whose 24h expiry is fatal to
   a pipeline meant to accumulate history. Record the location.
3. Budget alert plus a documented monthly cap. `bigquery-provision.sh`'s
   `gcloud billing budgets create` invocation is the template; US$5 is the test project's
   figure and is almost certainly too low here — pick a real number and write down why.
4. Per D1: create `smelt-dogfood@…`, grant it dataset-scoped roles plus
   `roles/bigquery.jobUser`, and run the impersonating ADC login. (Or plain ADC, if you
   declined D1.)
5. Confirm the reach: one REST call listing the dataset, run from this worktree.

Claude, in the repo:

6. Add `.claude/scripts/gcloud-scope-guard.sh` implementing the rules above, with the
   dogfood project id read from one place, and register it as a PreToolUse Bash hook.
7. Narrow the four `gcloud`/`bq` deny entries in `.claude/settings.json` accordingly,
   leaving the script denials and the config-dir `Read` deny intact.
8. Record the project id, dataset, location, cap, and the D1/D2 answers in the outcome's
   decision log.

## Tests (red-green on the guard)

The guard is the only testable artifact; write `.claude/scripts/gcloud-scope-guard.test.sh`
(or a bats file if one exists) red first, one case each:

- `gcloud … --project=<DOGFOOD> …` → allowed.
- `gcloud …` with no project flag → denied.
- `gcloud … --project=smelt-bq-test-20260816 …` → denied.
- `CLOUDSDK_CONFIG=~/.config/gcloud-smelt-bq gcloud … --project=<DOGFOOD>` → denied
  (right project, wrong credential store).
- `bq query --project_id=<DOGFOOD> …` → allowed if D2 says `bq` is in scope, denied if not.
- `gcloud projects delete <DOGFOOD>` → denied.
- A command not touching `gcloud`/`bq` at all → untouched (the guard must not intercept
  ordinary Bash; a false deny here would break every other session in this worktree).

Then verify the real thing rather than trusting the unit test: attempt one denied and one
allowed command through the actual tool path and confirm the decisions match.

## Verification gate

- The guard's own test file passes.
- A live denied/allowed pair confirmed through the real hook, not simulated.
- `bash .claude/scripts/verify-phase.sh` green (this phase touches no Rust, so this is a
  regression check that the settings edit broke nothing).
- `git diff .claude/settings.json` reviewed by you before commit — this is the one file
  where a mistake widens access silently.

## Commit message

```
feat(dogfood): provision the BigQuery dogfood project and scope session access

Creates the dedicated dogfood project's dataset (no table expiry, unlike the
test project's 24h) under a budget alert, and opens a credential path scoped
to it.

.claude/settings.json's blanket Bash(gcloud *) / Bash(bq *) denies are
replaced by a PreToolUse guard that admits only invocations carrying the
dogfood project, refuses the test project's config dir, and refuses
destructive project-scoped verbs. smelt-bq-test-20260816's isolation is
unchanged: its script denials and the Read deny on its gcloud config dir
stay.

Risk accepted, stated plainly: a hook matching command text is a guardrail
against mistakes, not containment — a dynamically built command defeats it.
The real boundary is IAM, which is why the identity behind ADC is
<D1 ANSWER> rather than an owner credential.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
```
