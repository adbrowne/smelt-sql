# Phase 7 — Open a no-expiry dogfood dataset and a scoped credential path

**Outcome:** `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`
**Serves criteria:** 1 (provisioned), 2 (reachable, deliberately)
**Driver:** human-executed, with the repo-side change made by Claude
**Spec delta:** none — no user-visible smelt behaviour changes in this phase.

## Objective

A dataset whose tables do **not** expire exists under a budget alert, and a session can
reach it — and no project other than the one holding it — with BigQuery from this
worktree.

## Two decisions to settle before executing

**D0 — resolved 2026-09-09 (human): reuse `smelt-bq-test-20260816`; the dogfood dataset is
new, the project is not.**

The original criterion 1 named a dedicated project and forbade this one *by name*. That
was the right default, but it over-fitted: the property the outcome actually needs is **a
dataset whose tables do not expire**, and `defaultTableExpirationMs` is a dataset
property, not a project one. `smelt_test`'s 24h expiry — fatal to a pipeline meant to
accumulate history — is escaped by adding a second dataset, not a second project.

What reuse gives up, recorded so nobody later mistakes it for an oversight:

- **Cost attribution.** Dogfood scans and test-suite scans land on one bill, so phase 10's
  cost-per-run must come from each load job's own `totalBytesProcessed` rather than the
  project total. This is the more honest measurement anyway.
- **Teardown.** There is no delete-the-project escape hatch; cleanup means dropping a
  dataset.
