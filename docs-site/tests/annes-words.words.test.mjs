import test from 'node:test';
import assert from 'node:assert/strict';
import { ANSWERS, ALLOWED } from '../docs/annes-words/words.js';

test('answer list has the expected size and shape', () => {
  assert.equal(ANSWERS.length, 2000);
  assert.ok(ANSWERS.every(w => /^[a-z]{5}$/.test(w)), 'all answers are 5 lowercase letters');
  assert.equal(new Set(ANSWERS).size, ANSWERS.length, 'no duplicate answers');
});

test('allowed list has the expected shape and does not overlap answers', () => {
  assert.ok(ALLOWED.length > 0);
  assert.ok(ALLOWED.every(w => /^[a-z]{5}$/.test(w)), 'all allowed words are 5 lowercase letters');
  assert.equal(new Set(ALLOWED).size, ALLOWED.length, 'no duplicate allowed words');
  const answers = new Set(ANSWERS);
  assert.equal(ALLOWED.filter(w => answers.has(w)).length, 0, 'no overlap between ALLOWED and ANSWERS');
});

test('no answer ends in s (plurals excluded)', () => {
  assert.ok(ANSWERS.every(w => !w.endsWith('s')));
});

test('answers are shuffled, not alphabetical', () => {
  const sorted = [...ANSWERS].sort();
  assert.notDeepEqual(ANSWERS.slice(0, 50), sorted.slice(0, 50));
});

test('allowed list is not shuffled (alphabetical by construction)', () => {
  // ALLOWED order is not gameplay-relevant, but this pins the generator's
  // deterministic output shape.
  const sorted = [...ALLOWED].sort();
  assert.deepEqual(ALLOWED, sorted);
});

test('common everyday words are present somewhere in the combined word set', () => {
  const combined = new Set([...ANSWERS, ...ALLOWED]);
  for (const word of ['crane', 'slate', 'about']) {
    assert.ok(combined.has(word), `expected "${word}" to be a valid word`);
  }
});

test('every answer contains a vowel (excludes non-words like Roman numerals)', () => {
  assert.ok(ANSWERS.every(w => /[aeiou]/.test(w)));
});

test('manually excluded words never appear as answers', () => {
  // Words that slip past the LDNOOBW profanity list; see MANUAL_EXCLUSIONS
  // in docs-site/tools/generate-words.mjs.
  assert.ok(!ANSWERS.includes('cunny'));
  assert.ok(!ALLOWED.includes('cunny'));
});
