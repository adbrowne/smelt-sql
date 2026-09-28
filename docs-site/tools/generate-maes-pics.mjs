#!/usr/bin/env node
// Generator for Mae's Words' pictogram entries.
//
//   node docs-site/tools/generate-maes-pics.mjs          # fetch, optimise, write img/ + generated blocks
//   node docs-site/tools/generate-maes-pics.mjs --check  # exit 1 if the generated blocks would change
//
// Sources (fetched over HTTPS at generation time only; the game never fetches):
//   mulberry: https://raw.githubusercontent.com/mulberrysymbols/mulberry-symbols/master/EN/<id>.svg
//             Mulberry Symbols, Steve Lee, CC BY-SA 2.0 UK. Verbs are named "<verb>_,_to".
//   arasaac:  https://static.arasaac.org/pictograms/<id>/<id>_300.png
//             ARASAAC, Government of Aragon, CC BY-NC-SA 4.0. Gap-filler only.
//
// Hand-curation lives in PICS below. The ambiguity rule applies to every row: shown only the
// picture, a five-year-old says exactly this word. Reviewers remove rows that fail it.
import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync, unlinkSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const GAME = join(HERE, '..', 'docs', 'maes-words');
const IMG = join(GAME, 'img');
const CHECK = process.argv.includes('--check');

