window.BENCHMARK_DATA = {
  "lastUpdate": 1789296321240,
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
          "id": "cb7002648f64b655528acb021928125df9c75484",
          "message": "research: table + commit metadata is capability enough for correctness state\n\nChallenges state.md's inference that no cross-table transaction implies no\ncorrectness structures on Delta/Iceberg. Cross-table atomicity is not the only\nway to make bookkeeping atomic with its write — putting the bookkeeping inside\nthe write's own commit is another, and both formats support it natively (Delta\nSetTransaction, Iceberg snapshot summary).\n\nFrames the authority/projection split: a small token inside the atomic data\ncommit is the authority, a second table holds the payload as an adjudicable\nprojection. Every disagreement is decidable, so nothing is made to *look*\natomic — distinct from the two-phase commit the Trino ledger outcome rightly\nbans.\n\nTrino measured rather than assumed: the Iceberg connector is autocommit-only\n(trinodb/trino#15385), no SQL surface sets a snapshot summary, and\nextra_properties is a separate commit — so the in-commit design is unreachable\nthere. But $snapshots/$history are fully readable, which admits the degenerate\nvariant where the commit's existence is the token. That needs a lock, not a new\ninterface; the catalog-side channel buys multi-table commits only.\n\nOpen question left open deliberately: what the metadata should be (cumulative\nvs per-commit, self-describing vs pointer, enforced vs inert).\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-13T20:40:03+10:00",
          "tree_id": "7d0111bbd67c34c1bf261ba1927326995df3c55d",
          "url": "https://github.com/adbrowne/smelt-sql/commit/cb7002648f64b655528acb021928125df9c75484"
        },
        "date": 1789296315315,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 61.406391,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 58.877345,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.320933,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.6090559999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.288408,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1225.134362,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.935178,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 3.412794,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 3.081907,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.823756,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 1004.024077,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.89676,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.50268,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.91136,
            "unit": "ms"
          }
        ]
      }
    ],
    "Smelt Throughput Benchmarks": [
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
          "id": "cb7002648f64b655528acb021928125df9c75484",
          "message": "research: table + commit metadata is capability enough for correctness state\n\nChallenges state.md's inference that no cross-table transaction implies no\ncorrectness structures on Delta/Iceberg. Cross-table atomicity is not the only\nway to make bookkeeping atomic with its write — putting the bookkeeping inside\nthe write's own commit is another, and both formats support it natively (Delta\nSetTransaction, Iceberg snapshot summary).\n\nFrames the authority/projection split: a small token inside the atomic data\ncommit is the authority, a second table holds the payload as an adjudicable\nprojection. Every disagreement is decidable, so nothing is made to *look*\natomic — distinct from the two-phase commit the Trino ledger outcome rightly\nbans.\n\nTrino measured rather than assumed: the Iceberg connector is autocommit-only\n(trinodb/trino#15385), no SQL surface sets a snapshot summary, and\nextra_properties is a separate commit — so the in-commit design is unreachable\nthere. But $snapshots/$history are fully readable, which admits the degenerate\nvariant where the commit's existence is the token. That needs a lock, not a new\ninterface; the catalog-side channel buys multi-table commits only.\n\nOpen question left open deliberately: what the metadata should be (cumulative\nvs per-commit, self-describing vs pointer, enforced vs inert).\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-13T20:40:03+10:00",
          "tree_id": "7d0111bbd67c34c1bf261ba1927326995df3c55d",
          "url": "https://github.com/adbrowne/smelt-sql/commit/cb7002648f64b655528acb021928125df9c75484"
        },
        "date": 1789296320032,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.78017965173786,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}