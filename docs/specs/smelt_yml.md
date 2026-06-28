---
feature: smelt_yml
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Project Configuration (`smelt.yml`)

> **What this is.** Normative spec for the project-level configuration file at the root of every smelt workspace. The `smelt.yml` surface defines the project name, where to find models and seeds, the executable backend targets, and project-wide defaults. Per-feature configuration (`incremental:` shape, function `backends:` frontmatter, …) is owned by the feature's own spec; this spec covers the top-level keys, the precedence rules, and unknown-key handling. This is a stub — sections may be brief — but every section says something concrete. Per-key reference (`docs-site/docs/reference/smelt-yml.md`) is the user-facing details page; this spec is what that reference must agree with.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### Top-level keys

| Key | Type | Required | Default | Meaning |
|-----|------|----------|---------|---------|
| `name` | string | yes | — | Project name. Decorative; appears in run logs. |
| `version` | integer | no | `1` | Schema version of the `smelt.yml` format. Optional to remove the trip-hazard where users wrote a semver string and got a parse error. Currently only `1` is meaningful. |
| `paths` | list of strings | no | `["models"]` | Directory prefixes **stripped** from entity addresses (`architecture.md` §"Resolution"). This does **not** gate discovery: smelt scans **every non-excluded subdirectory** under the project root and classifies each file by format/content (`.sql`, `.py`, `.csv`, `.yml`), regardless of which directory it lives in. The default `["models"]` simply strips a leading `models/` so the conventional layout addresses cleanly (`models/marts/x.sql` → `smelt.marts.x`); a project that groups files by domain under another container sets `paths:` to that container (`["src"]`). |
| `targets` | map of `<name>` → target object | yes | — | Named execution environments. The `--target` CLI flag selects one (default `dev`). |
| `default_materialization` | string | no | `"view"` | Project-level fallback storage mode (the storage axis) for any model that does not declare its own. Accepts only the four storage modes: `table`, `view`, `materialized_view`, `ephemeral`. Refresh strategies (`refresh: cumulative`, `incremental:`) and the kind axis are not settable here — they are per-model concerns (see Semantics §8). |
| `models` | map of `<name>` → model-config object | no | `{}` | Per-model overrides keyed by model name. Each entry may declare `materialization`, `tags`, `target`, `incremental`, etc. |
| `python` | string | no | — | Path to a Python interpreter for Python-model discovery. The `SMELT_PYTHON` environment variable takes precedence. |
| `unstable_schema` | bool | no | `false` | Gate for unstable feature surfaces. When `true`, the gated keys listed in §"`unstable_schema:` gated keys" parse without warnings. |
| `vars` | map of `<name>` → YAML scalar | no | `{}` | Compile-time variable declarations read by `smelt.config.var('<name>')`. The value shape is a flat map of name → scalar (string/number/bool/null); the lookup, YAML-scalar coercion, and per-target overlay semantics are owned by `meta_language.md` §"Compile-time variables". |
| `state` | object | no | `{ mode: stateless }` | Project state posture. Carries `mode:` (`stateless` \| `intervals` \| `environments`); the posture lattice, reuse semantics, and environment addressing are owned by `virtual_environments.md` §"`state.mode` — the opt-in posture". |

### `unstable_schema:` gated keys

Setting `unstable_schema: true` unlocks the following feature surfaces. Each is gated because its syntax is being prototyped against real usage and may change before graduating to stable:

| Key | Owning spec | Status |
|-----|-------------|--------|
| `joins:` on `smelt.define` / `smelt.extern` frontmatter | `functions.md` | Experimental |
| `provenance:` on `smelt.define` / `smelt.extern` frontmatter | `functions.md` | Experimental |

An enumeration command (`smelt unstable list`) that prints this list is open work. Until it exists, this table is the canonical v1 source of truth. Entries are removed from this table when the feature graduates to stable (gate dropped) or is removed.

The full per-key reference (target sub-shape, model-config sub-shape, incremental config, schema-evolution config) is in `docs-site/docs/reference/smelt-yml.md`. This spec mirrors the implemented `Config` struct in `crates/smelt-core/src/config.rs`; anything beyond what that struct accepts is "shape TBD" until it lands.