// { w, tier, src: 'mulberry' | 'arasaac', id, pos?: 'verb' | 'adj', noA?: true }
export const PICS = [
  // tier 1 nouns
  { w: 'mum', tier: 1, src: 'mulberry', id: 'mum_parent' },
  { w: 'dad', tier: 1, src: 'mulberry', id: 'dad_parent' },
  { w: 'man', tier: 1, src: 'arasaac', id: 4665 },
  { w: 'boy', tier: 1, src: 'arasaac', id: 7176 },
  { w: 'girl', tier: 1, src: 'arasaac', id: 27509 },
  { w: 'baby', tier: 3, src: 'mulberry', id: 'baby' },
  { w: 'cup', tier: 1, src: 'mulberry', id: 'cup_non_spill' },
  { w: 'mug', tier: 1, src: 'mulberry', id: 'mug_2' },
  { w: 'jug', tier: 1, src: 'arasaac', id: 2435 },
  { w: 'tap', tier: 1, src: 'mulberry', id: 'tap' },
  { w: 'cot', tier: 1, src: 'mulberry', id: 'cot' },
  { w: 'jam', tier: 1, src: 'mulberry', id: 'jam', noA: true },
  { w: 'bib', tier: 1, src: 'mulberry', id: 'bib' },
  { w: 'jar', tier: 1, src: 'mulberry', id: 'jar' },
  { w: 'rat', tier: 1, src: 'mulberry', id: 'rat' },
  { w: 'van', tier: 1, src: 'mulberry', id: 'van' },
  { w: 'hen', tier: 1, src: 'arasaac', id: 2403 },
  { w: 'pan', tier: 1, src: 'mulberry', id: 'frying_pan' },
  { w: 'peg', tier: 1, src: 'mulberry', id: 'clothes_peg' },
  { w: 'fan', tier: 1, src: 'arasaac', id: 2612 },
  { w: 'mop', tier: 1, src: 'mulberry', id: 'mop' },
  { w: 'zip', tier: 1, src: 'mulberry', id: 'zip' },
  // tier 2 nouns
  { w: 'nest', tier: 2, src: 'mulberry', id: 'nest' },
  { w: 'lamp', tier: 2, src: 'mulberry', id: 'lamp' },
  { w: 'sack', tier: 2, src: 'mulberry', id: 'sack' },
  { w: 'belt', tier: 2, src: 'mulberry', id: 'belt' },
  { w: 'vest', tier: 2, src: 'mulberry', id: 'vest' },
  { w: 'slug', tier: 2, src: 'mulberry', id: 'slug' },
  { w: 'swan', tier: 2, src: 'mulberry', id: 'swan' },
  { w: 'sink', tier: 2, src: 'mulberry', id: 'sink' },
  { w: 'lock', tier: 2, src: 'arasaac', id: 3261 },
  { w: 'bull', tier: 2, src: 'arasaac', id: 2595 },
  { w: 'paw',  tier: 2, src: 'mulberry', id: 'paw' },
  { w: 'salt', tier: 2, src: 'mulberry', id: 'salt' },
  { w: 'cart', tier: 2, src: 'mulberry', id: 'cart' },
  // tier 3 nouns
  { w: 'soup', tier: 3, src: 'mulberry', id: 'soup', noA: true },
  { w: 'comb', tier: 3, src: 'mulberry', id: 'comb' },
  { w: 'soap', tier: 3, src: 'mulberry', id: 'soap', noA: true },
  { w: 'rake', tier: 3, src: 'mulberry', id: 'rake' },
  { w: 'nail', tier: 3, src: 'mulberry', id: 'nail' },
  { w: 'lime', tier: 3, src: 'mulberry', id: 'lime' },
  { w: 'leek', tier: 3, src: 'mulberry', id: 'leek' },
  { w: 'coat', tier: 3, src: 'mulberry', id: 'coat' },
  { w: 'iron', tier: 3, src: 'mulberry', id: 'iron' },
  { w: 'pea',  tier: 3, src: 'mulberry', id: 'pea' },
  { w: 'vase', tier: 3, src: 'mulberry', id: 'vase' },
  { w: 'boot', tier: 3, src: 'arasaac', id: 8299 },
  { w: 'sofa', tier: 3, src: 'arasaac', id: 25479 },
  { w: 'rope', tier: 3, src: 'arasaac', id: 7006 },
  { w: 'tray', tier: 3, src: 'mulberry', id: 'tray' },
  { w: 'jeep', tier: 3, src: 'mulberry', id: 'jeep' },
  { w: 'cage', tier: 3, src: 'mulberry', id: 'cage' },
  { w: 'tie',  tier: 3, src: 'mulberry', id: 'tie' },
  { w: 'glue', tier: 3, src: 'mulberry', id: 'glue' },
  // verbs (Mulberry names verbs "<verb>_,_to")
  { w: 'run',  tier: 1, src: 'mulberry', id: 'run_,_to', pos: 'verb' },
  { w: 'sit',  tier: 1, src: 'mulberry', id: 'sit_,_to', pos: 'verb' },
  { w: 'hop',  tier: 1, src: 'mulberry', id: 'hop_,_to', pos: 'verb' },
  { w: 'dig',  tier: 1, src: 'mulberry', id: 'dig_,_to', pos: 'verb' },
  { w: 'eat',  tier: 2, src: 'mulberry', id: 'eat_,_to', pos: 'verb' },
  { w: 'hug',  tier: 1, src: 'mulberry', id: 'hug_,_to', pos: 'verb' },
  { w: 'stop', tier: 2, src: 'arasaac', id: 7196, pos: 'verb' },
  { w: 'jump', tier: 2, src: 'mulberry', id: 'jump_,_to', pos: 'verb' },
  { w: 'swim', tier: 2, src: 'mulberry', id: 'swim_,_to', pos: 'verb' },
  { w: 'read', tier: 3, src: 'mulberry', id: 'read_,_to', pos: 'verb' },
  { w: 'play', tier: 3, src: 'mulberry', id: 'play_,_to', pos: 'verb' },
  // adjectives
  { w: 'wet', tier: 1, src: 'mulberry', id: 'wet', pos: 'adj' },
  { w: 'sad', tier: 1, src: 'arasaac', id: 2606, pos: 'adj' },
  { w: 'bad', tier: 1, src: 'mulberry', id: 'bad', pos: 'adj' },
];

const urlFor = ({ src, id }) => src === 'mulberry'
  ? `https://raw.githubusercontent.com/mulberrysymbols/mulberry-symbols/master/EN/${encodeURIComponent(id)}.svg`
  : `https://static.arasaac.org/pictograms/${id}/${id}_300.png`;
