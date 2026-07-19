window.BENCHMARK_DATA = {
  "lastUpdate": 1784455966162,
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
          "id": "a3e4dadd263d1b59f8ace77e87814a5bea4149ad",
          "message": "Merge pull request #163 from adbrowne/spec-incremental-models-consolidation\n\nConsolidate maintenance_plan/batched/keyed/versioned specs into incremental_models.md",
          "timestamp": "2026-07-19T20:08:01+10:00",
          "tree_id": "e5116b01c604321477892f46887add01531ca9d5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/a3e4dadd263d1b59f8ace77e87814a5bea4149ad"
        },
        "date": 1784455961861,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 47.300388,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 45.25657,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.950018,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.5598559999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.26497800000000005,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 942.014706,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.4514340000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.725163,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.347644,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.8368209999999999,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 802.97207,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.75825,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.420260000000006,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.568247,
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
          "id": "a3e4dadd263d1b59f8ace77e87814a5bea4149ad",
          "message": "Merge pull request #163 from adbrowne/spec-incremental-models-consolidation\n\nConsolidate maintenance_plan/batched/keyed/versioned specs into incremental_models.md",
          "timestamp": "2026-07-19T20:08:01+10:00",
          "tree_id": "e5116b01c604321477892f46887add01531ca9d5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/a3e4dadd263d1b59f8ace77e87814a5bea4149ad"
        },
        "date": 1784455965461,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.662833283922218,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}