- **The tight cap.** US$5 was a real guardrail on a project that only ever ran short test
  suites. US$25 (D0's figure, below) is the number the dogfood pipeline needs, and raising
  it necessarily loosens the guardrail on the test suites sharing the project.
- **Session reachability now extends to `smelt_test`.** `roles/bigquery.jobUser` is
  project-scoped and cannot be narrowed to one dataset, so a session that can run jobs
  against `smelt_dogfood` can run them against `smelt_test` too. That dataset holds only
  ephemeral, self-deleting test tables, which is why the trade is acceptable — but it *is*
  a change to the "a Claude session cannot reach GCP at all" posture that
  `docs/research/20260816-bigquery-backend.md` established, and it is made deliberately.

What reuse does **not** give up — the part of the original design that carried the actual
security property. The blast radius that mattered was ADC carrying Andrew's whole Google
Cloud identity across every project. A `smelt-dogfood@` service account holding roles on
this one project still refuses everything else, so criterion 2's demonstration is
unchanged. The test project's key material stays exactly as isolated as it was: the
gpg-encrypted key, the separate `CLOUDSDK_CONFIG`, and the `Read` deny on
`~/.config/gcloud-smelt-bq/**` are untouched by anything in this phase.

Concretely: project `smelt-bq-test-20260816`, **new** dataset `smelt_dogfood`, location
`US` (matching `smelt_test`, and required for querying `githubarchive`, which is US-only),
budget cap **US$25/month**.

**D1 — resolved 2026-09-06, unchanged by D0: impersonation, scoped to one project.**

```
gcloud auth application-default login --impersonate-service-account=smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com
```

The effective identity is a service account holding roles only on this project, so nothing
this pipeline (or a session driving it) does can reach your other GCP projects. ADC still
works for every client library and no key material is minted or stored.

Two consequences for the tasks below. The service account must exist and be granted
**before** the ADC login, so task 4 splits in two. And impersonation needs
`roles/iam.serviceAccountTokenCreator` for your human account on that service account —
easy to miss, and its absence shows up as a confusing `PERMISSION_DENIED` at login rather
than a message about impersonation.

Grants, following `bigquery-provision.sh`'s reasoning: `roles/bigquery.jobUser` at project
scope (run jobs), plus `WRITER` on `smelt_dogfood` specifically. Deliberately **not**
`roles/bigquery.user` — the test suites need it because they create a dataset per run,
whereas the dogfood pipeline writes to one long-lived dataset and never creates its own.
Withholding `bigquery.datasets.create` costs nothing here and means an accident cannot
scatter datasets across the project.

Note that the existing `smelt-bq-test@` service account is a *different* identity and is
left completely alone. Two service accounts on one project is the point: the token-minting
path stays as locked down as it was, and the dogfood path is reachable.

**D2 — `bq` may be unusable anyway.** `scripts/bigquery-provision.sh` documents that `bq`
imports pyOpenSSL and dies on some installs (`module 'lib' has no attribute 'GEN_EMAIL'`),
which is why every existing script speaks BigQuery REST over `curl` with a
`gcloud auth print-access-token` bearer. Decide whether the dogfood path follows that same
convention (recommended — one way of talking to BigQuery, already proven here) or whether
this phase additionally makes `bq` work. If REST, the permission narrowing only has to
admit `gcloud`, which is a smaller hole.

Answerable cheaply: `bq version` needs no credentials and either prints a version or dies
on the pyOpenSSL import. Run it as the first check of task 5.

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

## The table-expiration trap

`scripts/bigquery-env.sh:31` sets `SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS` **unconditionally**,
defaulting to two hours, and `python/smelt/bigquery_adapter.py:152` stamps whatever it
finds there onto the dataset it is about to create. A dogfood run must source
`bigquery-env.sh` for `PYTHONPATH`, so on the face of it every dogfood table would expire
two hours after it was written — silently destroying the history this whole outcome exists
to accumulate.

**How bad it actually is, stated accurately rather than dramatically.** The adapter calls
`create_dataset(dataset_ref, exists_ok=True)`, and that call does not modify a dataset that
already exists — it returns it. So once task 2 has created `smelt_dogfood` with no default
expiration, the adapter's create is a no-op and the env var never reaches the dataset. The
trap therefore bites in one specific circumstance: a dogfood run pointed at a dataset that
does **not** yet exist, which the adapter would then create *with* the 2h default. That is
not the steady state, but it is exactly what happens the first time someone points the
pipeline at a fresh dataset name.

That mitigation is real but load-bearing and undocumented, which is not a safe place to
leave the one failure that destroys data a day later and looks like nothing at the time.
So this phase does both: an explicit unset on the dogfood path (task 6a), and a read-back
of the live dataset's `defaultTableExpirationMs` in the verification gate — the create call
succeeding is not evidence that the field is absent.

## The permission change

`.claude/settings.json` was expected to deny `Bash(gcloud)`, `Bash(gcloud *)`, `Bash(bq)`
and `Bash(bq *)`. **Checked in this worktree on 2026-09-08 and again on 2026-09-09: those
four entries are not present.** Only the `scripts/bigquery-*.sh` self-target denials and
the `Read(//home/andrew/.config/gcloud-smelt-bq/**)` deny are. So task 7 is a
**verification, not a removal** — confirm the list still reads that way and change nothing.

Both surviving denials stay, and they are the parts that are real boundaries rather than
pattern matching:

- `Read(//home/andrew/.config/gcloud-smelt-bq/**)` — the test project's credentials live
  in a separate `CLOUDSDK_CONFIG`, so that isolation rests on not being able to read the
  directory, which is enforced rather than inferred from command text. **Reuse does not
  weaken this**: the dogfood path uses ADC in the ordinary `~/.config/gcloud`, and the
  token-minting path stays unreachable.
- the `scripts/bigquery-*.sh` denials — those scripts self-target the test project's
  credential flow.

No command-scoping hook is added. An earlier draft proposed a PreToolUse guard admitting
only project-scoped invocations, and it is dropped: text matching over a command line was
never containment — a dynamically built string defeats it — so it would have bought
ceremony rather than safety. Worth knowing while reading the list: deny patterns match the
**start** of the command string, so `~/google-cloud-sdk/bin/bq …` — the SDK's real
location on this box — would never have been matched by `Bash(bq *)` anyway.

## Tasks

Human (you), in a shell — these spend money and create identities:

1. **Confirm rather than create.** The project and its billing link already exist.
   Confirm `bigquery.googleapis.com` is enabled (it is — the test suites run) and enable
   `billingbudgets.googleapis.com` if it is not. No project creation, no billing link.
2. Create dataset `smelt_dogfood` in `US` with **no** `defaultTableExpirationMs` — the one
   deliberate departure from `bigquery-provision.sh`'s `ensure_dataset`, whose 24h expiry
   is fatal to a pipeline meant to accumulate history. Leave `smelt_test` exactly as it is,
   24h expiry included.
3. Raise the project's budget to **US$25/month** with an alert. `bigquery-provision.sh`'s
   `gcloud billing budgets create` invocation is the template; this replaces the existing
   US$5 budget rather than adding a second one. Note in the decision log that the test
   suites now sit under the looser cap too — that is D0's accepted cost.
4. Create `smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`; grant it
   `roles/bigquery.jobUser` at project scope and `WRITER` on `smelt_dogfood` (the ACL
   read-modify-write in `bigquery-provision.sh` is the template — `PATCH` replaces the
   `access` array wholesale, so existing entries, **including `smelt-bq-test@`'s**, must be
   carried forward). Grant your own account `roles/iam.serviceAccountTokenCreator` on it.
5. Run the impersonating ADC login from D1, then set the default / quota project so
   ordinary invocations need no flag.

Claude, in the repo:

6. Per D3: `scripts/mise-setup-gcloud.sh` + `[tasks.setup-gcloud]` and the `[env]` bin
   resolution in `mise.toml`, mirroring the DuckDB pair; a `CLAUDE.md` line beside
   `mise run setup-duckdb`. Verify it is a no-op on this box (SDK 580.0.0 already present)
   before trusting it on a clean one.
6a. Close the table-expiration trap: a dogfood env entry point that sources
   `bigquery-env.sh` (for `PYTHONPATH` and the client venv) and then explicitly
   `unset SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS`, with a comment naming
   `bigquery_adapter.py:152` and saying why. The dogfood pipeline uses that entry point;
   nothing about the test path changes, since its 2h default is a feature there.
7. **Verify** `.claude/settings.json` still carries only the `scripts/bigquery-*.sh`
   denials and the config-dir `Read` deny, and that no blanket `gcloud`/`bq` deny has
   reappeared. Change nothing if so; record what the list actually contains.
8. Confirm the reach end to end, from this worktree.
9. Record the dataset, location, cap, the D0 trade-offs actually accepted, and the
   D1/D2/D3 answers in the outcome's decision log.

## Tests

Nothing here is a testable unit: the phase provisions cloud resources and adds an env
guard. Verification is by observation, and the phase is done only when each observation is
real rather than inferred from a create call having succeeded.

## Verification gate

- `mise run setup-gcloud` is a no-op on this box, and its script is plain enough that a
  clean machine's behaviour is evident from reading it. A genuinely clean-machine run is
  not reproducible here — say so rather than implying it was tested.
- `bq version` runs. This is also D2's answer, whichever way it goes: a version string, or
  the pyOpenSSL import failure that sends the dogfood path to REST like every other script
  in this repo.
- `smelt_dogfood` lists, under the impersonated credential.
- The impersonation is real, not nominal: `gcloud auth application-default print-access-token`
  resolves to the service account, and a call touching a **different project of yours** is
  refused. An impersonated login that silently fell back to your own identity would pass
  every other check on this list, so this is the one that proves D1 actually took effect.
  Note that under D0 this check is specifically *cross-project* — a call against
  `smelt_test` is expected to **succeed**, and that expected success is the reachability
  cost D0 accepted, not a failure of scoping.
- `smelt_dogfood`'s `defaultTableExpirationMs` is confirmed **absent**, read back from the
  API. This is the one detail whose silent failure destroys the pipeline's history a day
  later, and a successful create call is not evidence that it is unset.
- The dogfood env entry point leaves `SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS` unset:
  `bash -c '. scripts/<dogfood-env>.sh; echo "${SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS-UNSET}"'`
  prints `UNSET`, while the plain `bigquery-env.sh` still prints `7200000` for the test path.
- The budget alert exists, names this project, and reads US$25.
- `bash .claude/scripts/verify-phase.sh` green — no Rust changes here, so this only checks
  that the `mise.toml` edit broke no other tooling.

## Commit message

```
feat(dogfood): open a no-expiry dogfood dataset and a scoped credential path

Adds dataset smelt_dogfood (no table expiry, unlike smelt_test's 24h) to the
existing smelt-bq-test-20260816 project under a US$25/month budget, and opens
a credential path to it.

Reuses the project rather than provisioning a dedicated one (D0, human
decision of 2026-09-09): the property the pipeline needs is a dataset whose
tables do not expire, and defaultTableExpirationMs is a dataset property. The
accepted costs are a shared bill (so phase 10 measures cost per run from each
job's own totalBytesProcessed), no delete-the-project teardown, a looser cap
over the test suites, and session reachability extending to smelt_test —
bigquery.jobUser is project-scoped and cannot be narrowed to one dataset.

What reuse does not cost is the property that mattered: the credential is a
dogfood-scoped service account reached by ADC impersonation, not an owner
identity, so the pipeline and any session driving it reach this project and
nothing else. No key material is minted. smelt-bq-test@'s own isolation is
untouched — its key stays gpg-encrypted in a separate CLOUDSDK_CONFIG that
stays Read-denied, and its scripts stay denied.

The blanket Bash(gcloud *) / Bash(bq *) denies the earlier plan expected to
remove turned out never to be present in this worktree; the list is verified
rather than edited.

Closes the table-expiration trap: bigquery-env.sh sets
SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS unconditionally and
bigquery_adapter.py:152 stamps it onto any dataset it creates. An existing
dataset is not modified by create_dataset(exists_ok=True), so this only bites
a dataset the adapter creates itself — but that failure destroys history a day
later and looks like nothing at the time, so the dogfood path unsets it
explicitly and the gate reads the live value back.

The Cloud SDK is pinned via mise (task + env, not [tools], so the ~200MB
download stays out of seven CI jobs that never use it), mirroring the
existing setup-duckdb pair.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
```
