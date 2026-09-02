window.BENCHMARK_DATA = {
  "lastUpdate": 1788343408566,
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
          "id": "21464dd2c2c2d0bf7cffd73363a70a6808a8f8be",
          "message": "Merge pull request #183 from adbrowne/annes-words\n\nAnnes words",
          "timestamp": "2026-09-02T19:59:15+10:00",
          "tree_id": "e2175fb23771da74706e782aceff19d108f112dc",
          "url": "https://github.com/adbrowne/smelt-sql/commit/21464dd2c2c2d0bf7cffd73363a70a6808a8f8be"
        },
        "date": 1788343403417,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 33.406218,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 31.902656,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.669018,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.444342,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.191642,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 797.496199,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 2.83073,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.899814,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.665499,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.612706,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 677.754496,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.45858,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 26.592270000000003,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 11.258612,
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
          "id": "21464dd2c2c2d0bf7cffd73363a70a6808a8f8be",
          "message": "Merge pull request #183 from adbrowne/annes-words\n\nAnnes words",
          "timestamp": "2026-09-02T19:59:15+10:00",
          "tree_id": "e2175fb23771da74706e782aceff19d108f112dc",
          "url": "https://github.com/adbrowne/smelt-sql/commit/21464dd2c2c2d0bf7cffd73363a70a6808a8f8be"
        },
        "date": 1788343407675,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 30.61887202436677,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}