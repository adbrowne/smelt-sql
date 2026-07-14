window.BENCHMARK_DATA = {
  "lastUpdate": 1784021336828,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "2be3033b072fb5c2117b94e7e8092d365334abe0",
          "message": "Merge pull request #160 from adbrowne/web-analytics-tutorial",
          "timestamp": "2026-07-14T19:24:44+10:00",
          "tree_id": "15b0a9eff84b5272428b690b90320f44c4f6c13e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/2be3033b072fb5c2117b94e7e8092d365334abe0"
        },
        "date": 1784021331993,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.276031,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.807465,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.036002,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.648535,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.375564,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 930.144885,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.37103,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.819406,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.595036,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.690995,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 767.312961,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.82579,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.787150000000004,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.900651,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "2be3033b072fb5c2117b94e7e8092d365334abe0",
          "message": "Merge pull request #160 from adbrowne/web-analytics-tutorial",
          "timestamp": "2026-07-14T19:24:44+10:00",
          "tree_id": "15b0a9eff84b5272428b690b90320f44c4f6c13e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/2be3033b072fb5c2117b94e7e8092d365334abe0"
        },
        "date": 1784021335799,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.799270192453573,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}