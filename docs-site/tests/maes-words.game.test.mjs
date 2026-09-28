import test from 'node:test';
import assert from 'node:assert/strict';
import { emptyState, initialState, unlockedTier, introduceWords, applyAnswer,
         activeWords, learnedWords, INITIAL_INTRO, INTRO_BATCH, NEW_WORD_FLOOR,
         MAX_LEVEL, applyAttempt, retryAllowed,
         SIGHT_INITIAL_INTRO, SIGHT_INTRO_BATCH, SIGHT_NEW_WORD_FLOOR, isSight } from '../docs/maes-words/game.js';

// A small bank: 8 tier-1, 4 tier-2, 4 tier-3.
const mk = (w, tier) => ({ w, e: w.toUpperCase(), tier });
const BANK = [
  ...['cat', 'dog', 'sun', 'bed', 'pig', 'bus', 'egg', 'hat'].map(w => mk(w, 1)),
  ...['frog', 'milk', 'duck', 'bell'].map(w => mk(w, 2)),
  ...['cake', 'tree', 'boat', 'bear'].map(w => mk(w, 3)),
];
const byName = w => BANK.find(x => x.w === w);
const withLevels = levels => ({ ...emptyState(), levels: { ...levels } });
const SIGHTS = [...['the', 'is', 'my', 'in', 'on', 'it'].map(w => ({ w, tier: 1, sight: true })),
                ...['here', 'like', 'see', 'can'].map(w => ({ w, tier: 2, sight: true }))];
const ALL = [...BANK, ...SIGHTS];

test('initialState introduces the first INITIAL_INTRO tier-1 words at level 0', () => {
  const s = initialState(BANK);
  assert.deepEqual(Object.keys(s.levels), ['cat', 'dog', 'sun', 'bed', 'pig', 'bus']);
  assert.ok(Object.values(s.levels).every(l => l === 0));
  assert.equal(s.stars, 0);
  assert.equal(INITIAL_INTRO, 6);
});

test('applyAnswer moves a word up on correct and down on wrong, clamped', () => {
  let s = withLevels({ cat: 0 });
  s = applyAnswer(s, byName('cat'), false);
  assert.equal(s.levels.cat, 0, 'floor at 0');
  s = applyAnswer(s, byName('cat'), true);
  assert.equal(s.levels.cat, 1);
  assert.equal(s.stars, 1);
  s = applyAnswer(s, byName('cat'), true);
  s = applyAnswer(s, byName('cat'), true);
  s = applyAnswer(s, byName('cat'), true);
  assert.equal(s.levels.cat, MAX_LEVEL, 'cap at MAX_LEVEL');
  assert.equal(s.stars, 4);
  s = applyAnswer(s, byName('cat'), false);
  assert.equal(s.levels.cat, 2);
});

test('applyAnswer does not mutate its input', () => {
  const s = withLevels({ cat: 1 });
  applyAnswer(s, byName('cat'), true);
  assert.equal(s.levels.cat, 1);
  assert.equal(s.stars, 0);
});

test('reaching level 3 appends to learnedOrder once', () => {
  let s = withLevels({ cat: 2, dog: 2 });
  s = applyAnswer(s, byName('dog'), true);
  s = applyAnswer(s, byName('cat'), true);
  assert.deepEqual(s.learnedOrder, ['dog', 'cat']);
  s = applyAnswer(s, byName('dog'), false);
  s = applyAnswer(s, byName('dog'), true);
  assert.deepEqual(s.learnedOrder, ['dog', 'cat'], 'no duplicate on re-learn');
});

test('unlockedTier: tier 1 always; tier 2 once 75% of tier 1 is at level >= 2', () => {
  assert.equal(unlockedTier(emptyState(), BANK), 1);
  // 5 of 8 tier-1 at >=2 is 62.5%: not enough
  let s = withLevels({ cat: 2, dog: 3, sun: 2, bed: 2, pig: 2, bus: 1, egg: 0 });
  assert.equal(unlockedTier(s, BANK), 1);
  // 6 of 8 is 75%: unlocked
  s = withLevels({ cat: 2, dog: 3, sun: 2, bed: 2, pig: 2, bus: 2, egg: 0 });
  assert.equal(unlockedTier(s, BANK), 2);
  // tier 3 needs tier 2 as well (3 of 4 = 75%)
  s = withLevels({ ...s.levels, frog: 2, milk: 3, duck: 2, bell: 0 });
  assert.equal(unlockedTier(s, BANK), 3);
});

test('unlockedTier never exceeds the highest tier in the bank', () => {
  const s = withLevels(Object.fromEntries(BANK.map(x => [x.w, 3])));
  assert.equal(unlockedTier(s, BANK), 3);
});

