import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { WORDS, FRAMES } from '../docs/maes-words/words.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const GAME_DIR = join(HERE, '..', 'docs', 'maes-words');

test('every word is 2-4 lowercase letters with a tier and a well-formed pic', () => {
  for (const { w, tier, pic } of WORDS) {
    assert.match(w, /^[a-z]{2,4}$/, `bad word ${w}`);
    assert.ok([1, 2, 3].includes(tier), `bad tier for ${w}`);
    assert.ok(pic && typeof pic === 'object', `no pic for ${w}`);
    assert.ok(['emoji', 'svg', 'colour'].includes(pic.kind), `bad pic.kind for ${w}`);
    if (pic.kind === 'emoji') assert.ok(typeof pic.text === 'string' && pic.text.length > 0, `no emoji text for ${w}`);
    if (pic.kind === 'svg') assert.match(pic.src, /^img\/[a-z0-9_-]+\.(svg|png)$/, `bad src for ${w}`);
    if (pic.kind === 'colour') assert.match(pic.css, /^#[0-9a-f]{6}$/, `bad css for ${w}`);
  }
});

test('every svg/png src exists on disk', () => {
  for (const { w, pic } of WORDS) {
    if (pic.kind !== 'svg') continue;
    assert.ok(existsSync(join(GAME_DIR, pic.src)), `${w}: ${pic.src} missing`);
  }
});

test('words, emoji and image sources are unique', () => {
  const ws = WORDS.map(x => x.w);
  const es = WORDS.filter(x => x.pic.kind === 'emoji').map(x => x.pic.text);
  const srcs = WORDS.filter(x => x.pic.kind === 'svg').map(x => x.pic.src);
  assert.equal(new Set(ws).size, ws.length, 'duplicate word');
  assert.equal(new Set(es).size, es.length, 'duplicate emoji');
  assert.equal(new Set(srcs).size, srcs.length, 'duplicate src');
});

test('bank is big enough for the progression rules', () => {
  assert.ok(WORDS.length >= 80, `only ${WORDS.length} words`);
  assert.ok(WORDS.filter(x => x.tier === 1).length >= 20, 'tier 1 too small');
  assert.ok(WORDS.filter(x => x.tier === 2).length >= 20, 'tier 2 too small');
  assert.ok(WORDS.filter(x => x.tier === 3).length >= 20, 'tier 3 too small');
});

test('no plurals', () => {
  for (const { w } of WORDS) assert.ok(!/[^s]s$/.test(w) || ['bus', 'gas'].includes(w), `plural? ${w}`);
});

test('every frame has one {noun}, at most one {sight}, a fits list of SIGHT words, and a known pos', () => {
  assert.ok(FRAMES.length >= 6);
  for (const f of FRAMES) {
    assert.equal((f.text.match(/\{noun\}/g) || []).length, 1, `bad frame ${f.text}`);
    assert.ok((f.text.match(/\{sight\}/g) || []).length <= 1, `two sights in ${f.text}`);
    assert.ok(Array.isArray(f.fits), `no fits on ${f.text}`);
    if (f.text.includes('{sight}')) assert.ok(f.fits.length >= 1, `templated frame with empty fits: ${f.text}`);
    else assert.equal(f.fits.length, 0, `fits on an untemplated frame: ${f.text}`);
    assert.ok(['noun', 'verb', 'adj'].includes(f.pos ?? 'noun'), `bad pos on ${f.text}`);
  }
});

test('noA is only ever true where present', () => {
  for (const w of WORDS) if ('noA' in w) assert.equal(w.noA, true, `${w.w} has noA !== true`);
});

test('at least three noun frames use "the" so vowel/noA words still get variety', () => {
  const theFrames = FRAMES.filter(f => (f.pos ?? 'noun') === 'noun' && /\bthe\b/i.test(f.text));
  assert.ok(theFrames.length >= 3, `only ${theFrames.length} "the" frames`);
});

test('every pos used by a word has at least one frame, so framesFor never returns []', () => {
  const wordPoses = new Set(WORDS.map(w => w.pos ?? 'noun'));
  const framePoses = new Set(FRAMES.map(f => f.pos ?? 'noun'));
  for (const pos of wordPoses) assert.ok(framePoses.has(pos), `no frame has pos ${pos}`);
});
