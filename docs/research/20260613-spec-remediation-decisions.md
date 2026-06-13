# Spec remediation — design decisions to mark up

> **Companion to** [`docs/plans/20260613-spec-remediation.md`](../plans/20260613-spec-remediation.md) (Track C). Source findings: [`docs/research/20260612-spec-review.md`](20260612-spec-review.md).
>
> **Purpose.** This collapses the ~40 genuine *design decisions* from the 2026-06-12 spec review into one document so you can decide them in a single offline pass instead of ~40 interactive rounds. Each entry states the conflict, the options, and my recommendation. **Fill in the `Decision:` line** (a letter, or free text). Where you just agree with the recommendation, write `A` (or "rec"). Anything you leave blank, I'll come back and ask about before editing that spec.

## How to use this

- **Decide:** put a letter (or your own answer) on the `Decision:` line. `Notes:` is optional.
- **Scope:** decisions are grouped into 8 themes matching the plan. Each maps to one spec-edit commit once decided.
- **Not in this doc (handled elsewhere, nothing dropped):**
  - **cumulative_aggregate** & **incremental_models** internal design — decided in their `/smelt:spec` rewrites (R1/R2). Their cross-cutting touchpoints that affect *other* specs appear here, flagged `↔R1/R2`.
  - **Determinate fixes** (the review's `_Fix:_` leaves no real choice) — listed in **Appendix A** for a quick veto pass; otherwise they just get applied in Track B.
- **Legend:** `[C#]` Critical, `[Maj]` Major, `[Min]` Minor from the review. `impl→` = a code change follows in the named Track-D wave once the spec is settled.

---

## Theme 1 — Architecture resolution & naming
*(architecture.md is `stable`; these are the load-bearing addressing rules. impl→ D-resolve.)*

### D-01 · [C1] Resolution table vs dedicated-scan-path note — how are sources/seeds addressed?
**Conflict.** The example table (captioned `paths: ["models"]`) shows `sources/raw/events.yml → smelt.raw.events` and `seeds/raw/users.csv → smelt.raw.users` (prefixes stripped). But the note below says functions/, sources/, tests/ are dedicated scan roots whose prefix is **not** stripped (so it should be `smelt.sources.raw.events`); and per seeds.md a seed is a `.csv` under a `paths:` dir, so under `paths:["models"]` that file isn't a seed at all.
**Options.**
- **A (recommended)** — Sources and seeds are **dedicated scan roots, prefix retained**: `smelt.sources.raw.events`, `smelt.seeds.raw.users`. Fix the table rows; add seeds to the dedicated-scan-path note; reconcile seeds.md's "under `paths:`" wording to "under the `seeds/` scan root". Uniform "directory name = address prefix", matches the recommended-layout ergonomics architecture.md already sells.
- **B** — Seeds discovered under `paths:` (keep seeds.md as-is); to use the example, caption becomes `paths: ["models","seeds"]` and seeds strip → `smelt.raw.users`; sources stay dedicated/un-stripped.
**Recommendation:** A — symmetry between sources and seeds; avoids the "is it in paths?" ambiguity.
**Decision:** ____________   **Notes:** ____________

### D-02 · [C2] Default name-mapping is non-injective → silent table clobbering
**Conflict.** "Table = address path joined with `_`, underscores preserved (no escaping)" makes `smelt.staging.orders` and `smelt.staging_orders` both emit `main.staging_orders`. The only collision check is over `smelt.<path>` addresses, not over emitted `(schema, table)` names — so two models silently materialize into one table (a fail-loud violation, "wrong data with exit 0").
**Options.**
- **A (recommended)** — Add an **emitted-name collision diagnostic** (Error) computed over `(target schema, joined name)` of all persisted entities in a project, analogous to `DuplicateAddress`. Keeps the readable underscore join; catches the rare collision loudly. `impl→ D-resolve` (new diagnostic).
- **B** — Specify an **injective join** (escape `_` in components, e.g. double-underscore). No new diagnostic, but uglier table names and a migration for existing tables.
**Recommendation:** A — preserves ergonomic names, fail-loud on the genuine collision.
**Decision:** ____________   **Notes:** ____________

### D-03 · [C22] `smelt-logical` crate ownership vs every planner spec
**Conflict.** architecture.md (a `stable` invariant) assigns the logical `Plan`/`LogicalNode` model, `RuleContext`, `detect_builtin_rules`, and pure classifiers to a `smelt-logical` crate "below smelt-db and smelt-planner". But planner_integration.md / cumulative_aggregate.md / incremental_models.md References still place these in `crates/smelt-planner/...`. No feature spec mentions `smelt-logical`.
**This is a factual question, not a preference:** does the `smelt-logical` crate exist in the tree today?
**Options.**
- **A** — It exists / is intended-and-tracked → update the three feature specs' References to point at `smelt-logical`. (I'll verify against the crate tree before editing.)
- **B** — Not yet → move the `smelt-logical` ownership claim in architecture.md to Known Divergences with the tracking plan, and leave feature specs pointing at smelt-planner.
**Recommendation:** verify first, then A if the crate is real, else B. (I'll `cargo tree`/`ls crates/` to settle the fact rather than guess.)
**Decision:** ____________   **Notes:** ____________

### D-04 · [Maj] Target schema `main` default vs `schema:` required
**Conflict.** architecture.md says Schema = active target's `schema:` "default `main`"; smelt_yml.md marks `schema` **Required = yes** with no default. Omitting `schema` is either a hard error (smelt_yml) or silently `main` (architecture).
**Options.**
- **A (recommended)** — `schema` **optional, documented default `main`** in smelt_yml.md; drop the "required" mark. Lower-friction onboarding; matches architecture's examples.
- **B** — Keep `schema` **required**; drop "default `main`" in architecture.md (examples just pick `main`). More explicit, more boilerplate per target.
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

### D-05 · [Maj] Scan universe: "hardcoded `functions/`" vs "file kind is grammar, not location"
**Conflict.** §Resolution says `random/x.sql` declaring `smelt.define helper` is callable as `smelt.random.helper` (functions live anywhere). The parity rule says function discovery covers "definitions under the hardcoded `functions/` directory". Under that, `random/x.sql` is never scanned; and if `random` were in `paths:`, stripping would yield `smelt.helper`, not `smelt.random.helper`. No config produces the example's address.
**Options.**
- **A (recommended)** — Define the scan universe normatively: **all `.sql` under the project root are scanned for declarations** (kind by content), `functions/` is just the conventional home, not a gate; address = directory path of the declaring file (dedicated scan roots like `functions/` retain their prefix, consistent with D-01). Reword the parity-rule sentence.
- **B** — Functions only discovered under `functions/` (drop the "anywhere" example). Simpler scan, but contradicts "file kind is grammar" and breaks the `random/x.sql` example.
**Recommendation:** A — keeps the "kind is a property of the declaration" doctrine that the whole addressing scheme rests on.
**Decision:** ____________   **Notes:** ____________

### D-06 · [Min] Stem-uniqueness rule vs address-collision rule
**Conflict.** "Names must be unique within a directory across all kinds (`data/users.csv` + `data/users.sql` is a load error)" is a *file-stem* rule; but for multi-model or function-only files the stem registers no address, so the address-based `project_address_collisions` rule says no error. Two different answers.
**Options.** **A (recommended)** make address-collision the single normative rule, delete the stem sentence. **B** keep the stem rule as deliberate conservatism even when the stem registers no address, and name the diagnostic.
**Recommendation:** A — one rule, and it's the one that matches the universal-addressing model.
**Decision:** ____________   **Notes:** ____________

---

## Theme 2 — Diagnostics: which codes exist, who owns them, severities
*(impl→ D-diag.)*

### D-07 · [Maj] `ColumnTypeUnresolved` — minted-and-firing, or reserved? (recurs in 3 specs)
**Conflict.** types.md §Surface lists it and §Semantics says it "fires by default"; function_schema_inference.md describes its message/anchor/trigger in detail; but its own Known Divergences say it "has no trigger today and stays reserved… not yet minted", and diagnostics.md's catalogue has **no entry** (so the coverage gate is silent). Three specs disagree on whether the code exists/fires.
**Options.**
- **A (recommended)** — **Reserved/unminted today.** Soften types.md §Semantics ("intended schema-layer rule; minting tracked in function_schema_inference.md"), add a catalogue **stub marked reserved** in diagnostics.md, keep the firing rules as the target. Matches the owner spec's own status. `impl→ D-diag/D-misc` mints it later.
- **B** — **Declare it live now** — add a full catalogue row, make types.md/function_schema_inference.md present-tense, and treat non-emission as an implementation gap to close in D-misc.
**Recommendation:** A — don't advertise a code as firing when the owner says it isn't; reserved-with-stub keeps the gate honest.
**Decision:** ____________   **Notes:** ____________

### D-08 · [Maj] `UnknownSmeltPath` (lsp.md) absent from the catalogue
**Conflict.** lsp.md treats `UnknownSmeltPath` as a real code (References + a Code Action trigger); diagnostics.md has no such code — it has `UndefinedModelRef` and `UndefinedSource`. lsp.md's own Known Divergences admits the mismatch but Surface still uses the nonexistent code.
**Options.**
- **A (recommended)** — Rewrite lsp.md Surface + Code Actions to use the two catalogued codes (`UndefinedModelRef`/`UndefinedSource`); resolve the "which code for a bare unresolved `smelt.<path>`" via D-09's tie-break.
- **B** — Add `UnknownSmeltPath` to the catalogue as the single unresolved-address code and **retire** `UndefinedModelRef`/`UndefinedSource`. Cleaner single code, but a bigger change touching every spec that names the two existing codes.
**Recommendation:** A — least churn; the two codes already exist and are referenced widely. (But see D-09 — if you prefer one unified code, pick B here and B in D-09.)
**Decision:** ____________   **Notes:** ____________

### D-09 · [Min] `UndefinedModelRef` vs `UndefinedSource` — tie-break for a bare unresolved `smelt.<path>`
**Conflict.** Both triggers cover `smelt.<path>` resolving to nothing; for a path that resolves to no entity *of any kind*, the spec gives no rule for which fires (the intended kind is unknowable when it doesn't exist).
**Options.** **A (recommended)** bare `smelt.<path>` resolving to nothing → `UndefinedModelRef` (default); reserve `UndefinedSource` for explicit `smelt.source()` calls. **B** mint a dedicated `UnresolvedAddress` for the kind-unknown case (pairs with D-08 B). 
**Recommendation:** A (unless you chose D-08 B, then B here).
**Decision:** ____________   **Notes:** ____________

### D-10 · [Maj] lsp.md severities (UndeclaredColumn/AmbiguousColumn) Warning vs catalogue's Error
**Conflict.** lsp.md's severity table marks them **Warning**; diagnostics.md catalogues both as **Error**. The build gates on `severity == Error`, so this is load-bearing (block the build or not).
**Options.**
- **A (recommended)** — lsp.md **stops restating per-code severities** and defers to diagnostics.md (keep only the Severity→LSP-level mapping incl. `Info`→INFORMATION). Single source of truth; kills this whole drift class. Pairs with the lsp catalogue-duplication cleanup (Appendix A).
- **B** — Keep lsp.md's table but correct the rows to Error.
**Recommendation:** A — the duplicated catalogue in lsp.md has already drifted twice; remove it.
**Decision:** ____________   **Notes:** ____________

### D-11 · [Maj] `IncrementalNotBatchSafe` — define its trigger or remove it
**Conflict.** The catalogue lists it (Warning, "execution falls back to a safe chunking strategy") but incremental_models.md never mentions it and treats `BoundedSafe`/`PerPartitionOnly` as normal modes, not warned fallbacks. An implementer can't tell which class fires it. `↔R2`
**Options.** **A** define the trigger in the incremental rewrite (which class, what anchor) — defer to R2. **B (recommended)** **remove the code** if the three-class taxonomy supersedes it; if R2 surfaces a real need, mint it there.
**Recommendation:** B, but confirm in R2. Mark this `↔R2` so it's settled with the rewrite.
**Decision:** ____________   **Notes:** ____________

### D-12 · [Maj] Owner of the four planner-validation codes (functions.md vs planner_integration.md)
**Conflict.** diagnostics.md groups `ProvenanceMismatch`/`JoinsMismatch`/`DeclaredCardinalityUnverifiable`/`MissingProvenancePushdownAdvisory` under "Owned by functions.md", but planner_integration.md asserts ownership of the same four. (A2 already pointed planner_integration.md's note at the catalogue; the *owner* still needs picking.)
**Options.** **A (recommended)** owner = **planner_integration.md** (natural home for planner-validation codes); update diagnostics.md's group header. **B** owner = functions.md (expansion/provenance live there); update planner_integration.md.
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

### D-13 · [carried from A4] References section format — flat bullets vs nested `### Code`/`### Tests` (17 specs)
**Conflict.** SPEC_TEMPLATE mandates **flat bullets** under References; 17 specs use nested `### Code`/`### Tests` sub-headings. Likely the template rule changed after those specs were written.
**Options.** **A (recommended)** normalize all 17 to flat bullets (matches the template + keeps cross-spec parsing scripts simple). **B** update SPEC_TEMPLATE to bless the nested form and leave specs as-is.
**Recommendation:** A — the template says flat and cites a tooling reason; 17 reformats is mechanical once decided.
**Decision:** ____________   **Notes:** ____________

### D-14 · [Min] `BackendsWideningNotAllowed` overlaps `FrontmatterParseError` on malformed frontmatter
**Conflict.** `BackendsWideningNotAllowed` fires on backend widening **or** malformed frontmatter; `FrontmatterParseError` already owns malformed frontmatter. A code named for backend widening firing on unrelated malformation defeats stable cross-references.
**Options.** **A (recommended)** remove "or the frontmatter itself is malformed" from `BackendsWideningNotAllowed`; route all malformation to `FrontmatterParseError`. **B** split `FrontmatterParseError` into parse-failure (Error) vs unknown-key (Warning) and keep widening separate.
**Recommendation:** A (B is more than this finding needs; the unknown-key severity is its own decision — see D-31).
**Decision:** ____________   **Notes:** ____________

---

## Theme 3 — Meta-language reflection & precedence
*(meta_language.md. impl→ D-misc where noted.)*

### D-15 · [C11] Spread (`...`) vs pipe (`|>`) precedence
**Conflict.** §Pipe says `|>` is "lower precedence than every other operator" (so `...` binds tighter). But the build-path example `SELECT ...smelt.columns_of(smelt.orders) |> map(fn c => c.name)` needs `|>` to bind tighter than `...` (map the list, *then* spread). Under the stated precedence it spreads the list before mapping → ill-typed.
**Options.**
- **A (recommended)** — Make `...` the **outermost (lowest)** operator so it applies after the pipe chain. The example parses as `...(columns_of(...) |> map(...))` with no parens. Update the precedence line.
- **B** — Keep pipe lowest; **parenthesize** the example: `SELECT ...(smelt.columns_of(...) |> map(...))`. Smaller text change, but every reflection-spread-with-pipe needs parens.
**Recommendation:** A — the spread-after-transform shape is the common one; making it parens-free is the better surface.
**Decision:** ____________   **Notes:** ____________

### D-16 · [C12] `ModelRef.name`/`.path` — render as string literal or lift to identifier?
**Conflict.** Build-path says these resolve to `Text` "rendered as SQL **string literals** (a model name is a data value)". Semantics rule 8 says they're meta-`Text` that, in one of four lift positions (column-ref / AS-alias / ORDER BY / GROUP BY), **lift to identifiers**. Same `m.name` in an alias position is a string literal by one rule, an identifier by the other.
**Options.**
- **A** — Wide-reflection `Text` **follows the four-position identifier lift** (like `ColumnRef.name`); drop "rendered as string literals" from Build-path.
- **B (recommended)** — `ModelRef`/`SourceRef` `Text` is a **data value (string literal)**, *not* subject to identifier lift; carve it out of the lift table and amend rule 8. A model name is rarely a column/alias; treating it as data is less surprising and avoids accidental identifier injection.
**Recommendation:** B — but this is a real UX call; if you expect `SELECT m.name AS ...` to emit a bare identifier, pick A.
**Decision:** ____________   **Notes:** ____________

### D-17 · [C13 + Maj] Wide-reflection ordering: "by path" vs "by path then name"
**Conflict.** Rule 2 says `with_tag`/`all` sort "ascending by `path`"; rule 7 says the union is "by `path` then `name` (tiebreaker for co-emitted models sharing a generator `path`)". Since co-emitted models share one `path`, "by path" alone is non-deterministic — violating the byte-equal determinism guarantee.
**Options.** **A (recommended)** canonical order = **`path` then `name`** everywhere (rule 2 becomes authoritative; rule 7 cites it); applies to all wide-reflection results. Determinism restored. `impl→ D-misc`. **B** introduce a per-emission virtual path so `path` alone is unique (bigger model change; see D-20).
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

### D-18 · [Maj] Short-circuit ternary vs Unknown-propagation for a *deferred* `m.has(k)`
**Conflict.** Rule 3 motivates `if m.has(k) then m.get(k) else default` (short-circuit so `MapGetMissingKey` doesn't fire on the unreached branch). Rule 4 says if COND synthesises to `Unknown`, both branches are type-checked but neither evaluated. For a non-static key, `m.has(k)` defers to expansion — if that makes COND `Unknown`, the motivating pattern silently degrades.
**Options.** **A (recommended)** specify that a **deferred boolean `has` does *not* collapse COND to Unknown** — it stays a boolean meta-value, so short-circuit (rule 3) governs and the defaulting pattern works. **B** accept that deferred-key defaulting degrades and document the limitation (users must use a static key).
**Recommendation:** A — the defaulting pattern is the spec's own flagship; it should work for dynamic keys.
**Decision:** ____________   **Notes:** ____________

### D-19 · [Maj] HOF named-argument error — dedicated code vs reusing kind-mismatch codes
**Conflict.** Semantics rule 4 says a HOF call with named args emits `HofExpectsLambda`/`HofExpectsReducer` (codes about *wrong kind*), and a named arg that happens to *be* a lambda would then produce **no** diagnostic. Every sibling construct has a dedicated named-arg code.
**Options.** **A (recommended)** add **`HofNamedArgument`** to the HOF Surface table; rule 4 cites it for named args; reserve the Expects* codes for wrong-kind. Mirrors `ReducerNamedArgument` etc. `impl→ D-diag`. **B** keep reusing the Expects* codes (no new code) and accept the silent-accept hole.
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

### D-20 · [Min] `ModelRef.path` for generator-emitted models — generator-file path vs per-emission path
**Conflict.** Rule 7 sets `ModelRef.path` to the **generator file's** path, so N co-emitted models share one `path` — yet each is individually addressable and must have a distinct goto-def target. Any consumer keying ModelRefs by `path` (collision/dedup/ordering/goto-def) can't distinguish co-emissions. (Root cause of D-17.)
**Options.** **A (recommended)** keep `path` = generator file (provenance), and ensure **all path-keyed operations use a separate per-emission identity** (the emitted `smelt.<path>` address) for uniqueness/goto-def; ordering uses `path` then `name` (D-17 A). **B** make `ModelRef.path` the **per-emission** smelt path (unique), and expose the generator file via a separate `generator_file` field. Cleaner identity, but changes what `path` means for reflection consumers.
**Recommendation:** A if `path`-as-provenance is intended; B if you want `path` to be a unique key. Leaning A (matches data_catalog's `generator_file` framing).
**Decision:** ____________   **Notes:** ____________

### D-21 · [Min] `ColumnRef.type == Integer` comparison semantics for parameterised types
**Conflict.** `c.type : DataType` is "comparable for equality (`c.type == Integer`)". For `Decimal(p,s)`/`Varchar(n)`/`TimestampTz`, equality is undefined: does `== Decimal` match any `Decimal(p,s)` or only exact? Also Known Divergences says `c.type` actually returns `Unknown` today, so the predicate **silently degrades** — the exact shape the spec advertises.
**Options.** **A (recommended)** specify **exact structural equality** including type parameters; provide head-constructor predicates (`c.is_decimal`) for the "any Decimal" case; and note in Semantics that the `DataType`-literal comparison is **normative-but-unlanded** (cross-ref the divergence) so readers don't treat `c.type == Integer` as working today. `impl→ D-misc`. **B** define `==` as head-constructor match (`== Decimal` matches any `Decimal(p,s)`); simpler predicates but loses precision.
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

---

## Theme 4 — Python models ↔ meta-language reconciliation
*(python_models.md + meta_language.md. impl→ D-misc.)*

### D-22 · [C14] `--- name: X ---` delimiter clash
**Conflict.** python_models.md uses `--- name: combined_events ---` as a *frontmatter* header whose `name:` must equal the function name. But models.md defines exactly `--- name: <model> ---` as the Layer-1 multi-model **section delimiter** whose name **is** authoritative; any other `--- X ---` is a hard parse error. Same token, opposite meaning across SQL vs Python paths.
**Options.**
- **A (recommended)** — Python output uses **plain `---`/`---` single-model frontmatter** (no `name:` in the delimiter); identity comes from the function name. Sidesteps the Layer-1 collision entirely.
- **B** — Reuse `--- name: X ---` and specify how the Python path reinterprets the Layer-1 delimiter (name must echo the function name). Keeps the visual, but two parsers must agree on one token's two meanings.
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

### D-23 · [C15] Circular-dependency rule forbids the self-referential generation the feature exists for
**Conflict.** Circular detection fires when "a model queries for models with a tag it itself carries" — but Design endorses exactly that (a generator that tags `staging` and a marts generator that queries `tag=staging`; multi-round fixed-point is the supported mechanism). The canonical convergent case errors.
**Options.** **A (recommended)** redefine circular-ness as **non-convergence/oscillation** (output never stabilises across the bounded rounds), not "queries a tag it carries". A monotonically-growing-then-stable set converges. **B** keep the tag rule but exempt the documented generator pattern explicitly (narrower, more special-cases).
**Recommendation:** A — convergence is the real property; ties into D-24.
**Decision:** ____________   **Notes:** ____________

### D-24 · [Maj] Global evaluation order: Python iterative rounds vs SQL-generator W1–W4 single pass
**Conflict.** python_models.md runs Python discovery in ≤5 fixed-point rounds, each seeing "all currently known models". meta_language.md defines a single bounded pass where generators "cannot observe each other's emissions" until W4. Undefined: does Python `find_models` see SQL-generator emissions? Do SQL-generator literal refs see Python emissions? How do the loops interleave?
**Options.**
- **A (recommended)** — Define a **global ordering**: (1) SQL generators run their W1–W4 single pass to a fixed set; (2) Python iterative rounds then run, each round observing the *full* SQL-generated set + prior Python rounds; (3) SQL generators do **not** observe Python emissions (one direction only). Simple, acyclic, matches "generators cannot observe each other within a pass". Document in one spec, cross-ref the other.
- **B** — Fully interleave (Python and SQL generators in one combined fixed-point). More expressive, much harder to specify deterministically and to bound.
**Recommendation:** A — one-directional layering is far easier to make deterministic; revisit if a real use-case needs SQL-sees-Python.
**Decision:** ____________   **Notes:** ____________

### D-25 · [Maj] Python model location identity: `directory` (final component) vs full `path`
**Conflict.** python_models reflects location as `ModelInfo.directory` (final component only); meta_language `ModelRef` uses full workspace-relative `path` (and for emitted models, the generator file's path). Undefined for a generator-emitted Python model; the two surfaces are inconsistent.
**Options.** **A (recommended)** align Python reflection on **full workspace-relative `path`** (rename/extend `directory`→`path`, or define `directory` = final component of that path) and define it for generator-emitted models = generator file's directory; state whether `find_models` surfaces generator emissions (recommend: yes). **B** keep `directory` as-is and document the intentional difference + the generator-emission gap.
**Recommendation:** A — one location vocabulary across the two reflection surfaces.
**Decision:** ____________   **Notes:** ____________

### D-26 · [Maj] Python model canonical address: bare function name vs path-derived (like SQL)
**Conflict.** Constraint 4 keys uniqueness on "the full canonical address (the function name)". But SQL models derive the address from the **file path** (`models/archive/users.sql` → `archive.users`). Equating "function name" with "full canonical address" implies a nested Python model ignores its directory prefix.
**Options.** **A (recommended)** Python model address = **directory-prefix + name**, identical to SQL models (`models/py/archive.py` returning `users` → `archive.users` or per the file path). Uniform addressing. **B** Python model address = bare function name (flat). Simpler but breaks the universal path-derived model and invites cross-directory collisions.
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

### D-27 · [Maj] `PythonModelNameMismatch` — drop *all* frontmatter, or just the `name:` key?
**Conflict.** On a `name:` ≠ function-name mismatch, the spec says "frontmatter is dropped and model defaults apply" — but the diagnostic is **Error**, so the build fails fast (architecture parity rule) and "defaults apply" is unreachable; meanwhile dropping the whole block discards legitimate `materialization`/`tags`. (Same shape flagged for the build gate in a separate Minor.)
**Options.** **A (recommended)** It's a hard Error that **blocks the build** → remove the "frontmatter dropped, defaults apply" clause (it's dead); for LSP/analysis-time, keep the model with its other keys and only flag the bad `name:`. **B** Make it recoverable (Warning) and drop only the `name:` key, not the block.
**Recommendation:** A — Error semantics should be honest; don't define unreachable recovery.
**Decision:** ____________   **Notes:** ____________

---

## Theme 5 — Type system (genuine choices only)
*(types.md. Determinate fixes — C16 decimal widening, C17 fragment-kind, C26 nullability-in-signatures, NOW() nullability, decimal-arith trigger — are in Appendix A. impl→ D-types.)*

### D-28 · [Min] VALUES-derived column temporal-family LUB
**Conflict.** Column-wise LUB for VALUES cites "§5 String unification" (a dangling/wrong ref — §5 is "Canonical built-in returns") and no section defines how to LUB `Date` with `Timestamp`, or naive vs tz-aware timestamps, in a VALUES column. The strict tz-mixing rule (§16) is scoped to UNION/CASE only.
**Options.** **A (recommended)** fix the dangling ref and **add an explicit temporal-family LUB rule** for VALUES, **applying §16's strict tz-mixing** (incompatible temporal elements → `TypeMismatch`, like cross-family). Consistency with UNION/CASE. **B** leave VALUES tz-mixing permissive (coerce to a common type) — laxer, risks silent tz bugs.
**Recommendation:** A — match the strictness already chosen for UNION/CASE.
**Decision:** ____________   **Notes:** ____________

### D-29 · [Min] `Char` in the string-equality family
**Conflict.** Prose says `Text`/`Varchar(_)`/`Char(_)` are interchangeable for type-equality, but `normalize()` only collapses `Text ↔ Varchar(None)` — `Char` is absent, and Char has distinct padding semantics in SQL.
**Options.** **A (recommended)** extend `normalize()` to **fold `Char` into the string family** for equality (matches the prose; downstream cares about family, not padding, at the type level). `impl→ D-types`. **B** scope interchangeability to `Text`/`Varchar` only and treat `Char` as distinct (honours padding semantics, but `Char(5) = Text` then fails).
**Recommendation:** A unless padding-distinctness matters to you for type-checking.
**Decision:** ____________   **Notes:** ____________

---

## Theme 6 — Project surface / config

### D-30 · [Maj] Function name-uniqueness scope: directory vs project vs workspace *(carried from A3)*
**Conflict.** functions.md Constraint 4 says `smelt.define`/`smelt.extern` share **one workspace-wide** name namespace; but Surface derives identity from directory+name (same-named defines in different dirs are unambiguous), and architecture.md says "unique **within a directory** across all kinds". `DuplicateFunctionDefinition` currently states no scope (A3 left it neutral).
**Options.**
- **A (recommended)** — **Directory-scoped** uniqueness for `smelt.define` (matches `smelt.<path>` identity + architecture.md); workspace-wide flat namespace only for **externs/built-ins**. `DuplicateFunctionDefinition` fires for two defines in the **same directory** sharing a name; a define clashing with a built-in is a separate (extern-style) error. `impl→ D-diag`.
- **B** — Workspace/project-wide bare-name uniqueness for defines (stricter; a define in any dir clashes with another of the same leaf name). Contradicts directory-derived addressing.
**Recommendation:** A — uniqueness should match the addressing model (path-derived).
**Decision:** ____________   **Notes:** ____________

### D-31 · [Maj/Min] Unknown frontmatter key severity: Warning (current) vs Error (doctrine)
**Conflict.** functions.md Constraint 6 pins unknown-key → `FrontmatterParseError` at **Warning**; architecture.md's unknown-key **doctrine requires Error**; functions.md Known Divergences calls the Warning "divergent". So a typo like `deterministc: true` is silently accepted past a warning.
**Options.** **A (recommended)** make the **doctrine win**: unknown key → **Error**; move the current-Warning behaviour to Known Divergences as the implementation gap. Fail-loud, catches typos. `impl→ D-diag`. **B** keep Warning as intended (lenient for forward-compat frontmatter), drop the "divergent" framing.
**Recommendation:** A — consistent with fail-loud; the lenient behaviour is the known gap, not the intent.
**Decision:** ____________   **Notes:** ____________

### D-32 · [Maj] Model `format` (delta|parquet) vs target-level `format` — precedence & ownership
**Conflict.** models.md has a `format` frontmatter key; smelt_yml.md defines `format` only at `targets.<name>` (Spark-only) and its precedence rules never mention `format`. No stated precedence between model-frontmatter and target `format`.
**Options.** **A (recommended)** add `format` to smelt_yml.md's model-config shape **and** §Precedence with **model-frontmatter > target** (model override wins, like materialization). **B** model `format` defers entirely to target (drop the model key). 
**Recommendation:** A — overrides should win at the model level, matching every other model key.
**Decision:** ____________   **Notes:** ____________

### D-33 · [Maj] `default_materialization` accepting `test`/`cumulative_aggregate`/`ephemeral`
**Conflict.** smelt_yml.md lists all six modes as accepted project-wide defaults. A default of `test` makes every un-annotated model a non-materialising test; `cumulative_aggregate`/`ephemeral` are similarly nonsensical as a blanket fallback.
**Options.** **A (recommended)** **restrict** `default_materialization` to `table`/`view`/`materialized_view`/`ephemeral` and **reject `test`/`cumulative_aggregate`** with a validation error. (Keep `ephemeral`? — it's defensible as a default; exclude only `test`/`cumulative`.) **B** allow all six and document the foot-gun.
**Recommendation:** A (excluding `test` + `cumulative_aggregate`; keep `ephemeral`).
**Decision:** ____________   **Notes:** ____________

### D-34 · [Maj] Orphaned top-level keys `vars:` and `state:` — who owns the declaration grammar?
**Conflict.** smelt_yml.md claims to cover top-level keys but omits both `vars:` and `state:`. meta_language.md references the `vars:` block as pre-existing (only specs the *accessor*); virtual_environments.md shows a `state:` block but disclaims owning smelt.yml grammar. No spec defines the `vars:` value grammar or `state:` as a smelt.yml key.
**Options.** **A (recommended)** add `vars:` and `state:` **rows to smelt_yml.md §Top-level keys** (structural owner), each pointing to meta_language.md / virtual_environments.md for semantics. Single structural owner; semantics stay with the feature specs. **B** leave grammar in the feature specs and have smelt_yml.md cross-link them (no rows). 
**Recommendation:** A — smelt_yml.md should own *that a key exists and its value shape*.
**Decision:** ____________   **Notes:** ____________

### D-35 · [Maj] Source `name:` override `<schema>.<table>` hardcodes schema → breaks multi-target
**Conflict.** sources.md says `name:` override "must be a `<schema>.<table>` literal", but schema otherwise comes from the active target. A literal pins one schema, so `--target dev`/`prod` both read the same hardcoded schema — defeating multi-target portability.
**Options.** **A (recommended)** `name:` overrides **only the table component**; schema still from the active target. **B** make the override **target-aware** (a per-target name map). **C** document that source schemas are environment-invariant (justify the pin).
**Recommendation:** A — simplest; if you genuinely need per-target external schemas, B.
**Decision:** ____________   **Notes:** ____________

---

## Theme 7 — CLI & selection
*(cli.md, model_selection.md. impl→ D-cli.)*

### D-36 · [Maj] Canonical-display round-trip vs "argument never carries `smelt.` prefix"
**Conflict.** Canonical-display says printed identifiers use full `smelt.<path>` and copy-pasting one back must reproduce the resolution; the same section says the argument **never** carries the leading `smelt.` prefix. A printed `smelt.silver.events_parsed` pasted back carries the prefix the rule says it never carries.
**Options.** **A (recommended)** entity arguments **accept and strip** a leading `smelt.` prefix (round-trip works; bare form also accepted). **B** printed identifiers are **prefix-less full paths** (reword Canonical-display; printing drops `smelt.`).
**Recommendation:** A — copy-paste round-trip is the more valuable invariant.
**Decision:** ____________   **Notes:** ____________

### D-37 · [Maj] Unresolvable selector: hard "not found" error vs graceful "no models matched" no-op
**Conflict.** §Argument resolution mandates a "not found" diagnostic (non-zero) when nothing resolves, and says `--select` uses the same algorithm; §No-op rebuild mandates a stderr "no models matched the selector(s)" (success). For `--select typo_name` the two require different behaviour; the empty-selection exit code is never stated.
**Options.** **A (recommended)** an **entity-name selector resolving to nothing is a hard error** (consistent with ambiguity-safety); reserve the no-op message for **valid selectors whose result set is legitimately empty** (exit 0). **B** treat all empty selections as a quiet no-op (exit 0) — friendlier, but a typo'd `--select` silently does nothing.
**Recommendation:** A — a typo should fail loudly, not silently build nothing.
**Decision:** ____________   **Notes:** ____________

### D-38 · [impl] Selector `+`/graph operators and `path:` method in cli.md ↔ model_selection.md
**Conflict.** cli.md passes selectors containing `:` through unchanged, but `+events_parsed`/`events_parsed+` contain no `:` and would be fed to entity resolution as literal names (can't resolve); and cli.md's `path:models/silver` example names a method model_selection.md's grammar doesn't define.
**Options.** **A (recommended)** strip leading/trailing `+` **before** entity resolution and re-attach to the resolved full path; **add `path:` to model_selection.md's grammar** (or replace the example with a defined method). `impl→ D-cli`. **B** require full paths with graph operators (no leaf+operator shorthand). 
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

### D-39 · [Maj] `--exclude +model` leaving an inconsistent working set
**Conflict.** `--exclude +model` removes the model **and its upstreams**, which can drop shared dependencies other selected models need; Constraint 4 still executes the set in topological order — running models against absent inputs. Currently shipped as "untested/undefined".
**Options.** **A (recommended)** excluding an upstream that a **retained model needs is an error/warning** (don't ship an inconsistent set). **B** make upstream traversal on `--exclude` **opt-in** (bare `--exclude model` removes only that model). 
**Recommendation:** A (or B if you want the convenience; A is safer).
**Decision:** ____________   **Notes:** ____________

### D-40 · [Min] cwd-scope fall-through stability hazard
**Conflict.** Scope fall-through is silent (`<scope>.<arg>` else `<arg>`). With scope `silver` and only top-level `events_parsed`, a command resolves via fall-through; later adding `silver/events_parsed.sql` silently retargets the same command — violating the stability principle the Design section claims.
**Options.** **A (recommended)** **drop the fall-through** (scoped shorthand resolves only `<scope>.<arg>`; full path required otherwise). **B** keep fall-through but **emit a notice** naming the resolved path whenever it occurs.
**Recommendation:** A — matches the "adding an entity never changes a passing command" principle cli.md states.
**Decision:** ____________   **Notes:** ____________

### D-41 · [Min] `smelt test --select` substring-match divergence
**Conflict.** `--select` on `smelt test` is a substring match on test names, not selector syntax — inconsistent with every other command (`tag:`/`+model` treated as literal substrings).
**Options.** **A (recommended)** make `smelt test --select` use **full selector syntax** (consistent everywhere). `impl→ D-cli`. **B** rename the test flag to **`--name-filter`** (keep substring match but stop overloading `--select`).
**Recommendation:** A (B if selector-on-tests is too big for now).
**Decision:** ____________   **Notes:** ____________

---

## Theme 8 — Per-spec smaller design calls

### D-42 · [C18] `testing.md` `inputs` key form: bare name vs `smelt.<path>`
**Conflict.** The example keys inputs by bare name (`orders:`); Semantics + Constraint 4 say keys are the `smelt.<path>` form. An implementer can't tell whether the key is `orders` or `smelt.orders`.
**Options.** **A (recommended)** canonical key = **bare address path** (the `smelt.<path>` minus the `smelt.` prefix, e.g. `orders` or `silver.orders`); make example/Semantics/Constraint 4 identical. **B** require the full `smelt.<path>` form as the key.
**Recommendation:** A — matches the example and is less noisy.
**Decision:** ____________   **Notes:** ____________

### D-43 · [Maj] `testing.md` empty-CTE substitution for unlisted deps (false-green foot-gun)
**Conflict.** "Dependencies not listed in `inputs` are replaced with empty CTEs" → a typo'd input key (`order` vs `orders`) becomes an empty CTE, the test passes against empty `expect`, false green — in a *testing* tool.
**Options.** **A (recommended)** **emit a diagnostic** when an `inputs` key matches no compiled dependency (catches typos); optionally require unlisted deps to be explicitly opted-into-empty via a marker. **B** warning only (non-fatal). 
**Recommendation:** A (at least a warning; Error is defensible given it's a test framework).
**Decision:** ____________   **Notes:** ____________

### D-44 · [Maj] `testing.md` DECIMAL actual comparison (money columns)
**Conflict.** Coercion maps Float→DOUBLE, Integer→INTEGER, **no DECIMAL**; `SUM(amount)` yields DECIMAL; the `1e-6` float tolerance never says whether it applies to DECIMAL actuals or how a DOUBLE `300.0` compares to a DECIMAL `300.00`.
**Options.** **A (recommended)** expected values compared **by numeric value with the `1e-6` tolerance regardless of actual SQL type** (covers DECIMAL/DOUBLE/INTEGER uniformly). **B** add an exact-compare path for DECIMAL (no tolerance) and a DECIMAL coercion row.
**Recommendation:** A — simplest predictable rule for money tests; B if you need exact decimal equality.
**Decision:** ____________   **Notes:** ____________

### D-45 · [Maj] `testing.md` CTE-level test transitive-dependency closure
**Conflict.** A CTE-level test mocks *direct* upstream CTEs; if a target CTE references a direct upstream that references a further-upstream CTE, it's undefined whether the transitive one is executed as-written or must also be mocked.
**Options.** **A (recommended)** only **directly-referenced** CTEs are mockable/required; all **transitively-needed** CTEs execute as-written. **B** the full transitive set must be mocked.
**Recommendation:** A — least surprising; you mock your direct boundary.
**Decision:** ____________   **Notes:** ____________

### D-46 · [Maj] `virtual_environments.md` split `accept_current` vs `assert_deterministic`
**Conflict.** Reuse condition 3 lumps two hatches with opposite guarantees: `assert_deterministic` asserts the model *is* deterministic (reuse = rebuild-identical if true); `accept_current` accepts reuse for a *known non-deterministic* model (reuse ≠ rebuild). One condition, two contracts.
**Options.** **A (recommended)** **split** condition 3 into 3a (deterministic OR `assert_deterministic` ⇒ rebuild-identity preserved) and 3b (`accept_current` ⇒ non-deterministic, output-preserving reuse without rebuild-identity), each with its own logged-trust note; and split the rebuild-identity invariant accordingly. **B** keep them merged, add a clarifying sentence.
**Recommendation:** A — the soundness stories genuinely differ.
**Decision:** ____________   **Notes:** ____________

### D-47 · [Min] `virtual_environments.md` posture lattice + candidate-table lookup
**Conflict.** (i) "A model may narrow but not widen the project posture" never defines the ordering or what narrowing means for reuse. (ii) Reuse condition 2 needs `fingerprint(M) == fingerprint(T.source)` but never says how candidate table `T` is located (which environments, precedence, where `T.source`'s fingerprint is persisted).
**Options.** **A (recommended)** define the **posture lattice `environments ⊇ intervals ⊇ stateless`** explicitly + state a narrowed-to-`stateless` model opts out of reuse; and name the **fingerprint→table index** consulted (cite run_state.md) with a precedence rule for multiple candidates. **B** defer the lookup to run_state.md by reference only.
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

### D-48 · [Maj] `lsp.md` watched-file set (model `.sql`, derived globs) + cross-file republication
**Conflict.** Watch globs hardcode `**/models/**` and `**/functions/**` (but `paths:` is configurable and defines may live anywhere); model `.sql` files aren't watched (external `git checkout` edits missed); and "diagnostics published on every file change" never says *whose* (upstream change can stale a downstream file's diagnostics).
**Options.** **A (recommended)** derive watch globs from the **loaded project's `paths:`** + watch function-bearing files wherever defines resolve + **add model `.sql`** to the watched set; and republish diagnostics for **the changed file plus every file whose Salsa-derived diagnostics changed** (≥ all open files in the same project). `impl→ D-diag`. **B** document the gaps as Known Divergences (cheaper, leaves the holes).
**Recommendation:** A — these are correctness/freshness holes, not cosmetics.
**Decision:** ____________   **Notes:** ____________

### D-49 · [Maj] `lsp.md` column-rename scope (destructive multi-file edit)
**Conflict.** Rename traversal ("local, upstream, downstream") leaves open: downstream of *what* (invocation file vs definition site)? Does re-aliasing terminate propagation? And rename rewrites **source `.yml`** columns for externally-managed tables — turning the declaration green while every runtime query against the real external table breaks.
**Options.** **A (recommended)** traversal **rooted at the resolved definition site**; all transitive consumers rewritten; `AS` re-aliasing terminates propagation; `SELECT *` chains propagate. **And** for a **source column**, `prepare_rename` responds **not-supported** (the table is external — refuse, with an explanatory message). **B** allow source rename but require explicit user confirmation that it's declaration-only.
**Recommendation:** A — refuse source-column rename; it can't be safe.
**Decision:** ____________   **Notes:** ____________

### D-50 · [Maj] `data_catalog.md` lineage shapes — unknown-lineage, `--select` exclusion, `path` portability
**Conflict.** Three under-specified JSON shapes: (i) unknown column lineage is `source.type:"unknown"` in one place, "omitted" in another; (ii) what happens to `upstream`/`downstream`/`tag_index`/links for `--select`-excluded models is unspecified; (iii) `path` is `<absolute path>` (machine-specific; contradicts the spec's own workspace-relative usage — can't diff across CI).
**Options.** **A (recommended)** (i) **always present, `source:{type:"unknown"}`** when undeterminable (safer for consumers); (ii) **edge arrays retain excluded names** (full lineage) but `models`/`tag_index`/`execution_order`/`model_count` contain only selected models, markdown renders excluded deps as plain text; (iii) `path` is **workspace-relative**. **B** (i) omit unknown lineage; (ii) drop excluded names from edges. 
**Recommendation:** A — stable, diffable, lineage-preserving contract for orchestrator consumers.
**Decision:** ____________   **Notes:** ____________

### D-51 · [Maj] `run_state.md` interval ledger granularity (date-only vs sub-day)
**Conflict.** The interval ledger uses calendar-date string keys; incremental models routinely filter on hourly/second event-time boundaries. A date-only ledger can't record or gap-detect sub-day windows. `↔R2` (incremental cadence).
**Options.** **A (recommended)** specify keys as **RFC3339 instants** (sub-day capable); coordinate with R2's cadence model. **B** keep **date-only** as a deliberate, documented limitation (Constraints + cross-link incremental_models.md) — only if sub-day incremental is out of scope for now.
**Recommendation:** A, coordinated with R2 — but if sub-day incremental isn't a near-term goal, B is acceptable. Mark `↔R2`.
**Decision:** ____________   **Notes:** ____________

### D-52 · [Maj] `timeseries.md` partition/event-time nullability + granularity-vs-partition-type
**Conflict.** (i) No rule constrains nullability of `partition_column`/`event_time_column`; a NULL partition value silently escapes `>= start AND < end` pruning (never deleted/re-inserted) — a correctness hole for incremental. (ii) `granularity: hour` with a `DATE` partition_column can't represent hour boundaries, yet nothing forbids it. `↔R2`.
**Options.** **A (recommended)** add invariants: partition_column (and event_time when it drives pruning) **must be NOT NULL** on output/source → else `MalformedTimeseries`; and **sub-day granularity requires a timestamp-resolution partition type** (not plain `date`) → else `MalformedTimeseries`. `impl→ D-incr`. **B** define explicit NULL-row handling + silent coarsening (laxer, more failure modes).
**Recommendation:** A — fail-loud on the combinations that silently corrupt incremental output. Coordinate with R2.
**Decision:** ____________   **Notes:** ____________

### D-53 · [Maj] `datagen.md` scale-factor pool-invariance + FK bounds at scale ≠ 1
**Conflict.** Design claims `--scale-factor` keeps the device/user universe identical "for comparability", but shape-level `foreign_key` resolves against scaled `fk_counts`, so pool contents change under scaling; and FK bound adjustment is specified only for `scale_factor < 1` (behaviour at >1 and pool-ratio scaling left to guess; `floor` can yield 0 → `[1,0]`).
**Options.** **A (recommended)** **qualify scale-invariance**: pool contents are scale-invariant **only if no shape field uses `foreign_key`** (state in Semantics + Constraint 6); FK bounds always equal the referenced dataset's **effective (scaled) row count for any scale factor**; define the error when an effective row count or pool size is 0. **B** make pool-level FK resolve against **unscaled** counts (breaks referential integrity at scale<1 — not recommended).
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

### D-54 · [Maj] `expansion.md` `Caller(span)` provenance identity under nested expansion
**Conflict.** `Caller(span)` carries no file/fn identity (unlike `Callee(fn_id, span)`). In nested expansion (model → A → B), the "caller" of B is A's body in a different file; a bare span can't be resolved to a file, and the spec never says whether nested expansion re-tags argument subtrees.
**Options.** **A (recommended)** tags are assigned **once relative to original source**; expansion of B **leaves prior tags intact**, so `Caller` only ever denotes the root model file (state this rule explicitly). `impl→ D-misc` if code differs. **B** add a file/fn identity to `Caller` (richer, but a struct change).
**Recommendation:** A — simpler invariant, preserves source-mapping to user code.
**Decision:** ____________   **Notes:** ____________

### D-55 · [Min] `meta_config_loading.md` record overlay: deep merge vs shallow replace for nested records
**Conflict.** Labeled "field-by-field **deep merge**" but defined as "an overlay's value for a field **replaces** the base's value" (shallow). For a nested-record field the two diverge.
**Options.** **A (recommended)** **shallow replace** (drop the word "deep"): an overlay field replaces the base field wholesale, including nested records. Simpler, matches the per-target override intent. **B** recursive deep merge for nested record sub-fields (give a nested example; scalars/lists/maps replace, record sub-fields recurse).
**Recommendation:** A — overlays as wholesale field overrides is the simpler mental model.
**Decision:** ____________   **Notes:** ____________

### D-56 · [Min] `meta_config_loading.md` hover exposes file mtime vs no-clock determinism
**Conflict.** Hover "shows the file's last-modified timestamp", but §Semantics asserts "No clock" / deterministic re-evaluation keyed on `(file bytes, schema, target)`. mtime isn't a function of bytes; surfacing it risks registering mtime as a Salsa input.
**Options.** **A (recommended)** **drop mtime from hover** (cleanest; nothing else needs it). **B** keep it but explicitly carve it out as **presentation-only**, never part of the loaded value or any Salsa input.
**Recommendation:** A.
**Decision:** ____________   **Notes:** ____________

### D-57 · [Min] `planner_integration.md` `joins:` cardinality string→enum mapping (load-bearing soundness gate)
**Conflict.** Semantics 8 gates the only soundness-bearing rewrite (`EliminateUnusedLeftJoin`) on `cardinality == OneToOne`, but Known Divergences says cardinality is a raw string with "mapping rule unspecified — likely exact match, no normative claim". The soundness gate rests on an undefined string→enum mapping.
**Options.** **A (recommended)** make it normative: **exact spelling `1:1` maps to OneToOne**; any unrecognised string maps to a non-OneToOne value that **never enables elision** (fail-safe). `impl→ D-incr/planner`. **B** accept a small set of spellings (`1:1`, `one_to_one`) — more lenient, more parsing.
**Recommendation:** A — exact-match + fail-safe default is the safe gate.
**Decision:** ____________   **Notes:** ____________

### D-58 · [Min] `schema_evolution.md` NOT NULL reclassification (`default:` vs `backfill:` vs both)
**Conflict.** "Add NOT NULL column without `default:` → Blocked"; but Surface also defines `backfill:` (the UPDATE that populates existing rows). Whether `backfill:` **alone** (no `default:`) makes a NOT NULL add Safe is unspecified.
**Options.** **A (recommended)** **either `default:` or `backfill:`** (or both) reclassifies a NOT NULL add as Safe (both populate existing rows); state it and what fires when neither is present. **B** require `default:` specifically (backfill is additive only). 
**Recommendation:** A — backfill populates rows, so it should satisfy the NOT NULL requirement.
**Decision:** ____________   **Notes:** ____________

---

## Appendix A — determinate fixes (Track B; veto-only, no deliberation needed)

These have a single correct answer per the review; they'll be applied as worded unless you object. Listed so nothing is invisible.

- **[C16] schema_evolution DECIMAL widening** — require `s2≥s` **and** `(p2−s2)≥(p−s)` (integer-digit capacity must not shrink). Pure correctness. `impl→ D-types`.
- **[C17] scoping FragmentKindMismatch direction** — reverse the example: fires when a fragment's kind is *higher* than the splice point admits (Agg/Window where Scalar-only). `impl→ D-types`.
- **[C26] gradual_typing nullability-in-signatures** — replace "No nullability in user signatures" with a pointer to types.md §11 (`NOT NULL` is opt-in on top-level param/return; bare types stay nullable).
- **[Maj] types NOW()/CURRENT_TIMESTAMP** — add a non-nullable origin in §11 for registry-declared non-nullable nullary built-ins (or cross-ref §16).
- **[Min] types decimal-arithmetic trigger** — integer lifting applies only when **at least one operand is already Decimal-family**.
- **[C24] cli seed step** — replace `read_csv_auto` recipe with "run the seed lifecycle per seeds.md (`Backend::load_table`); ephemeral seeds skipped; sources never loaded".
- **[Maj] cumulative `--start/--end`** → `--event-time-start`/`--event-time-end` (match incremental). *(cli-flag wording; the cumulative rewrite R1 owns deeper CLI semantics.)*
- **[Maj] partition-column projection rule** — delete the duplicate in incremental_models.md, link timeseries.md rule 1 as owner. *(coordinate with R2.)*
- **[Maj] data_catalog/cli explain enums** — add `cumulative_aggregate` to the materialization enums.
- **[Maj] meta_language `UnknownColumn`** → `UndeclaredColumn` (use the catalogued code).
- **[Maj] cumulative idempotency parenthetical** — re-merge converges only for idempotent combiners (MIN/MAX/BOOL_*/BIT_AND/BIT_OR); SUM/COUNT/BIT_XOR are not idempotent. *(R1 owns the surrounding rewrite.)*
- **[Min] expansion CteShadowsCallerCte** — name the dedicated code (drop the `CteCycle`-family hedge); detected at check time, anchored at the call site (defer anchor to scoping.md).
- **[Min] seeds invariant 1** — reword "cannot diverge by construction" → "shared code path; output may differ solely by sample size".
- **[Min] sources Constraint 6 / Constraint 7** — soften the hard-migration-error claim to match its own Known Divergence; drop the "sources namespace" framing (single `smelt.<path>` namespace).
- **[Min] data_catalog `generated_at` / "deterministic"** — reword Constraint 3 to "deterministic key ordering"; note `generated_at` is the one intentionally non-deterministic field.
- **Template conformance (A4 mechanical part)** — add diagnostics.md's scope-callout blockquote; standardise `**What this is.**`; fix the two timeless-oracle "Phase 42"/"as of Phase 4" References notes; add missing Out-of-scope pointers flagged per spec.
- *(…and the remaining `[Min]`/`[Nit]` wording fixes from the review's Minor/Nit section, applied per-spec in Track B.)*

## Appendix B — handled by the two rewrites (R1/R2), not here

For visibility — these design decisions are made inside the spec rewrites:

- **R1 `cumulative_aggregate`:** merged-partition state/ledger mechanism & retry-after-partial-failure (C3); classifier gaps — HAVING/DISTINCT/LIMIT/ORDER BY/set-ops (C4); `state.mode` dependency (C25); NULL-aware cross-partition combine; NULL `unique_key` merge matching; driving-source self-reference; GROUP BY ordinals/expressions; `merge_into` primitive home (C23, ↔architecture §Backend trait surface — see also D-02's collision model); reversible-aggregator `--auto`/staleness.
- **R2 `incremental_models`:** write-window vs run-window single source of truth (C7); write-skew bound derivation (C8); chained-session classification (C9); unified window-admission rule (C10); strategy-choice correctness constraint (Append/Merge); chunk-sizing rule; per-source `n` vs pushdown; post-override classification. Touches D-11 (`IncrementalNotBatchSafe`), D-51, D-52.
