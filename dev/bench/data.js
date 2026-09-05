window.BENCHMARK_DATA = {
  "lastUpdate": 1788590889893,
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
          "id": "8b2993bb8b8237cf0871958670a55235f8beb884",
          "message": "outcome(20260904-decided-gap-residue): record human decision, unblock phase 3\n\nResolves the once-write FD-requirement block: classify_once_write will skip\nthe declared-FD check when the candidate is already a unique_key member\n(option c), rather than widening SourceRecipe/KeyedRecipe (a) or relaxing\nFD self-contradiction validation generally (b).",
          "timestamp": "2026-09-05T16:38:41+10:00",
          "tree_id": "df944d2a69570932fa8c067fc01c3aeb3fad48e2",
          "url": "https://github.com/adbrowne/smelt-sql/commit/8b2993bb8b8237cf0871958670a55235f8beb884"
        },
        "date": 1788590884718,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.359804,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 58.116815,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.984931,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.612769,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.34324699999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1183.900961,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.228092,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.112908,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.1320639999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.6845920000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 984.307451,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.849880000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.46261,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.721162,
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
          "id": "8b2993bb8b8237cf0871958670a55235f8beb884",
          "message": "outcome(20260904-decided-gap-residue): record human decision, unblock phase 3\n\nResolves the once-write FD-requirement block: classify_once_write will skip\nthe declared-FD check when the candidate is already a unique_key member\n(option c), rather than widening SourceRecipe/KeyedRecipe (a) or relaxing\nFD self-contradiction validation generally (b).",
          "timestamp": "2026-09-05T16:38:41+10:00",
          "tree_id": "df944d2a69570932fa8c067fc01c3aeb3fad48e2",
          "url": "https://github.com/adbrowne/smelt-sql/commit/8b2993bb8b8237cf0871958670a55235f8beb884"
        },
        "date": 1788590888888,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.123673927907856,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}