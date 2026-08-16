
"use strict";
const DATA = JSON.parse(document.getElementById("atlas-data").textContent);

const TECHS = ["DeleteInsert", "KeyedFold", "ColumnScopedMerge", "InPlaceUpdate"];
const TECH_VAR = { DeleteInsert: "--t-di", KeyedFold: "--t-kf", ColumnScopedMerge: "--t-csm", InPlaceUpdate: "--t-ipu" };
const TECH_BLURB = {
  DeleteInsert: "Recompute the affected region and swap it in: DELETE the window, re-INSERT it from source. The universal fallback — always admissible, never wrong, pays full region cost.",
  KeyedFold: "Fold only the new rows into keyed state through the column-family combiner (SUM, MAX, MAX_BY…), guarded by the reconciliation ledger so a window is never folded twice.",
  ColumnScopedMerge: "MERGE only the columns a mutated dimension actually feeds, keyed by the model's unique key. Needs proven row identity and membership sensitivity pruned by a closure proof.",
  InPlaceUpdate: "UPDATE existing rows in place — the narrowest write. Admitted for triggers like a produced ColumnAdded where row membership provably cannot change.",
};
const RECIPE_STORY = {
  "recipe:value_enriched": {
    title: "Closure-pruned enrichment",
    story: "A LEFT JOIN against a dimension that declares its own unique_key. The skeleton-source closure proof shows the join can never add or drop fact rows, so the dimension's mutation cell is value-only — and ColumnScopedMerge is admitted for exactly the columns the dimension feeds.",
  },
  "recipe:mutable_enriched": {
    title: "Open-closure enrichment (the contrast)",
    story: "The same fact + dimension shape, but an INNER JOIN with no closure proof: a dimension mutation can add or remove fact rows, so membership sensitivity is row-scoped and every cell falls back to DeleteInsert. Same SQL family, opposite verdict — this pair is the prover in one picture.",
  },
  "recipe:keyed_additive": {
    title: "Additive keyed fold",
    story: "SUM over an invertible commutative group. KeyedFold with the Additive ledger grade: entries record delta identities, and re-folding an already-processed window is refused loudly (KeyedReprocessedWindow) instead of double-counting.",
  },
  "recipe:keyed_order_monotone": {
    title: "Order-monotone overwrite",
    story: "MAX_BY(val, d) — last-value-wins along a proven-monotone ordering. Classified into the order-monotone overwrite family with the Idempotent ledger grade: re-merging an already-reflected delta converges.",
  },
  "recipe:keyed_snapshot_overwrite": {
    title: "Snapshot-reconcile overwrite",
    story: "ANY_VALUE under the snapshot-reconcile run shape — the newest run shape. Incoming-row-wins is idempotent by construction, so the executor keeps no ledger at all, and the reconcile lands as a ColumnScopedMerge.",
  },
};

