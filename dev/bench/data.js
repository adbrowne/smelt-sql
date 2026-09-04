window.BENCHMARK_DATA = {
  "lastUpdate": 1788519520008,
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
          "id": "c38c504c033a5075e65bf2d362d25fb91257a1c2",
          "message": "settings: enable ralph-loop plugin\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-04T20:52:49+10:00",
          "tree_id": "868721e05b62015d20b4337ddcf7c959afc57bd8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/c38c504c033a5075e65bf2d362d25fb91257a1c2"
        },
        "date": 1788519514127,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 58.497262,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 56.228316,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.963944,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.611855,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.395771,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1142.27113,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.285592,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.187422,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.153409,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.656381,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 951.791919,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.121479999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.794900000000005,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.2258,
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
          "id": "c38c504c033a5075e65bf2d362d25fb91257a1c2",
          "message": "settings: enable ralph-loop plugin\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-04T20:52:49+10:00",
          "tree_id": "868721e05b62015d20b4337ddcf7c959afc57bd8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/c38c504c033a5075e65bf2d362d25fb91257a1c2"
        },
        "date": 1788519518794,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.232450899070702,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}