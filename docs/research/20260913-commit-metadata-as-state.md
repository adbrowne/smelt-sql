# Table + commit metadata is capability enough (2026-09-13)

**Thesis.** smelt's state doctrine declines every correctness structure on Spark (Delta) and
Trino (Iceberg) because those engines have no cross-table transaction. That inference is too
strong. Cross-table atomicity is not the only way to make bookkeeping atomic with the write it
describes — *putting the bookkeeping inside the write's own commit* is another, and both formats
support it natively. **A table plus its per-commit metadata is capability enough to carry
correctness state.** What that metadata should contain is the open question this paper frames
rather than settles.

**Status.** Research. Nothing here is a spec change yet. §7 lists the spec edits the thesis
implies if accepted; §9 lists what must be measured first.

## 1. The claim being challenged

`docs/specs/state.md` §"Which dialects realise which structure" records Spark's column as five
`no`s, marked permanent rather than pending, with this reason:

> Delta provides per-table atomicity and no cross-table transaction, so a ledger write and its
> data write cannot be made atomic, and the additive fold's never-fold-twice refusal has no sound
> realisation there.

`docs/outcomes/20260913-trino-ledger/outcome.md` inherits that verdict for Trino on the ruling of
2026-09-13, and accepts the cost: the additive keyed fold's never-fold-twice refusal, the
succession patch's window-forward route, `contract.deferral`, and the sidecar-dependent
key-addressed per-group route are all unsupported on those backends, each taking its specified
downgrade.

The reasoning has two steps. The first — per-table atomicity, no cross-table transaction — is
correct for both formats as commonly deployed. The second — *therefore* a ledger write and its
data write cannot be made atomic — holds only if the ledger must be a **second table**. It need
not be.

## 2. What other tools do

Four distinct strategies exist in the wild. smelt currently uses the fourth exclusively.

### 2.1 Bookkeeping rides inside the data table's own commit

The dominant pattern, and native on both formats.

**Delta** exposes idempotent writes via `txnAppId` and `txnVersion` write options, which produce a
`SetTransaction` action *in the same atomic commit as the data files*. Before writing, Delta
builds a map of the most recent version per application from the commit log, and the write
**succeeds only if `txnVersion` exceeds the recorded value** — otherwise it is skipped. This is
functionally smelt's never-fold-twice refusal, realised with no second table, and the check
happens at commit time rather than as a separate read.

**Iceberg** carries arbitrary writer-supplied key/values in each snapshot's **summary**, committed
atomically with the snapshot. The Flink sink stores `flink.max-committed-checkpoint-id` there and
reads it back on recovery to skip already-committed checkpoints — the same mechanism, expressed as
data rather than as an engine-enforced rule.

The consequence for smelt: per-table atomicity is sufficient, provided the bookkeeping is a
*property of the write* rather than a *row in a sibling table*. `state.md` §"The residency rule"
is then satisfied more strictly than DuckDB satisfies it, not weakened — there is one commit, so
there is no window in which half of it exists.

### 2.2 Cross-table atomicity, rented from the catalog

The Iceberg REST protocol defines `POST /v1/{prefix}/transactions/commit`
(`RESTCatalog.commitTransaction` in Java), taking a list of per-table updates committed atomically.
Servers are not required to implement it; Apache Polaris maps it onto a single relational
transaction over the participating tables' metadata rows, and Nessie has offered git-style
multi-table commits from the start — Nessie's own Trino documentation presents cross-table
transactions as the headline reason to use it.

This matters for the *form* of the spec's claim more than for its content: cross-table atomicity
on Iceberg is a **catalog** property, not a format property. "Permanently no" is therefore the
wrong shape of answer for a Trino/Iceberg target — the right shape is a capability probed against
the deployed catalog, with `no` as the common result.

### 2.3 Restrict admission to re-runnable shapes

dbt keeps no correctness state at all: incremental models are merges or insert-overwrites, and a
failure is recovered by re-running the same region, which is safe because the operation is
idempotent for that region. SQLMesh keeps interval records in an external state database and gets
away with the non-transactional coupling only because its incremental models are
interval-idempotent by construction — `state.md` §Design already records this analysis.

This is smelt's downgrade-to-recompute path. It is the honest floor, and it stays the floor under
every proposal here.

