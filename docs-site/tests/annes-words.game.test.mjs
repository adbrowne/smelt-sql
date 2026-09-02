import test from 'node:test';
import assert from 'node:assert/strict';
import { scoreGuess, mergeKeyStates, dailyIndex, puzzleNumber, msUntilNextPuzzle, shareText } from '../docs/annes-words/game.js';

const C = 'correct', P = 'present', A = 'absent';

test('scores an exact match', () => {
  assert.deepEqual(scoreGuess('crane', 'crane'), [C, C, C, C, C]);
});

test('scores a miss', () => {
  assert.deepEqual(scoreGuess('crane', 'moldy'), [A, A, A, A, A]);
});

test('scores simple presents', () => {
  // guess SPENT vs answer NOTES: p is the only letter not in the answer.
  assert.deepEqual(scoreGuess('spent', 'notes'), [P, A, P, P, P]);
});

test('duplicate letters in the guess do not over-claim', () => {
  // answer has one L; guess has three. Only the positional one scores.
  assert.deepEqual(scoreGuess('lolly', 'lymph'), [C, A, A, A, P]);
});

test('duplicate letters in the guess are marked left to right', () => {
  // answer ALLOY has two Ls. Guess LLAMA: the L at index 1 is positional, which
  // consumes one L from the pool; the L at index 0 takes the remaining one.
  // The trailing A is absent because the single A was already claimed at index 2.
  assert.deepEqual(scoreGuess('llama', 'alloy'), [P, C, P, A, A]);
});

test('exact matches consume the pool before presents', () => {
  // answer ABBEY: guess BABES -> B(present) A(present) B(correct) E(correct) S(absent)
  assert.deepEqual(scoreGuess('babes', 'abbey'), [P, P, C, C, A]);
});

test('a later exact match beats an earlier present for the same letter', () => {
  // answer PIZZA has one A at index 4. Guess AROMA: leading A must be absent.
  assert.deepEqual(scoreGuess('aroma', 'pizza'), [A, A, A, A, C]);
});

test('key states take the best result seen', () => {
  let keys = mergeKeyStates({}, 'crane', [A, P, A, A, C]);
  assert.deepEqual(keys, { c: A, r: P, a: A, n: A, e: C });
  keys = mergeKeyStates(keys, 'ratio', [C, A, A, A, A]);
  assert.equal(keys.r, C, 'present is upgraded to correct');
  assert.equal(keys.e, C, 'existing correct is retained');
  assert.equal(keys.a, A, 'absent stays absent');
});

test('key states never downgrade', () => {
  const keys = mergeKeyStates({ r: 'correct' }, 'rrrrr', [A, A, A, A, A]);
  assert.equal(keys.r, 'correct');
});

test('mergeKeyStates does not mutate its input', () => {
  const prev = { a: A };
  mergeKeyStates(prev, 'aaaaa', [C, C, C, C, C]);
  assert.deepEqual(prev, { a: A });
});

test('puzzle number counts whole local days from the epoch', () => {
  assert.equal(puzzleNumber(new Date(2026, 0, 1, 0, 0)), 0);
  assert.equal(puzzleNumber(new Date(2026, 0, 1, 23, 59)), 0);
  assert.equal(puzzleNumber(new Date(2026, 0, 2, 0, 1)), 1);
  assert.equal(puzzleNumber(new Date(2026, 1, 1, 12, 0)), 31);
});

test('daily index wraps around the list', () => {
  assert.equal(dailyIndex(new Date(2026, 0, 1), 10), 0);
  assert.equal(dailyIndex(new Date(2026, 0, 12), 10), 1);
});

test('daily index is stable across times of day', () => {
  const morning = dailyIndex(new Date(2026, 5, 9, 0, 0, 1), 2315);
  const night = dailyIndex(new Date(2026, 5, 9, 23, 59, 59), 2315);
  assert.equal(morning, night);
});

test('countdown targets the next local midnight', () => {
  const ms = msUntilNextPuzzle(new Date(2026, 5, 9, 23, 0, 0));
  assert.equal(ms, 60 * 60 * 1000);
});

test('share text renders an emoji grid', () => {
  const marks = [[A, P, A, A, C], [C, C, C, C, C]];
  const out = shareText(12, marks, true);
  assert.equal(out, "Anne's Words 12 2/6\n\n⬜🟨⬜⬜🟩\n🟩🟩🟩🟩🟩");
});

test('share text marks a loss with X', () => {
  const marks = Array.from({ length: 6 }, () => [A, A, A, A, A]);
  assert.ok(shareText(3, marks, false).startsWith("Anne's Words 3 X/6"));
});