### Target shape (per `targets.<name>`)

| Field | Type | Required | Meaning |
|-------|------|----------|---------|
| `type` | string | yes | Backend type — `duckdb` or `spark`. |
| `schema` | string | no | Schema name used for materialised tables/views and target-schema seeds. Defaults to `main` when omitted (matches `architecture.md` §"Default materialization name mapping"). |
| `database` | string | DuckDB only | Path to the `.duckdb` file (relative to project root). |
| `connect_url` | string | Spark only | Spark Connect URL (e.g. `sc://localhost:15002`). |
| `catalog` | string | Spark only | Optional Spark catalog name. |
| `warehouse` | string | Spark only | Base directory for file-based output (Parquet warehouse). |
| `format` | string | Spark only | `delta` (default) or `parquet`. Affects schema-evolution capabilities. |
| `settings` | map of string → string | DuckDB only | Connection-time settings applied as `SET key = value` on open. Unknown keys are rejected with an error. Common keys: `memory_limit`, `threads`, `temp_directory`. When `memory_limit` and/or `temp_directory` are absent, smelt supplies conservative defaults (see Semantics §8); any key the user sets is applied verbatim and never overridden. |

### Model-config shape (per `models.<name>`)

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `materialization` | string | inherits `default_materialization` | Override per model. |
| `timeseries` | object | absent | Time-dimension declaration — full shape in `timeseries.md`; required when `incremental:` is set. |
| `incremental` | object | absent | Incremental-materialization config — full shape in `incremental_models.md`. |
| `tags` | list of strings | `[]` | Selector tags merged with frontmatter tags (union, deduplicated). |
| `target` | string | inherits CLI `--target` | Override which target this model executes on. |
| `format` | string (`delta` \| `parquet`) | inherits `targets.<name>.format` | Per-model table format override. Ignored for DuckDB targets; meaningful only on Spark targets. Full key semantics in `models.md` §"YAML frontmatter keys". |

### Precedence rules

1. **Materialization**: SQL frontmatter `materialization:` > `smelt.yml` `models.<name>.materialization` > `smelt.yml` `default_materialization` > built-in default `view`.
2. **Incremental**: SQL frontmatter > `smelt.yml::models.<name>.incremental`.
3. **Target**: SQL frontmatter `target:` > `smelt.yml::models.<name>.target` > CLI `--target` flag (default `dev`).
4. **Tags**: union of `smelt.yml::models.<name>.tags` and frontmatter `tags`, deduplicated.
5. **Format**: SQL frontmatter `format:` > `smelt.yml::models.<name>.format` > `targets.<name>.format` (Spark default `delta`). The model override wins over the target, the same way `materialization:` does.

### Unknown keys

An unknown top-level key produces a warning naming the key, not an error. The file otherwise parses normally. This is intentional — adding a key is non-breaking, and rejecting unknown keys would force a lockstep between the `smelt.yml` parser and every consumer of the config struct.

A typo'd known key (e.g. `default_matrialization`) is reported as an unknown key and produces the same warning. Whether smelt should additionally emit a fuzzy "did you mean …" hint is open (see Known Divergences).

## Semantics