### 2.4 Decline

What smelt does today on Spark, and plans to do on Trino.

## 3. The design the thesis implies: authority and projection

The strong version of §2.1 — put *everything* in the commit — fails on payload size. Commit
metadata holds small strings; observed output deltas are key sets, and tombstone ledgers are rows.
The workable version splits the two roles:

- **The authority** is a token written *inside the data table's atomic commit*. Small.
- **The projection** is an ordinary second table holding the full payload. Written separately, and
  therefore able to disagree with the data write.

The projection is not independent state. It is a *cache of a fact authoritatively recorded in the
commit log*, and every disagreement is adjudicable in a known direction:

| Data commit | Projection row | Verdict | Action |
|---|---|---|---|
| token present | row present | consistent | nothing |
| token present | row missing | the write landed | re-derive or replay the row |
| token absent | row present | the write never landed | discard the row |
| token absent | row missing | consistent | nothing |

No state is undecidable, because the authority is atomic with the data by construction. This is
the property that separates the design from a compensating transaction or a two-phase commit —
both of which `20260913-trino-ledger` §"Out of scope" rules out, correctly, as mechanisms that
make a non-atomic ledger *look* atomic. Here nothing is made to look atomic: the atomic fact is
genuinely atomic and genuinely small, and the large fact is explicitly derived from it.

The pattern is the familiar one of a log as source of truth with a materialised view over it. The
engine's commit log is the log; the ledger table is the view; reconciliation is view maintenance.

### 3.1 Reconcile before trust

The design adds one run phase: **before any derived plan reads the projection, reconcile it
against the commit log.** This is the architectural addition, and it is where the correctness
argument lives. A plan that reads an unreconciled projection has exactly the properties the
current doctrine rejects.

### 3.2 What the authority must be strong enough to do

Two strengths, and which one a structure needs depends on what its loss costs:

- **Adjudicate.** The token tells you whether a given write landed, so a projection row can be
  trusted or discarded. Sufficient where a missing row costs only *cost* — the window is re-run.
- **Reconstruct.** The payload can be rebuilt from the token plus the data. Necessary where a
  missing row costs *correctness*. Delta's change data feed and Iceberg's incremental scan between
  two snapshot ids both yield exactly what a commit changed, which is what makes observed output
  deltas and tombstones reconstructable rather than merely adjudicable.

### 3.3 The degenerate case: the commit itself is the token

On a table smelt is the **sole writer** of — which every maintained model's table is — the
authority need not be written at all. It already exists: the snapshot lineage.

Record the target's current snapshot id in the projection *before* writing. After the crash,
compare: a snapshot descending from the recorded parent means the write landed; no such snapshot
means it did not. The commit's *existence* is the token, and Iceberg gives it to any reader for
free through `$snapshots` and `$history`.

This is strictly weaker than §2.1 in one way and stronger in another. Weaker: it is two commits
with a decidable oracle, not one commit, so it depends on the sole-writer assumption and cannot
survive a foreign writer. Stronger: it requires **no write-side metadata capability whatsoever**,
which is what makes it the only variant available on Trino (§6.2).

It provides adjudication but not concurrency safety, and the distinction is sharp: two concurrent
runs both read the same parent snapshot, both fold, and Iceberg's optimistic concurrency does
**not** save them, because appends do not conflict with appends. Concurrency safety on this
variant must come from a lock, not from the format.

## 4. The open question: what the metadata should be

This is the part the thesis does not settle. Three axes, and they trade against each other.

### 4.1 Cumulative versus per-commit

The sharpest axis, because it decides whether the design depends on the user's table maintenance.

- **Per-commit tokens** (one per absorbed window) require reading the commit *history*, and
  history is pruneable: Iceberg `expire_snapshots`, Delta log retention, and — specifically for
  `SetTransaction` actions — Delta's `delta.setTransactionRetentionDuration`. Iceberg's own Flink
  documentation warns to preserve the sink's last snapshot for this reason, and an open Iceberg
  issue records silent data loss when a job restores from a savepoint older than the retained
  history. Depending on retention means depending on an operational policy smelt does not own.
