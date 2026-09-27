// Pure logic only. No DOM, no clock, no Math.random: randomness is passed in as
// `rng: () => number in [0,1)` so tests can pin it.

export const MAX_LEVEL = 3;
export const ROUND_LENGTH = 10;
export const INITIAL_INTRO = 6;
export const INTRO_BATCH = 2;
export const NEW_WORD_FLOOR = 6;
export const UNLOCK_FRACTION = 0.75;
export const ACTIVITY_FOR_LEVEL = Object.freeze(['read', 'pick', 'build', 'sentence']);

export const emptyState = () => ({ levels: {}, stars: 0, learnedOrder: [] });

const cloneState = s => ({ levels: { ...s.levels }, stars: s.stars, learnedOrder: [...s.learnedOrder] });
const isIntroduced = (state, w) => Object.prototype.hasOwnProperty.call(state.levels, w);

export function initialState(words) {
  const s = emptyState();
  for (const word of words.filter(x => x.tier === 1).slice(0, INITIAL_INTRO)) s.levels[word.w] = 0;
  return s;
}

/** Highest tier new words may be introduced from. Tier 1 is always open. */
export function unlockedTier(state, words) {
  const maxTier = Math.max(1, ...words.map(x => x.tier));
  let tier = 1;
  while (tier < maxTier) {
    const inTier = words.filter(x => x.tier === tier);
    const mastered = inTier.filter(x => (state.levels[x.w] ?? 0) >= 2 && isIntroduced(state, x.w)).length;
    if (inTier.length === 0 || mastered / inTier.length < UNLOCK_FRACTION) break;
    tier += 1;
  }
  return tier;
}

/** One trickle step: if fewer than NEW_WORD_FLOOR introduced words sit at level 0, bring in up to INTRO_BATCH more. */
export function introduceWords(state, words) {
  const atZero = Object.values(state.levels).filter(l => l === 0).length;
  if (atZero >= NEW_WORD_FLOOR) return state;
  const tier = unlockedTier(state, words);
  const fresh = words.filter(x => x.tier <= tier && !isIntroduced(state, x.w)).slice(0, INTRO_BATCH);
  if (fresh.length === 0) return state;
  const next = cloneState(state);
  for (const word of fresh) next.levels[word.w] = 0;
  return next;
}

export function applyAnswer(state, word, correct) {
  const next = cloneState(state);
  const cur = next.levels[word.w] ?? 0;
  const level = correct ? Math.min(MAX_LEVEL, cur + 1) : Math.max(0, cur - 1);
  next.levels[word.w] = level;
  if (correct) next.stars += 1;
  if (level === MAX_LEVEL && !next.learnedOrder.includes(word.w)) next.learnedOrder.push(word.w);
  return next;
}

export const activeWords = (state, words) =>
  words.filter(x => isIntroduced(state, x.w) && state.levels[x.w] < MAX_LEVEL);

export function learnedWords(state, words) {
  const byName = new Map(words.map(x => [x.w, x]));
  const ordered = state.learnedOrder.map(w => byName.get(w)).filter(x => x && state.levels[x.w] === MAX_LEVEL);
  const seen = new Set(ordered.map(x => x.w));
  const rest = words.filter(x => state.levels[x.w] === MAX_LEVEL && !seen.has(x.w));
  return [...ordered, ...rest];
}
