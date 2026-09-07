# scd2_succession

Worked example for the succession grain (`docs/specs/incremental_shapes.md` §"The succession
grain"; user guide: `docs-site/docs/guide/scd2-succession.md`).

`models/sources/customer_changes.yml` declares an append-only change-event stream, arriving on
`ingested_date` (a landing date) but clocked by `effective_ts` (the event's own business time) —
an arrival-partitioned source, distinct from the ordinary case where a source's partition and
event-time columns coincide.

`models/customer_history.sql` derives history rows from that stream: `LEAD(effective_ts)` over
each customer closes the previous row's validity window, and `QUALIFY NOT is_deleted` drops
delete events from the presented table while their `effective_ts` still closes out their
predecessor's interval. The model declares no `unique_key:`, `timeseries:`, or `grain:` — the
succession grain is recognised from the SQL shape, not declared.

Run `smelt explain customer_history` to see the recognised grain, identity, technique, and the
hidden tombstone ledger that tracks delete events.