const fileFor = ({ w, src }) => `${w}.${src === 'mulberry' ? 'svg' : 'png'}`;

/** Whitespace/comment/metadata stripping only — the symbol itself is unmodified (share-alike is not triggered). */
function optimiseSvg(text) {
  return text
    .replace(/<\?xml[^>]*\?>\s*/g, '')
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<metadata>[\s\S]*?<\/metadata>/g, '')
    .replace(/>\s+</g, '><')
    .trim();
}

async function fetchAsset(row) {
  const res = await fetch(urlFor(row));
  if (!res.ok) throw new Error(`${row.w}: ${res.status} for ${urlFor(row)}`);
  return row.src === 'mulberry' ? Buffer.from(optimiseSvg(await res.text())) : Buffer.from(await res.arrayBuffer());
}

const q = s => `'${s.replace(/'/g, "\\'")}'`;
const entryLine = r => {
  const extra = (r.pos ? `, pos: ${q(r.pos)}` : '') + (r.noA ? ', noA: true' : '');
  return `  { w: ${q(r.w)}, tier: ${r.tier}, pic: { kind: 'svg', src: ${q('img/' + fileFor(r))} }${extra} },`;
};

function replaceBlock(text, begin, end, body) {
  const a = text.indexOf(begin), b = text.indexOf(end);
  if (a < 0 || b < 0 || b < a) throw new Error(`markers ${begin} / ${end} not found`);
  return text.slice(0, a + begin.length) + '\n' + body + '\n' + text.slice(b);
}

const SOURCE_NAME = { mulberry: 'Mulberry Symbols (CC BY-SA 2.0 UK)', arasaac: 'ARASAAC (CC BY-NC-SA 4.0)' };
const creditsRows = () => PICS.map(r =>
  `<tr><td><img src="img/${fileFor(r)}" alt="" width="64" height="64"></td><td>${r.w}</td><td>${SOURCE_NAME[r.src]}</td><td><a href="${urlFor(r)}">${r.id}</a></td></tr>`).join('\n');

async function main() {
  const seen = new Set();
  for (const r of PICS) {
    if (!/^[a-z]{2,4}$/.test(r.w)) throw new Error(`bad word ${r.w}`);
    if (seen.has(r.w)) throw new Error(`duplicate ${r.w}`);
    seen.add(r.w);
  }
  const wordsPath = join(GAME, 'words.js');
  const creditsPath = join(GAME, 'credits.html');
  const words = readFileSync(wordsPath, 'utf8');
  const credits = readFileSync(creditsPath, 'utf8');
  const nextWords = replaceBlock(words, '// BEGIN GENERATED PICS', '// END GENERATED PICS', PICS.map(entryLine).join('\n'));
  const nextCredits = replaceBlock(credits, '<!-- BEGIN GENERATED CREDITS -->', '<!-- END GENERATED CREDITS -->', creditsRows());
  const wanted = new Set(PICS.map(fileFor));
  const missing = PICS.filter(r => !existsSync(join(IMG, fileFor(r))));
  if (CHECK) {
    const changed = nextWords !== words || nextCredits !== credits || missing.length > 0;
    if (changed) { console.error('generated output is stale; run without --check'); process.exit(1); }
    return;
  }
  mkdirSync(IMG, { recursive: true });
  for (const r of missing) {
    writeFileSync(join(IMG, fileFor(r)), await fetchAsset(r));
    console.log('fetched', fileFor(r));
  }
  for (const f of readdirSync(IMG)) if (/\.(svg|png)$/.test(f) && !wanted.has(f)) { unlinkSync(join(IMG, f)); console.log('removed', f); }
  writeFileSync(wordsPath, nextWords);
  writeFileSync(creditsPath, nextCredits);
  console.log(`${PICS.length} pictogram words`);
}

main().catch(e => { console.error(e.message); process.exit(1); });
