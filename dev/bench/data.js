window.BENCHMARK_DATA = {
  "lastUpdate": 1788527751319,
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
          "id": "3e9c1a4adfd1a4602456274de7320b5480808616",
          "message": "decisions: 2026-09-04 decision track — lateness is orchestration-only, seven more product calls\n\nRecords docs/research/20260904-decision-track.md and lands each decision as a\nspec diff (spec-first):\n\n- incremental_shapes: non-deterministic membership is a permanent refusal\n  (divergence deleted, frozen-membership as a future extension); key-grain\n  rule 16 — derived recurrence authoritative, declared is a check\n  (KeyedRecurrenceDeclarationMismatch), key sets order-independent; route 2\n  derived sub-route scheduled; rungs 3–4 gated on the change-feed design;\n  route 1 margin is the SQL-derived skew, never declared lateness.\n- model_properties: \"Declared lateness is orchestration-only\" constraint;\n  two temporal walks by design (open question closed); closure-through-a-fold\n  as a future extension with a trigger; MP-04 and the posture-probe bullet\n  restated as implementation gaps under the new rule.\n- sources: lateness leaves the trust rule — it is never a plan input.\n- models: per-column data_latency retired (still live; scheduled).\n- diagnostics: KeyedRecurrenceDeclarationMismatch catalogued.\n- walk-migration-residue: phase 4 (probe consults lateness) removed.\n- New outcome docs/outcomes/20260904-decision-residue queued after\n  decided-gap-residue; ROADMAP entry.\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>",
          "timestamp": "2026-09-04T23:13:01+10:00",
          "tree_id": "c8482fef59203d76c4a154a9468a422f39d06bb8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3e9c1a4adfd1a4602456274de7320b5480808616"
        },
        "date": 1788527745663,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 61.689882,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 59.242329000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.155996,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.6484530000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.336625,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1161.930675,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.5117,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.387012,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.2521120000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.664474,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 968.755205,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.901610000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.91828,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.767173,
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
          "id": "3e9c1a4adfd1a4602456274de7320b5480808616",
          "message": "decisions: 2026-09-04 decision track — lateness is orchestration-only, seven more product calls\n\nRecords docs/research/20260904-decision-track.md and lands each decision as a\nspec diff (spec-first):\n\n- incremental_shapes: non-deterministic membership is a permanent refusal\n  (divergence deleted, frozen-membership as a future extension); key-grain\n  rule 16 — derived recurrence authoritative, declared is a check\n  (KeyedRecurrenceDeclarationMismatch), key sets order-independent; route 2\n  derived sub-route scheduled; rungs 3–4 gated on the change-feed design;\n  route 1 margin is the SQL-derived skew, never declared lateness.\n- model_properties: \"Declared lateness is orchestration-only\" constraint;\n  two temporal walks by design (open question closed); closure-through-a-fold\n  as a future extension with a trigger; MP-04 and the posture-probe bullet\n  restated as implementation gaps under the new rule.\n- sources: lateness leaves the trust rule — it is never a plan input.\n- models: per-column data_latency retired (still live; scheduled).\n- diagnostics: KeyedRecurrenceDeclarationMismatch catalogued.\n- walk-migration-residue: phase 4 (probe consults lateness) removed.\n- New outcome docs/outcomes/20260904-decision-residue queued after\n  decided-gap-residue; ROADMAP entry.\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>",
          "timestamp": "2026-09-04T23:13:01+10:00",
          "tree_id": "c8482fef59203d76c4a154a9468a422f39d06bb8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3e9c1a4adfd1a4602456274de7320b5480808616"
        },
        "date": 1788527750224,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.039708587957744,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}