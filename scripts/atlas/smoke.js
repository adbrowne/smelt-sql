// Smoke-run the atlas's app.js against its embedded data with a minimal DOM
// shim, to catch template/app.js/data-shape breakage without a browser.
// Usage: node scripts/atlas/smoke.js <path-to-atlas.html>
const fs = require("fs");

const htmlPath = process.argv[2];
if (!htmlPath) {
  console.error("usage: node smoke.js <path-to-atlas.html>");
  process.exit(2);
}
const html = fs.readFileSync(htmlPath, "utf8");
const dataMatch = html.match(/<script type="application\/json" id="atlas-data">(.*?)<\/script>/s);
// app.js is inlined as the LAST <script>...</script> block in the file.
const openTag = "<script>\n";
const lastOpen = html.lastIndexOf(openTag);
const closeIdx = lastOpen >= 0 ? html.lastIndexOf("</script>") : -1;
const appJs = lastOpen >= 0 && closeIdx > lastOpen
  ? html.slice(lastOpen + openTag.length, closeIdx)
  : null;
if (!dataMatch || !appJs) {
  console.error("FAIL: could not locate atlas-data or app.js block in", htmlPath);
  process.exit(1);
}

const stores = {};
global.document = {
  getElementById: id => stores[id] ||= {
    textContent: id === "atlas-data" ? dataMatch[1] : "",
    set innerHTML(v) { this._html = v; }, get innerHTML() { return this._html; },
  },
  querySelectorAll: () => [],
};
global.location = { hash: "#/" };
global.window = { addEventListener: () => {}, scrollTo: () => {} };
eval(appJs + "\n;global.renderModel = renderModel; global.renderSide = renderSide;");

let failures = 0;
const overview = stores["view"] && stores["view"]._html;
if (!overview || !overview.includes("censusbar")) {
  console.error("FAIL: overview did not render a census bar");
  failures++;
}
const svgCount = (overview && (overview.match(/<svg/g) || []).length) || 0;
if (svgCount === 0) {
  console.error("FAIL: overview rendered no DAG svgs");
  failures++;
}
console.log("overview length:", overview ? overview.length : 0, "svg count:", svgCount);

let ok = 0;
const DATA2 = JSON.parse(dataMatch[1]);
for (const proj of Object.keys(DATA2.projects)) {
  for (const [name, m] of Object.entries(DATA2.projects[proj].models)) {
    if (m.no_plan) continue;
    try {
      renderModel(proj, name);
      if (!stores["view"]._html.includes("technique tournament")) throw new Error("no tournament");
      ok++;
    } catch (e) {
      console.error("FAIL", proj, name, e.message);
      failures++;
    }
  }
}
console.log("model views ok:", ok, "failures:", failures);

if (ok === 0) {
  console.error("FAIL: no model views rendered at all");
  failures++;
}
if (failures > 0) {
  process.exit(1);
}
