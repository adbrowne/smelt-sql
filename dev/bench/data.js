window.BENCHMARK_DATA = {
  "lastUpdate": 1788775586046,
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
          "id": "af60af612e0aafd34c043ed4c6e571b7fd029da5",
          "message": "Merge pull request #186 from adbrowne/outcome-loop-20260904-programme-hygiene\n\noutcome: 2026-09-04 programme hygiene + backlog",
          "timestamp": "2026-09-07T20:02:38+10:00",
          "tree_id": "ecb8feaeb7fe48dc2761b8634599433c2b672181",
          "url": "https://github.com/adbrowne/smelt-sql/commit/af60af612e0aafd34c043ed4c6e571b7fd029da5"
        },
        "date": 1788775581197,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 43.924268000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 42.067083,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.889564,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.482323,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.242709,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 930.160153,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.2341990000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.770286,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.709947,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.536173,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 780.8629860000001,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.611689999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 25.81785,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 10.740861,
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
          "id": "af60af612e0aafd34c043ed4c6e571b7fd029da5",
          "message": "Merge pull request #186 from adbrowne/outcome-loop-20260904-programme-hygiene\n\noutcome: 2026-09-04 programme hygiene + backlog",
          "timestamp": "2026-09-07T20:02:38+10:00",
          "tree_id": "ecb8feaeb7fe48dc2761b8634599433c2b672181",
          "url": "https://github.com/adbrowne/smelt-sql/commit/af60af612e0aafd34c043ed4c6e571b7fd029da5"
        },
        "date": 1788775585043,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 32.094819959033074,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}