1. **`smelt.yml` must exist at the workspace root.** Absence is a hard error from any subcommand that loads the project. This is the only filesystem invariant the architecture spec mandates (`architecture.md` §"Project layout").
2. **`name` is decorative.** It appears in run logs but is not used as a schema name, table name, or namespace component. Renaming the project is safe.
3. **`version: 1` is the only accepted value today.** Omitting `version` parses as `1`; supplying a string (`"0.1.0"`) is rejected with a deserialisation error so the user-mistake (mirroring `pyproject.toml`) surfaces clearly.
4. **`default_materialization` is `view` by default.** This is the implementation default in `Config::default_materialization()`. Users who want every model materialised as a table must set `default_materialization: table` explicitly (or annotate per-model).
5. **`paths` is an address strip-list; discovery is project-wide.** Smelt walks **every non-excluded subdirectory** under the project root (the directory containing `smelt.yml`), classifying each file by format/content (`.sql` bare-SELECT → model, `.sql` `smelt.define` → function, `.csv` → seed, `.yml` with sibling `.csv` → sidecar, `.yml` standalone → source). `paths:` does **not** restrict what is discovered — its entries are directory prefixes **stripped** from the resulting `smelt.<path>` addresses (`architecture.md` §"Resolution"). **Excluded** directories — hidden directories (`.`-prefixed, e.g. `.git`, `.smelt`) and the conventional build output `target/` — are not scanned. A `paths:` entry naming a directory that does not exist is harmless: there is simply nothing for it to strip.
6. **`unstable_schema: true` opts in to gated keys.** The list of currently-gated keys is owned by the feature spec that introduces them (today: `joins:` and `provenance:` in `functions.md`). The flag itself is parsed by a small text-based helper (`parse_unstable_schema_flag`) so even malformed `smelt.yml` files can be probed for the gate.
7. **Forward compatibility via warnings.** A new release that adds a top-level key produces a warning on older consumers, not an error — the old binary keeps working with the field absent. This invariant means tooling can read newer projects without crashing.
8. **`default_materialization` is the storage axis only.** The legal values are the four storage modes: `table`, `view`, `materialized_view`, and `ephemeral`. It does not touch the other two axes. The kind axis is not a materialization (a unit test is a `smelt.test` declaration, `testing.md`), so there is nothing test-shaped to default. The refresh axis is intentionally per-model: `refresh: cumulative` derives its driving partition shape and unique key from the individual model's SQL, so a project-wide default would be meaningless; refresh strategies are set per-model (frontmatter or `models.<name>`), never as a blanket fallback. `ephemeral` is retained as a defensible storage default.
9. **DuckDB resource use is bounded by default.** Left to its own defaults, DuckDB sizes its buffer pool at ~80% of *total host RAM* and only spills to disk once that ceiling is reached. On a shared machine a single heavy model can therefore consume the whole host and drive other processes into memory pressure. To prevent this, when a DuckDB target's `settings:` omits a key, smelt fills it in at connection time:
   - **`memory_limit`** — defaults to `min(50% of total RAM, total RAM − 20 GB)`, floored at 40% of total RAM so small hosts stay usable. This is deliberately conservative: DuckDB's `memory_limit` bounds its buffer pool but *not* total process RSS, which runs several GB higher (untracked operator, scan, and Arrow-assembly memory), so the limit is set well below the host to keep actual RSS within a safe envelope and leave headroom for the OS and co-tenant processes. If smelt cannot determine total RAM on the platform, it sets no default and DuckDB's own ~80% default applies.
   - **`temp_directory`** — defaults to `<database-parent>/.smelt-duckdb-tmp` (i.e. alongside the `.duckdb` file, normally under `target/`), so a query that exceeds `memory_limit` spills to disk rather than failing or growing unbounded.

   A user-supplied value for either key is applied verbatim and is **never** overridden — explicit configuration always wins. `threads` is left at DuckDB's default.

## Design

**`name` and `version` are decorative because identity is the directory.** Earlier shapes used `name` as a namespace component (e.g. `<name>.<schema>.<table>` qualified paths). That conflated workspace identity with database identity and made renames painful — every downstream model had to update its qualified references. Today the workspace's identity is its filesystem location, the directory containing `smelt.yml`; `name` is informational. This composes with the universal `smelt.<path>` addressing scheme (`architecture.md`) — paths refer to entities by location, not by project name.