- **Cumulative tokens** (a frontier high-water mark, or a compact encoding of the absorbed set)
  need only the *latest* snapshot's metadata. Immune to expiry. The cost is that the token must be
  a monotone summary — which the reconciliation frontier naturally is, and which a set of
  arbitrary absorbed windows is not, unless bounded and re-encoded on every write.

A reconciliation frontier is therefore the easiest structure to carry this way, and an
absorbed-window set the hardest. That is a useful ordering for any implementation.

### 4.2 Self-describing versus pointer

- **Self-describing** (the token *is* the window identity, or the frontier value) adjudicates with
  no projection table present, and answers "have I already folded W?" from history alone.
- **A pointer** (a write id, plus a digest of the projection row it corresponds to) is smaller and
  uniform, but means nothing without the projection, and requires the projection row to be written
  *first* so the id has a referent.

Self-describing is preferred wherever it fits, because it keeps the authority meaningful in
isolation. §3.3's snapshot-lineage variant is the pointer case taken to its limit: the token is
chosen by the engine, not by smelt, and carries no meaning at all outside the projection.

### 4.3 Enforced versus inert — and the two Delta slots

Delta offers two places to put commit metadata, with materially different semantics, and the
difference is not a spelling detail:

- **`SetTransaction`** (`txnAppId` / `txnVersion`) is **enforced by the engine at commit time**. A
  writer that loses an optimistic-concurrency race retries, re-reads the log, sees the token, and
  no-ops. That is concurrency safety obtained for free. The cost is a constrained shape: an
  application id and a **monotonically increasing** version.
- **`commitInfo.userMetadata`** is an arbitrary user-supplied string in the commit. Unconstrained
  payload, and entirely inert — nothing checks it.

Iceberg's snapshot summary behaves like the second: arbitrary, and inert.

The monotonicity constraint on `SetTransaction` is a genuine mismatch with smelt, which may absorb
an older window after a newer one during a backfill — under a naive encoding that backfill would
be **silently skipped**, the same failure mode as the Iceberg/Flink savepoint issue. One candidate
encoding sidesteps it: make the *window identity* the `txnAppId` and pin `txnVersion` to 1, so
every window is its own application and the check degenerates to set membership. Whether the
resulting unbounded set of `SetTransaction` actions is acceptable — in log size, checkpoint size,
and under `setTransactionRetentionDuration` — is unmeasured and is a blocking question for this
route.

A plausible combination worth evaluating: `SetTransaction` for enforcement, `userMetadata` for the
self-describing payload. This is a hypothesis, not a recommendation.

### 4.4 The contract difference

Delta's idempotent write **silently no-ops** on a repeat. smelt's never-fold-twice is a **loud
refusal**. These are different contracts: a silent skip cannot be distinguished after the fact
from "nothing to do", which is precisely the ambiguity the current design refuses to tolerate.
Recoverable — the commit log can be read back to confirm which case occurred — but it is a
decision to take deliberately, not a free substitution.

## 5. Per-structure applicability

Against `state.md` §"The state-structure inventory", assessed rather than measured:

| Structure | Fits the design? | Note |
|---|---|---|
| Reconciliation ledger (frontier record) | **best fit** | A monotone scalar. Cumulative, self-describing, expiry-immune. |
| Transactional merge ledger | **good fit** | The absorbed-window set; §4.1's hard case, and Delta's `SetTransaction` is nearly this feature already. |
| Observed output deltas | **fits as projection** | Payload too large for metadata, but reconstructable from CDF / incremental scan, so adjudication suffices. |
| Tombstone ledger (succession grain) | **fits as projection, or redesign** | Reconstructable from the commit's deletes. A soft-delete column on the presented table would be single-commit outright, at the cost of a schema redesign. |
| Fingerprint sidecar | **unclear — check separately** | It records digests of *source* tables, so there is no write of ours for it to be atomic with. It may not be a cross-table-atomicity problem at all, and may have been declined by association. |

## 6. Per-backend reachability

The design needs a seam that can set commit metadata, or failing that, read commit lineage. smelt
emits SQL, which is where this gets uneven — and the unevenness is the paper's most practical
finding.

### 6.1 Spark — the promising target

The backend holds a live PySpark `SparkSession` object (`crates/smelt-backend-spark/src/lib.rs:32`),
not merely a SQL string pipe, so session configuration and the DataFrame writer are reachable in
principle. Delta therefore offers the full §2.1 design including commit-time enforcement, which is
the only place concurrency safety comes for free. Unverified: no probe has been run.

