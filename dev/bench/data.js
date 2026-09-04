window.BENCHMARK_DATA = {
  "lastUpdate": 1788518811745,
  "repoUrl": "https://github.com/adbrowne/smelt-sql",
  "entries": {
    "Smelt Latency Benchmarks": [
      {
        "commit": {
          "author": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "committer": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "distinct": true,
          "id": "6fc35fe78e2d493d544bb28fcfcf72f236638897",
          "message": "outcome(20260904-*): scaffold six low-input outcomes from the 2026-09-04 review\n\nprogramme-hygiene, state-residency, walk-migration-residue,\ndelta-signature-front-door, decided-gap-residue, ratchet-paydown.\nBacklog reordered per docs/research/20260904-incremental-state-review.md.\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>",
          "timestamp": "2026-09-04T20:42:19+10:00",
          "tree_id": "c528693b8d5e515526c0190f2166ca4fb6dec19d",
          "url": "https://github.com/adbrowne/smelt-sql/commit/6fc35fe78e2d493d544bb28fcfcf72f236638897"
        },
        "date": 1788518808829,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 58.690965,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 56.391792,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.967933,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.6354839999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.370871,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1152.260977,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.348438,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.204938,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.156598,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.704573,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 953.417549,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.86663,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.84535,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.278692,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}