**`default_materialization: view`.** A view-by-default doctrine keeps development cheap (no table-rebuild cost on every change) and pushes the user to opt in to `table` exactly where performance matters. Earlier discussions floated `table` as the default, but that punishes the iteration loop where most models are still being shaped. The view default is consistent with the dbt convention and matches the implementation. (Notably, the smelt-loop finding DG-9 originally suggested the default was `table`; that was an artefact of the loop agents' discovery process — the canonical default is `view`.)

**`unknown keys = warning, not error`.** This is the project-level side of the unknown-key doctrine (`architecture.md` §"Constraints & Invariants" §8). The forward-compatibility invariant in Semantics §7 is the load-bearing reason. Strict-rejection of unknown keys forecloses staged rollouts (a new key cannot be authored in the project until every consumer recognises it). Warning-on-unknown lets new fields land in the parser ahead of every consumer recognising them, while still surfacing typos to the user.

**DuckDB defaults bound the host, not just the query.** DuckDB's native `memory_limit` default is a fraction of *total system RAM*, which is the right choice for a dedicated analytics box but antisocial on a developer machine or CI runner shared with other work (editors, language servers, a second build, an agent loop). A single 1-billion-row aggregation will happily grow toward that ~80% ceiling and tip the whole host into memory pressure — on Linux, `systemd-oomd` then reaps a cgroup chosen by pressure, which may be an unrelated session rather than the offending build. Smelt therefore treats *bounded, spilling-by-default* execution as the correct out-of-the-box posture: leave real headroom, and always give DuckDB a temp directory so it spills instead of consuming or failing. We deliberately do **not** override a user who has set these keys — someone who writes `memory_limit: 48GB` has opted into that trade — and we leave `threads` alone because thread count is a throughput/latency choice, not a safety one. The smaller-of-two formula (`50%` vs `RAM − 20 GB`) keeps the absolute headroom generous on large hosts while the percentage keeps it proportional on small ones; the 40%-floor stops the `RAM − 20 GB` term from going to zero (or negative) on a ≤20 GB laptop. The constants are intentionally conservative because the limit governs DuckDB's buffer pool, not process RSS — measured RSS on a real 1-billion-row aggregation ran ~5 GB above the configured limit, so a limit set close to the host would still let RSS approach the OOM wall.

**Per-feature config lives in feature specs.** `incremental:` shape, function frontmatter keys (`deterministic`, `idempotent`, …), schema-evolution config, and the per-column `tests:` map are owned by their feature specs (`incremental_models.md`, `functions.md`, future `schema_evolution.md`). This spec lists only the top-level shape and the cross-feature precedence rules; otherwise, the page would have to be re-written every time a new key lands.

**`paths:` strips addresses; it does not gate discovery.** Earlier the config had `model_paths` and `seed_paths` as separate scan lists, with `sources.yml` declaring sources at the project root regardless. That conflated *where to look* with *what kind of thing is here* — yet `architecture.md` §"Resolution" already says kind is determined by file format/content, not by directory. The resolution is to make discovery **unconditional** — smelt scans every non-excluded subdirectory and classifies on read — and to repurpose `paths:` as a list of address-prefixes to **strip**. This directly serves large projects organised by *domain* rather than by *kind*: a team can keep `billing/staging/invoices.sql`, `billing/raw/invoices.yml` (a source), and `billing/seeds/tax_rates.csv` side by side under one `billing/` subtree, and they address as `smelt.billing.staging.invoices` / `smelt.billing.raw.invoices` / `smelt.billing.seeds.tax_rates` with no per-kind directory required. A project that wants to hide a single top-level container from the address (the `src/` pattern) lists it in `paths:`. The earlier scan-list design forced files to live under enumerated roots; the strip-list design lets the filesystem hierarchy follow the team's mental model while keeping addresses clean.

## Constraints & Invariants

1. The implemented `Config` struct in `crates/smelt-core/src/config.rs` is the source of truth for the parser. The Surface table above must match the struct's serde-deserialisation contract.
2. `default_materialization` defaults to `Materialization::View` — never `Table` — until a deliberate spec change moves the default.
3. Unknown top-level keys produce a warning, not an error. Strict-rejection requires a spec change.
4. `version` is an integer — string values are rejected at parse time.
5. The `unstable_schema:` gate is read by a separate text-based helper so it remains probeable on otherwise-malformed configs.
6. Smelt-supplied DuckDB defaults (`memory_limit`, `temp_directory`) are computed by a **pure function** of `(total_ram_bytes, database_parent_dir, user_settings)` so the policy is unit-testable without opening a connection; platform RAM detection is a thin impure shim whose failure mode is "supply no `memory_limit` default" (never panic, never block the connection). A user-provided value for a key suppresses the default for that key.

## Known Divergences / Open Questions

- **Fuzzy typo hints.** Unknown top-level keys (including typos of known keys) are warned by name. A future "did you mean …" hint that fuzzy-matches the offending key against the known-key set is open; not implemented today.
- **Per-key reference drift.** The user-facing reference (`docs-site/docs/reference/smelt-yml.md`) currently documents some fields this spec does not yet cover (`schema_evolution`, `columns`). The reference is ahead of the spec on those keys; when the corresponding feature specs land they will absorb those fields.
- **Multi-target precedence with frontmatter `target:`.** The model-level `target:` frontmatter overrides `smelt.yml::models.<name>.target`, which overrides the CLI `--target`. The frontmatter form is a relatively recent addition; whether it should also be allowed to declare a target *not* defined in `smelt.yml::targets` is open (today: hard error before any work begins).
- **Default `memory_limit` formula is a heuristic.** `min(50% RAM, RAM − 20 GB)` floored at 40% is a deliberately simple policy, not a tuned one; it does not account for cgroup memory limits (a container may advertise 64 GB of host RAM while capped at 8 GB), other concurrent smelt targets, or NUMA. A future refinement could read the process's own cgroup `memory.max` instead of host RAM, and/or expose a project-level `default_memory_headroom` knob.
- **Cross-platform RAM detection.** Total-RAM detection is implemented for Linux (`/proc/meminfo`) and macOS (`sysctl hw.memsize`); on other platforms detection returns nothing and no `memory_limit` default is applied (DuckDB's native ~80% default stands). `temp_directory` is always defaulted regardless of platform.
- **`unstable_schema:` discoverability.** There is no `smelt unstable_schema list` or similar way to enumerate the currently-gated keys. Users find them through individual feature docs; a discoverability mechanism is open.
- **User-extensible top-level keys.** Today the top-level key set is closed (unknown keys warn). A future direction is to let projects register their own top-level keys to carry configuration for custom planner rules. That extensibility is not specified now; the key set in §"Top-level keys" remains the authoritative list, and `vars:` / `state:` point at their owning feature specs for semantics.
- **Configurable discovery exclusions.** Project-wide discovery skips hidden directories (`.`-prefixed) and the conventional build output `target/`. Whether a project can extend this with an explicit `exclude:` key (to omit, say, a `sandbox/` or `archive/` subtree from discovery) is open; today the skip-list is fixed and conventional. The `paths:` strip-list is a separate concern (it renames addresses, it does not hide files).

## References

- **Code**: `crates/smelt-core/src/config.rs` — `Config`, `Target`, `ModelConfig`, `Materialization`, `parse_unstable_schema_flag`, `parse_active_backends`, `parse_with_warnings`. The default functions are `default_paths` (for the unified `paths:` list) and `default_materialization`. DuckDB resource defaults live in `crates/smelt-backend-duckdb/src/lib.rs` — the pure `resolve_duckdb_settings` policy function and the `detect_total_ram_bytes` platform shim, applied inside `DuckDbBackend::new_with_settings`.
- **Tests**: `crates/smelt-core/src/config.rs::tests` — round-trip tests for materialization defaults, version handling, target shape, validation rules.
- **User docs**: `docs-site/docs/reference/smelt-yml.md` — per-key reference; `docs-site/docs/concepts/project-structure.md` — orientation page.
- **Plans (history)**: `docs/plans/20260502-smelt-loop-findings.md` — the spec-authoring plan that produced this stub (DG-4 close).
- **Related specs**:
  - `architecture.md` — `smelt.yml` is the workspace marker; discovery is project-wide and `paths:` is the address strip-list (not a scan gate); kind-by-content is owned there.
  - `incremental_models.md` — `incremental:` shape on `models.<name>`.
  - `functions.md` — `unstable_schema: true` gates `joins:` and `provenance:` frontmatter.
  - `seeds.md` — CSV parsing rules and `smelt seed` lifecycle; consumes `paths:` and `targets.<name>.schema`.
  - `sources.md` — source declaration shape; consumes `paths:` for discovery. The target-aware source `name:` override is keyed by `targets.<name>` but its grammar lives in `sources.md` §"Target-aware `name:` override" — `smelt.yml` carries no source-name config key.
  - `cli.md` — `--target` resolves against `targets`; `--project-dir` is the workspace root.
  - `meta_language.md` — semantics of the `vars:` block consumed by `smelt.config.var('<name>')`.
  - `virtual_environments.md` — semantics of the `state:` block (`mode:` posture).