### 6.2 Trino — measured, and it changes the interface question

Four facts, checked against Trino's documentation and issue tracker on 2026-09-13:

1. **The Iceberg connector is autocommit-only.** `START TRANSACTION` … `COMMIT` over the connector
   fails with `Catalog only supports writes using autocommit: iceberg`
   ([trinodb/trino#15385](https://github.com/trinodb/trino/issues/15385)). This is not merely the
   absence of *multi-table* transactions — Trino refuses multi-statement write transactions over
   Iceberg outright. `20260913-trino-ledger`'s phase-1 probe should expect this exact string.
2. **No SQL surface sets a snapshot summary property on a write.** The connector populates
   snapshot summaries itself; nothing in the SQL grammar reaches them.
3. **`extra_properties` exists but does not help.** It is a *table* property — "not used by Trino,
   and available in the `$properties` metadata table" — set by `CREATE TABLE … WITH` or a separate
   `ALTER TABLE … SET PROPERTIES`. Under autocommit that is a **separate commit** from the data
   write, so it reconstructs the very two-commit problem it would be meant to solve. It is not a
   route to §2.1.
4. **Commit lineage is fully readable.** `$snapshots` exposes `snapshot_id`, `parent_id`,
   `committed_at`, `operation` and `summary`; `$history` logs metadata changes; `$properties`
   exposes table properties.

Taken together: **Trino cannot do §2.1 at all, but can do §3.3.** The write side is closed and the
read side is wide open, which is exactly the shape that admits the snapshot-lineage variant —
record the parent snapshot id, write, adjudicate afterwards by reading `$snapshots`. That gives
crash-safety with no new interface, at the cost of the sole-writer assumption and with no
concurrency safety.

This bears directly on the concern that prompted the investigation — that the finding might change
smelt's Trino interface. It does, but less than feared, and the two candidate changes should not
be confused:

- **A catalog-side channel** (smelt holding an Iceberg REST catalog client alongside the Trino
  connection) would unlock §2.2's `commitTransaction` where the catalog supports it, and would let
  smelt write metadata Trino cannot. It is a genuine second interface, with a genuine hazard: the
  *data* write still goes through Trino and still will not carry smelt's token, so a catalog
  channel does not make §2.1 reachable. It buys §2.2 and nothing else.
- **A metadata-read channel** is not an interface change at all. `$snapshots` is an ordinary table
  read over the existing SQL connection. §3.3 needs only this.

The honest recommendation is therefore to treat §3.3 as Trino's route and defer the catalog client
until §2.2 is independently wanted. Trino's missing piece is not metadata access — it is the lock
that §3.3's concurrency gap requires, which `20260913-trino-ledger` success criterion 9 already
demands be realised or refused by name.

### 6.3 BigQuery, DuckDB

Not affected; both already realise the structures directly.

### 6.4 The finding this yields

**Commit metadata gives crash-safety wherever the commit log is readable, and concurrency-safety
only where the engine enforces a token at commit.** The two are separate capabilities with
different realisations per backend, and should be probed and claimed separately rather than
collapsed into one "ledger: yes/no" verdict.

## 7. What would have to change

If the thesis is accepted, the spec edits are bounded but real:

1. **`state.md` §"The residency rule"** stands unamended for §2.1 — one commit contains both
   facts, so "impossible by construction" holds. It does **not** stand for §3.3, which is two
   commits with a decidable oracle; admitting the Trino route means amending the rule to something
   like "atomic, or non-atomic with a total adjudication oracle and a correct fallback".
2. **`state.md` §"Corollary — the trust boundary"** needs amending either way. It grounds trust in
   *smelt being the only writer*. The engine's commit log is written by Delta or Iceberg, not by
   smelt, and is pruneable by a user running ordinary table maintenance. Making it
   correctness-bearing is a deliberate widening of the trust boundary and must be stated as one.
   §3.3 additionally promotes the sole-writer property from an incidental truth to a load-bearing
   precondition, which should be stated and ideally checked.
3. **`state.md` §"Which dialects realise which structure"** changes shape: a single per-dialect
   `yes`/`no` becomes a pair — is the structure *realisable*, and is it *concurrency-safe* — with
   Iceberg's answer depending on the deployed catalog rather than on the format.
4. **A reconcile-before-trust phase** is new normative behaviour and needs an owner spec
   (`run_state.md` is the likely home).
5. **`20260913-trino-ledger`** would be rescoped rather than abandoned: its phase-1 probe stays
   (and now has an expected error string), its schema-evolution half is untouched, and its
   locking criterion becomes load-bearing rather than incidental. Its §"Out of scope" ban on
   two-phase commit and compensating transactions should be kept and sharpened, since none of the
   designs here is either and the distinction is easy to lose.

## 8. Recommendation

Two tracks, decided separately, in this order:

1. **Spark/Delta, §2.1.** Narrow, high value, doctrinally free — it satisfies the residency rule
   as written — and restores the additive fold and its frontier where the seam already exists.
   The blocking unknown is §4.3's encoding question.
2. **Trino/Iceberg, §3.3.** Requires the residency-rule amendment and a lock, and should not be
   started until item 1 has shown the reconcile-before-trust phase works somewhere.

Restoring the *whole* correctness-structure family off DuckDB is a much larger programme than
either, and this paper does not argue for it.

## 9. Open questions

Ordered by how much they gate the rest.

1. **Is the engine's commit log something smelt is willing to make correctness-bearing?**
   Doctrinal, and the crux. Everything else is downstream.
2. **What is the metadata?** §4. Specifically: cumulative or per-commit, and if per-commit, is the
   retention dependency acceptable?
3. **Is the per-window-`txnAppId` encoding viable at scale** — log size, checkpoint size, and
   behaviour under `delta.setTransactionRetentionDuration`? Blocking for the Delta route.
4. **Is the sole-writer precondition §3.3 needs acceptable, and can it be checked** rather than
   assumed? A foreign write to a maintained table would silently invalidate the oracle.
5. **Does the fingerprint sidecar belong in this discussion?** §5 suspects not.
6. **Is a silent no-op an acceptable realisation of a loud refusal?** §4.4.
7. **Does the deployed Trino catalog support `commitTransaction`?** Only worth asking if §2.2 is
   wanted for its own sake; §3.3 does not need it.

## 10. Sources

External claims are cited rather than asserted; each was checked on 2026-09-13.

- Delta idempotent writes (`txnAppId` / `txnVersion`): <https://docs.delta.io/delta-streaming/>
- Databricks structured-streaming Delta docs:
  <https://docs.databricks.com/aws/en/structured-streaming/delta-lake>
- `SetTransaction` internals and configuration properties:
  <https://books.japila.pl/delta-lake-internals/configuration-properties/>
- Iceberg REST catalog protocol (`POST /v1/{prefix}/transactions/commit`):
  <https://iceberg.apache.org/docs/nightly/rest-protocol/>
- Iceberg multi-table transactions, catalog support:
  <https://lakeops.dev/blog/iceberg-multi-table-transactions>
- Iceberg Flink writes, snapshot summary and `flink.max-committed-checkpoint-id`:
  <https://iceberg.apache.org/docs/1.10.0/flink-writes/>
- Silent data loss restoring from an older savepoint:
  <https://github.com/apache/iceberg/issues/10892>
- Trino Iceberg autocommit-only transactions:
  <https://github.com/trinodb/trino/issues/15385>
- Trino Iceberg connector (`extra_properties`, `$snapshots`, `$history`, `$properties`, REST
  catalog support): <https://trino.io/docs/current/connector/iceberg.html>
- Nessie + Iceberg + Trino, cross-table transactions: <https://projectnessie.org/iceberg/trino/>

## 11. References

- `docs/specs/state.md` — §"Which dialects realise which structure", §"The residency rule",
  §"The degradation contract", §Design
- `docs/specs/incremental_models.md` — §"The frontier record (reconciliation ledger)",
  §"The equivalence invariant", §"The graph layer"
- `docs/specs/incremental_shapes.md` — §"The transactional frontier write (merge ledger)",
  §"The tombstone ledger (hidden state)"
- `docs/outcomes/20260913-trino-ledger/outcome.md` — the verdict this paper questions
- `docs/specs/multi_backend.md` — §"Incremental & schema evolution per backend"
