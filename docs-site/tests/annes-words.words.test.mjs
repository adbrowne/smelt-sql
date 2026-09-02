import test from 'node:test';
import assert from 'node:assert/strict';
import { ANSWERS, ALLOWED } from '../docs/annes-words/words.js';

test('answer list has the expected size and shape', () => {
  assert.equal(ANSWERS.length, 2315);
  assert.ok(ANSWERS.every(w => /^[a-z]{5}$/.test(w)), 'all answers are 5 lowercase letters');
  assert.equal(new Set(ANSWERS).size, ANSWERS.length, 'no duplicate answers');
});

test('allowed list has the expected size and does not overlap answers', () => {
  assert.equal(ALLOWED.length, 10657);
  assert.ok(ALLOWED.every(w => /^[a-z]{5}$/.test(w)));
  const answers = new Set(ANSWERS);
  assert.equal(ALLOWED.filter(w => answers.has(w)).length, 0);
});

test('answers are shuffled, not alphabetical', () => {
  const sorted = [...ANSWERS].sort();
  assert.notDeepEqual(ANSWERS.slice(0, 50), sorted.slice(0, 50));
});
