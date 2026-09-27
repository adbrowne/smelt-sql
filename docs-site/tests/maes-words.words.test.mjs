import test from 'node:test';
import assert from 'node:assert/strict';
import { WORDS, FRAMES } from '../docs/maes-words/words.js';

test('every word is 2-4 lowercase letters with an emoji and a tier', () => {
  for (const { w, e, tier } of WORDS) {
    assert.match(w, /^[a-z]{2,4}$/, `bad word ${w}`);
    assert.ok(typeof e === 'string' && e.length > 0, `no emoji for ${w}`);
    assert.ok([1, 2, 3].includes(tier), `bad tier for ${w}`);
  }
});

test('words and emoji are unique', () => {
  const ws = WORDS.map(x => x.w), es = WORDS.map(x => x.e);
  assert.equal(new Set(ws).size, ws.length, 'duplicate word');
  assert.equal(new Set(es).size, es.length, 'duplicate emoji');
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

test('every frame has exactly one blank', () => {
  assert.ok(FRAMES.length >= 4);
  for (const f of FRAMES) assert.equal((f.match(/\{\}/g) || []).length, 1, `bad frame ${f}`);
});
