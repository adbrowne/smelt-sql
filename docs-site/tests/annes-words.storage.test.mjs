import test from 'node:test';
import assert from 'node:assert/strict';
import { load, serialize, DEFAULTS, recordResult } from '../docs/annes-words/storage.js';

test('load returns defaults for missing or corrupt data', () => {
  assert.deepEqual(load(null), DEFAULTS);
  assert.deepEqual(load('not json'), DEFAULTS);
  assert.deepEqual(load('{"version":99}'), DEFAULTS);
});

test('load round-trips a saved game', () => {
  const data = { ...DEFAULTS, daily: { puzzle: 7, guesses: ['crane'], status: 'playing' } };
  assert.deepEqual(load(serialize(data)), data);
});

test('load repairs a partial stats object', () => {
  const out = load('{"version":1,"stats":{"played":3}}');
  assert.equal(out.stats.played, 3);
  assert.equal(out.stats.wins, 0);
  assert.deepEqual(out.stats.dist, [0, 0, 0, 0, 0, 0]);
});

test('a win increments the streak and the distribution', () => {
  const s = recordResult(DEFAULTS.stats, { won: true, guesses: 3, puzzle: 10, lastPuzzle: 9 });
  assert.equal(s.played, 1);
  assert.equal(s.wins, 1);
  assert.equal(s.streak, 1);
  assert.equal(s.maxStreak, 1);
  assert.deepEqual(s.dist, [0, 0, 1, 0, 0, 0]);
});

test('a loss resets the streak but keeps the max', () => {
  let s = recordResult(DEFAULTS.stats, { won: true, guesses: 2, puzzle: 10, lastPuzzle: 9 });
  s = recordResult(s, { won: true, guesses: 4, puzzle: 11, lastPuzzle: 10 });
  s = recordResult(s, { won: false, guesses: 6, puzzle: 12, lastPuzzle: 11 });
  assert.equal(s.streak, 0);
  assert.equal(s.maxStreak, 2);
  assert.equal(s.played, 3);
});

test('a skipped day breaks the streak', () => {
  let s = recordResult(DEFAULTS.stats, { won: true, guesses: 2, puzzle: 10, lastPuzzle: 9 });
  s = recordResult(s, { won: true, guesses: 2, puzzle: 15, lastPuzzle: 10 });
  assert.equal(s.streak, 1, 'streak restarts at 1 rather than continuing');
  assert.equal(s.maxStreak, 1);
});

test('recordResult does not mutate its input', () => {
  const before = { ...DEFAULTS.stats };
  recordResult(DEFAULTS.stats, { won: true, guesses: 1, puzzle: 1, lastPuzzle: 0 });
  assert.deepEqual(DEFAULTS.stats, before);
});