function planModels(proj) {
  const out = [];
  for (const [name, m] of Object.entries(DATA.projects[proj].models)) {
    if (!m.no_plan) out.push(name);
  }
  return out.sort();
}
function modelTechs(plan) {
  return [...new Set(plan.cells.map(c => c.technique))];
}
function esc(s) {
  return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
function techChip(t, extra) {
  return `<span class="chip t-${t}${extra ? " " + extra : ""}"><span class="dot" style="background:var(${TECH_VAR[t]})"></span>${t}</span>`;
}
function parseTrigger(tr) {
  const m = tr.match(/^(\w+)(?:\s*\{\s*source:\s*"([^"]+)"\s*\})?/);
  return m ? { kind: m[1], source: m[2] || null } : { kind: tr, source: null };
}
function highlightSql(sql) {
  const compact = sql.replace(/^--[ \t]*$\n?/gm, "").replace(/\n{3,}/g, "\n\n");
  return esc(compact).replace(
    /\b(DELETE|INSERT INTO|INSERT|UPDATE|MERGE INTO|MERGE|SELECT|FROM|WHERE|JOIN|LEFT JOIN|INNER|ON|AND|OR|GROUP BY|ORDER BY|VALUES|SET|WHEN MATCHED|WHEN NOT MATCHED|THEN|USING|AS|CAST|CREATE TABLE|NOT|NULL|IN|EXISTS|BEGIN|COMMIT|SUM|MAX|MIN|COUNT|ANY_VALUE|ARG_MAX|COALESCE)\b/g,
    '<span class="kw">$1</span>');
}

/* ---------- sidebar ---------- */
function renderSide(activeProj, activeModel) {
  const el = document.getElementById("side");
  let h = `<div class="brand"><a href="#/"><div class="word"><b>smelt</b> · Maintenance Atlas</div>
    <div class="sub">the plan prover, mapped</div></a></div>`;
  for (const proj of Object.keys(DATA.projects)) {
    const withPlans = planModels(proj);
    const total = Object.keys(DATA.projects[proj].graph.models).length;
    const label = proj.startsWith("recipe:") ? "specimen · " + proj.slice(7) : "examples/" + proj;
    if (!withPlans.length && proj.startsWith("recipe:")) continue;
    h += `<div class="grp"><div class="grp-h">${esc(label)}</div>`;
    for (const name of withPlans) {
      const plan = DATA.projects[proj].models[name];
      const on = proj === activeProj && name === activeModel;
      const dots = modelTechs(plan).map(t => `<span class="dot" style="background:var(${TECH_VAR[t]})"></span>`).join("");
      h += `<a class="mrow${on ? " on" : ""}" href="#/m/${encodeURIComponent(proj)}/${encodeURIComponent(name)}">
        <span class="nm">${esc(name)}</span><span class="dots">${dots}</span></a>`;
    }
    const rest = total - withPlans.length;
    if (rest > 0) h += `<div class="rest">+ ${rest} model${rest === 1 ? "" : "s"} without a maintenance plan</div>`;
    h += `</div>`;
  }
  el.innerHTML = h;
}

/* ---------- overview ---------- */
function censusCounts() {
  const byTech = Object.fromEntries(TECHS.map(t => [t, 0]));
  let cells = 0, models = 0, refusals = 0;
  for (const proj of Object.keys(DATA.projects)) {
    for (const name of planModels(proj)) {
      models++;
      const plan = DATA.projects[proj].models[name];
      for (const c of plan.cells) {
        cells++;
        byTech[c.technique] = (byTech[c.technique] || 0) + 1;
        for (const p of c.technique_previews || []) {
          if (p.admissibility && p.admissibility.verdict !== "admitted") refusals++;
        }
      }
    }
  }
  return { byTech, cells, models, refusals };
}

function dagSvg(proj) {
  const models = DATA.projects[proj].graph.models;
  const names = Object.keys(models);
  if (!names.length) return "";
  const depth = {};
  const seen = new Set();
  function d(n) {
    if (n in depth) return depth[n];
    if (seen.has(n)) return 0;
    seen.add(n);
    const deps = (models[n] && models[n].dependencies) || [];
    const ds = deps.filter(x => models[x]).map(d0 => d(d0));
    depth[n] = ds.length ? Math.max(...ds) + 1 : 0;
    return depth[n];
  }
  names.forEach(d);
  const cols = {};
  names.sort().forEach(n => { (cols[depth[n]] = cols[depth[n]] || []).push(n); });
  const colKeys = Object.keys(cols).map(Number).sort((a, b) => a - b);
  const NW = 168, NH = 26, GX = 70, GY = 12, PAD = 10;
  const maxRows = Math.max(...colKeys.map(k => cols[k].length));
  const H = PAD * 2 + maxRows * (NH + GY) - GY;
  const W = PAD * 2 + colKeys.length * (NW + GX) - GX;
  const pos = {};
  for (const k of colKeys) {
    const colH = cols[k].length * (NH + GY) - GY;
    cols[k].forEach((n, i) => {
      pos[n] = { x: PAD + k * (NW + GX), y: PAD + (H - 2 * PAD - colH) / 2 + i * (NH + GY) };
    });
  }
  let edges = "", nodes = "";
  for (const n of names) {
    for (const dep of (models[n].dependencies || [])) {
      if (!pos[dep] || dep === n) continue; // self-edge (e.g. a chained sessionizer) — the loop marker below covers it
      const a = pos[dep], b = pos[n];
      const x1 = a.x + NW, y1 = a.y + NH / 2, x2 = b.x, y2 = b.y + NH / 2;
      const mx = (x1 + x2) / 2;
      edges += `<path class="dag-edge" d="M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}" marker-end="url(#dagarrow-${cssId(proj)})"/>`;
    }
  }
  for (const n of names) {
    const p = pos[n];
    const inc = !!models[n].incremental;
    const plan = DATA.projects[proj].models[n];
    const hasPlan = plan && !plan.no_plan;
    const stroke = inc ? "var(--accent)" : "var(--line-2)";
    const label = n.length > 22 ? n.slice(0, 21) + "…" : n;
    nodes += `<g class="dag-node${inc ? " inc" : ""}${hasPlan ? " link" : ""}" ${hasPlan ? `data-href="#/m/${encodeURIComponent(proj)}/${encodeURIComponent(n)}"` : ""}>
      <rect x="${p.x}" y="${p.y}" width="${NW}" height="${NH}" style="stroke:${stroke}"></rect>
      <text x="${p.x + 9}" y="${p.y + NH / 2 + 3.5}">${esc(label)}</text>
      ${inc ? `<circle cx="${p.x + NW - 11}" cy="${p.y + NH / 2}" r="3.5" fill="var(--accent)"></circle>` : ""}
      ${(models[n].dependencies || []).includes(n) ? `<text x="${p.x + NW - 22}" y="${p.y + NH / 2 + 3.5}" style="fill:var(--ink-3)" aria-label="reads its own prior state">↺</text>` : ""}
    </g>`;
  }
  return `<svg viewBox="0 0 ${W} ${H}" style="min-width:${Math.min(W, 980)}px;max-width:100%;height:auto" role="img"
    aria-label="Dependency graph of ${esc(proj)}; incremental models are marked with a copper dot">
    <defs><marker id="dagarrow-${cssId(proj)}" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto">
      <path d="M0,0 L8,4 L0,8 z" fill="var(--line-2)"/></marker></defs>
    ${edges}${nodes}</svg>`;
}
function cssId(s) { return s.replace(/[^a-z0-9]/gi, "_"); }

function renderOverview() {
  const { byTech, cells, models, refusals } = censusCounts();
  let h = `
  <p class="eyebrow">derived, never declared</p>
  <h1>The maintenance plan, made visible</h1>
  <p class="lede">For every incremental model, smelt's prover derives a <em>maintenance plan</em>: it splits the
  model's columns into cells by what can change upstream, then admits the cheapest write technique it can
  <em>prove</em> equivalent to a full refresh. This atlas maps every plan across the example workspaces and the
  conformance testkit's staged specimens — every verdict below, including each refusal and its reason, is the
  prover's own output via <code>smelt explain --json --show-sql</code>.</p>

  <div class="census">
    <div class="stat"><div class="v">${models}</div><div class="k">maintained models</div></div>
    <div class="stat"><div class="v">${cells}</div><div class="k">plan cells</div></div>
    <div class="stat"><div class="v">${TECHS.filter(t => byTech[t]).length}</div><div class="k">techniques in play</div></div>
    <div class="stat"><div class="v">${refusals}</div><div class="k">explained refusals</div></div>
  </div>
  <div class="censusbar" role="img" aria-label="Share of cells by admitted technique">`;
  for (const t of TECHS) {
    if (!byTech[t]) continue;
    h += `<div style="width:${(100 * byTech[t] / cells).toFixed(1)}%;background:var(${TECH_VAR[t]})" title="${t}: ${byTech[t]}"></div>`;
  }
  h += `</div>
  <h2>Field guide to the techniques</h2>
  <p class="lede">Each plan cell admits exactly one technique — the narrowest write the proofs support.</p>
  <div class="guide">`;
  for (const t of TECHS) {
    h += `<div class="gcard t-${t}"><h3>${techChip(t)} <span class="n">${byTech[t]} cell${byTech[t] === 1 ? "" : "s"}</span></h3>
      <p>${TECH_BLURB[t]}</p></div>`;
  }
  h += `</div>

  <h2>Specimen bench</h2>
  <p class="lede">Five shapes staged from the conformance testkit's recipes — the same fixtures the
  oracle gate drives through real DuckDB runs. Together they reach the techniques the shipped examples don't.</p>
  <div class="specs">`;
  for (const [proj, story] of Object.entries(RECIPE_STORY)) {
    const p = DATA.projects[proj];
    if (!p) continue;
    const name = planModels(proj)[0];
    if (!name) continue;
    const plan = p.models[name];
    h += `<div class="spec"><h3>${esc(story.title)}</h3>
      <div class="row">${modelTechs(plan).map(t => techChip(t)).join("")}</div>
      <p>${esc(story.story)}</p>
      <div><a class="mono" href="#/m/${encodeURIComponent(proj)}/${encodeURIComponent(name)}">open ${esc(name)} →</a></div>
    </div>`;
  }
  h += `</div>

  <h2>examples/web_analytics — the maintenance demo, mapped</h2>
  <p class="lede">The bronze → silver → gold → marts pipeline behind the web-analytics tutorial series:
  nine of thirteen models carry maintenance plans, including the composed keyed+timeseries dedupe stage
  (<code>silver.events_deduped</code>) and the self-referential sessionizer chain. Copper dot = incremental;
  click a planned model to open its cells.</p>
  <div class="dagbox"><figure>${dagSvg("web_analytics")}
    <figcaption>Model-to-model dependencies in <code>examples/web_analytics</code>; models reading only external sources show no inbound edge here.</figcaption>
  </figure></div>

  <h2>examples/timeseries — the working atlas</h2>
  <p class="lede">Seven of seventeen models carry maintenance plans (copper dot = incremental). Click a
  planned model to open its cells.</p>
  <div class="dagbox"><figure>${dagSvg("timeseries")}
    <figcaption>Model-to-model dependencies in <code>examples/timeseries</code>; models reading only external sources show no inbound edge here.</figcaption>
  </figure></div>

  <h2>examples/retail_analytics — a maintenance frontier</h2>
  <p class="lede">Twenty-five TPC-DS models, staging → intermediate → marts — and not one maintenance plan yet:
  every model full-refreshes. This DAG is what the prover has left to claim.</p>
  <div class="dagbox"><figure>${dagSvg("retail_analytics")}
    <figcaption>Dependency graph of <code>examples/retail_analytics</code>. No incremental grain is declared anywhere in this project today.</figcaption>
  </figure></div>

  <p class="note" style="margin-top:34px">Data harvested ${esc(DATA.generated_at || "")} from
  <code>smelt explain</code> on branch <code>${esc(DATA.branch || "")}</code>. Statements shown are the pure
  emitters' output with symbolic <code>{{window_start}}</code>/<code>{{window_end}}</code> bounds — the exact
  SQL a run executes (statement-parity gate).</p>`;
  document.getElementById("view").innerHTML = h;
  wireDagLinks();
}

function wireDagLinks() {
  for (const g of document.querySelectorAll(".dag-node.link")) {
    g.addEventListener("click", () => { location.hash = g.dataset.href.slice(1); });
  }
}

/* ---------- model view ---------- */
function detLevelChip(level) {
  const cls = level === "Clean" ? "good" : (level === "Unknown" ? "bad" : "warn");
  return `<span class="chip ${cls}">${esc(level)}</span>`;
}

function renderModel(proj, name) {
  const p = DATA.projects[proj];
  const plan = p && p.models[name];
  if (!plan || plan.no_plan) { renderOverview(); return; }
  const g = p.graph.models[name] || {};
  const inc = g.incremental || {};
  const ct = plan.contract || {};
  const clock = ct.clock || {};
  const story = RECIPE_STORY[proj];

  let h = `<div class="mhead">
    <p class="eyebrow">${esc(proj.startsWith("recipe:") ? "testkit specimen" : "examples/" + proj)}</p>
    <h1 class="mono" style="font-family:var(--mono);font-size:26px">${esc(name)}</h1>`;
  if (story) h += `<p class="lede">${esc(story.story)}</p>`;
  h += `<div class="contract">
      <span class="kv"><b>grain</b>${esc(ct.derived_grain || "—")}</span>
      <span class="kv"><b>partition</b>${esc(clock.partition_column || "—")}</span>
      <span class="kv"><b>event time</b>${esc(clock.event_time_column || "—")}</span>
      <span class="kv"><b>granularity</b>${esc(clock.granularity || "—")}</span>`;
  if (inc.unique_key) h += `<span class="kv"><b>unique key</b>${esc(inc.unique_key.join(", "))}</span>`;
  if (inc.batch_safety) h += `<span class="kv"><b>batch safety</b>${esc(inc.batch_safety)}</span>`;
  h += `</div>`;

  if ((plan.inbound_edges || []).length) {
    h += `<div class="edges">`;
    for (const e of plan.inbound_edges) {
      const ec = e.contract || {};
      h += `<div class="edge"><span class="arr">→</span><span>${esc(e.name)}</span>
        <span class="chip plain">${esc(e.provider)}</span>
        <span class="chip plain">grain: ${esc(ec.derived_grain || "?")}</span>
        ${ec.identity ? `<span class="chip plain">identity: ${esc(ec.identity.join(", "))}</span>` : ""}</div>`;
    }
    h += `</div>`;
  }
  h += `</div>`;

  h += `<h2>Plan cells (${plan.cells.length})</h2>
  <p class="lede">One cell per (column group × trigger). The admitted technique won its cell's tournament;
  every loser's refusal reason is the prover's, verbatim.</p>`;

  plan.cells.forEach((c, ci) => {
    const tr = parseTrigger(c.trigger);
    const ex = c.extras || {};
    h += `<section class="cell">
      <div class="cell-h">
        <span class="grp">${esc(c.group)}</span>
        <span class="tr">on <b>${esc(tr.kind)}</b>${tr.source ? ` · ${esc(tr.source)}` : ""}</span>
        <span class="spacer"></span>
        ${techChip(c.technique)}
      </div>
      <div class="cell-meta">
        <div><div class="k">corner</div><div class="v">${esc(c.corner)}</div></div>
        <div><div class="k">row identity</div><div class="v">${esc(c.row_identity)}</div></div>
        <div><div class="k">ledger catch-up</div><div class="v">${esc(ex.ledger_catch_up || "—")}</div></div>
        <div><div class="k">write patterns</div><div class="v">${esc(ex.write_patterns || "—")}</div></div>
        ${ex.scan_clamps && ex.scan_clamps !== "(none)" ? `<div><div class="k">scan clamps</div><div class="v">${esc(ex.scan_clamps)}</div></div>` : ""}
        ${ex.write_pin && ex.write_pin !== "(none)" ? `<div><div class="k">write pin</div><div class="v">${esc(ex.write_pin)}</div></div>` : ""}
        ${ex.locality ? `<div class="wide"><div class="k">locality</div><div class="v">${esc(ex.locality)}</div></div>` : ""}
      </div>
      <div class="tourney">
        <div class="t-h">technique tournament</div>
        <div class="tourney-grid">`;
    for (const t of TECHS) {
      const prev = (c.technique_previews || []).find(x => x.technique === t);
      if (!prev) continue;
      const win = prev.admissibility && prev.admissibility.verdict === "admitted";
      h += `<div class="tent${win ? " win" : ""}">
        <div class="t-name"><span class="dot" style="background:var(${TECH_VAR[t]})"></span>${t}
          <span class="verdict">${win ? "✓ admitted" : "✗ refused"}</span></div>
        ${win ? "" : `<div class="why">${esc((prev.admissibility && prev.admissibility.reason) || "not applicable")}</div>`}
      </div>`;
    }
    h += `</div></div>
      <details class="sqlbox">
        <summary>executed statements — the pure emitters' output (${c.statements.length})</summary>
        <div class="sqlwrap">`;
    for (const s of c.statements) {
      h += `<div><div class="txg">transactional group ${s.transactional_group}</div>
        <pre class="sql">${highlightSql(s.sql)}</pre></div>`;
    }
    h += `</div></details>
    </section>`;
  });

  const props = plan.properties;
  if (props && (props.determinism || []).length) {
    h += `<h2>Column properties</h2>
    <p class="lede">The walk's folded verdicts per output column — these feed admission upstream of the tournament.</p>
    <div class="tblwrap"><table class="props">
      <thead><tr><th>column</th><th>determinism</th><th>comparability</th></tr></thead><tbody>`;
    const comp = Object.fromEntries((props.comparability || []).map(x => [x.output, x.comparability]));
    for (const dt of props.determinism) {
      h += `<tr><td>${esc(dt.output)}</td><td>${detLevelChip(dt.level)}</td>
        <td>${esc(comp[dt.output] || "—")}</td></tr>`;
    }
    h += `</tbody></table></div>
    <div class="flags">
      <span class="chip ${props.has_fan_out_join ? "warn" : "plain"}">fan-out join: ${props.has_fan_out_join ? "yes" : "no"}</span>
      <span class="chip ${props.has_set_op_barrier ? "warn" : "plain"}">set-op barrier: ${props.has_set_op_barrier ? "yes" : "no"}</span>
      <span class="chip plain">row identity: ${esc((props.row_identity && props.row_identity.identity) || "—")}</span>
      ${(props.grain && props.grain.keys && props.grain.keys.length) ? `<span class="chip plain">grain keys: ${esc(props.grain.keys.join(", "))}</span>` : ""}
    </div>`;
  }

  document.getElementById("view").innerHTML = h;
}

/* ---------- router ---------- */
function route() {
  const hash = location.hash || "#/";
  const m = hash.match(/^#\/m\/([^/]+)\/([^/]+)$/);
  if (m) {
    const proj = decodeURIComponent(m[1]), name = decodeURIComponent(m[2]);
    renderSide(proj, name);
    renderModel(proj, name);
  } else {
    renderSide(null, null);
    renderOverview();
  }
  window.scrollTo(0, 0);
}
window.addEventListener("hashchange", route);
route();