test('introduceWords adds INTRO_BATCH words only when fewer than NEW_WORD_FLOOR are at level 0', () => {
  assert.equal(INTRO_BATCH, 2);
  assert.equal(NEW_WORD_FLOOR, 6);
  // six at level 0: nothing happens
  let s = withLevels({ cat: 0, dog: 0, sun: 0, bed: 0, pig: 0, bus: 0 });
  assert.deepEqual(introduceWords(s, BANK), s);
  // five at level 0: two more come in, in bank order
  s = withLevels({ cat: 1, dog: 0, sun: 0, bed: 0, pig: 0, bus: 0 });
  const next = introduceWords(s, BANK);
  assert.deepEqual(Object.keys(next.levels), ['cat', 'dog', 'sun', 'bed', 'pig', 'bus', 'egg', 'hat']);
  assert.equal(next.levels.egg, 0);
  assert.equal(next.levels.hat, 0);
});

test('introduceWords respects the unlocked tier', () => {
  // all tier 1 introduced and mastered except none at level 0; tier 2 locked (0% of tier 1 at >=2)
  const s = withLevels({ cat: 1, dog: 1, sun: 1, bed: 1, pig: 1, bus: 1, egg: 1, hat: 1 });
  assert.deepEqual(introduceWords(s, BANK), s, 'nothing to introduce while tier 2 is locked');
  const t = withLevels({ cat: 2, dog: 2, sun: 2, bed: 2, pig: 2, bus: 2, egg: 1, hat: 1 });
  const next = introduceWords(t, BANK);
  assert.equal(next.levels.frog, 0);
  assert.equal(next.levels.milk, 0);
  assert.equal(next.levels.duck, undefined);
});

test('introduceWords introduces a single word when only one is left', () => {
  const all = Object.fromEntries(BANK.map(x => [x.w, 3]));
  delete all.bear;
  const next = introduceWords(withLevels(all), BANK);
  assert.equal(next.levels.bear, 0);
});

test('activeWords and learnedWords split introduced words by level, learned in learnedOrder', () => {
  const s = { ...withLevels({ cat: 3, dog: 1, sun: 3, bed: 0 }), learnedOrder: ['sun', 'cat'] };
  assert.deepEqual(activeWords(s, BANK).map(x => x.w), ['dog', 'bed']);
  assert.deepEqual(learnedWords(s, BANK).map(x => x.w), ['sun', 'cat']);
});

test('applyAttempt: a wrong-then-right turn changes the level by exactly -1 and adds no star', () => {
  let s = withLevels({ cat: 2 });
  s = applyAttempt(s, byName('cat'), false, 0);
  assert.equal(s.levels.cat, 1);
  assert.equal(s.stars, 0);
  s = applyAttempt(s, byName('cat'), false, 1);
  assert.equal(s.levels.cat, 1, 'second wrong on the same turn does not count again');
  s = applyAttempt(s, byName('cat'), true, 2);
  assert.equal(s.levels.cat, 1, 'fixing it earns no level');
  assert.equal(s.stars, 0, 'fixing it earns no star');
});

test('applyAttempt: a clean first try behaves like applyAnswer', () => {
  const s = applyAttempt(withLevels({ cat: 0 }), byName('cat'), true, 0);
  assert.equal(s.levels.cat, 1);
  assert.equal(s.stars, 1);
});

test('retryAllowed allows exactly one Build retry and unlimited tile retries', () => {
  assert.equal(retryAllowed({ activity: 'build' }, 1), true);
  assert.equal(retryAllowed({ activity: 'build' }, 2), false);
  for (const activity of ['read', 'pick', 'sentence', 'fill']) {
    assert.equal(retryAllowed({ activity }, 1), true);
    assert.equal(retryAllowed({ activity }, 3), true);
  }
});

test('initialState introduces 6 tier-1 nouns and 2 tier-1 sight words', () => {
  const s = initialState(ALL);
  assert.deepEqual(Object.keys(s.levels), ['cat', 'dog', 'sun', 'bed', 'pig', 'bus', 'the', 'is']);
  assert.equal(SIGHT_INITIAL_INTRO, 2);
});

test('sight words trickle independently: one at a time when fewer than 2 are at level 0', () => {
  const s0 = withLevels({ cat: 0, dog: 0, sun: 0, bed: 0, pig: 0, bus: 0, the: 1, is: 0 });
  const s1 = introduceWords(s0, ALL);
  assert.deepEqual(Object.keys(s1.levels).filter(w => isSight(ALL.find(x => x.w === w))), ['the', 'is', 'my']);
  assert.equal(Object.keys(s1.levels).length, 9, 'nouns untouched because 6 sit at level 0');
  assert.equal(SIGHT_INTRO_BATCH, 1); assert.equal(SIGHT_NEW_WORD_FLOOR, 2);
});

test('sight tier 2 unlocks on the 75% rule over SIGHT only', () => {
  const s = withLevels({ the: 2, is: 2, my: 2, in: 2, on: 2, it: 0, cat: 0 });  // 5/6 tier-1 sight mastered
  const next = introduceWords(s, ALL);
  assert.ok('here' in next.levels, 'tier-2 sight word introduced');
});
