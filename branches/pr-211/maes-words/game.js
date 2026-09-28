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

export const BUILD_RETRIES = 1;

/** Only the first attempt of a turn moves the ladder or earns a star. */
export function applyAttempt(state, word, correct, attempts) {
  return attempts === 0 ? applyAnswer(state, word, correct) : state;
}

/** May Mae have another go after `attempts` wrong answers on this turn? */
export function retryAllowed(turn, attempts) {
  return turn.activity === 'build' ? attempts <= BUILD_RETRIES : true;
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

// --- turn construction -------------------------------------------------------

export function shuffle(arr, rng) {
  const out = [...arr];
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(rng() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

export function editDistance(a, b) {
  const dp = Array.from({ length: a.length + 1 }, (_, i) => [i, ...Array(b.length).fill(0)]);
  for (let j = 1; j <= b.length; j++) dp[0][j] = j;
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      dp[i][j] = Math.min(dp[i - 1][j] + 1, dp[i][j - 1] + 1, dp[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
    }
  }
  return dp[a.length][b.length];
}

/** Look-alike words from the bank, closest first, ties by length difference then bank order. */
export function distractors(word, words, n) {
  return words
    .map((x, i) => ({ x, i }))
    .filter(({ x }) => x.w !== word.w)
    .sort((p, q) =>
      editDistance(p.x.w, word.w) - editDistance(q.x.w, word.w)
      || Math.abs(p.x.w.length - word.w.length) - Math.abs(q.x.w.length - word.w.length)
      || p.i - q.i)
    .slice(0, n)
    .map(({ x }) => x);
}

const ALPHABET = 'abcdefghijklmnopqrstuvwxyz'.split('');

const canSpell = (tray, w) => {
  const pool = [...tray];
  for (const ch of w) {
    const k = pool.indexOf(ch);
    if (k < 0) return false;
    pool.splice(k, 1);
  }
  return true;
};

/** Word's letters plus 1 (<=3 letters) or 2 (4 letters) distractor letters, shuffled. A distractor is
 *  rejected if the resulting tray could spell any other bank word of the same length. */
export function letterTray(word, words, rng) {
  const letters = word.w.split('');
  const wanted = word.w.length <= 3 ? 1 : 2;
  const others = words.filter(x => x.w !== word.w && x.w.length === word.w.length).map(x => x.w);
  const tray = [...letters];
  const candidates = shuffle(ALPHABET.filter(ch => !letters.includes(ch)), rng);
  for (const ch of candidates) {
    if (tray.length - letters.length >= wanted) break;
    const trial = [...tray, ch];
    if (others.some(o => canSpell(trial, o))) continue;
    tray.push(ch);
  }
  return shuffle(tray, rng);
}

export const posOf = word => word.pos ?? 'noun';

/** Frames usable for a word: same part of speech; and no "a {noun}" for vowel-initial, mass-noun or number words. */
export function framesFor(word, frames) {
  const samePos = frames.filter(f => (f.pos ?? 'noun') === posOf(word));
  const needsThe = word.noA || /^[aeiou]/.test(word.w);
  const ok = needsThe ? samePos.filter(f => !/\ba \{noun\}/.test(f.text)) : samePos;
  return ok.length ? ok : samePos;
}

const capitalise = s => s.charAt(0).toUpperCase() + s.slice(1);

/** Fill in the sight slot (capitalised if it opens the sentence) and leave `{}` where the pictured word goes. */
export function resolveFrame(frame, sight) {
  let text = frame.text;
  if (sight !== null && sight !== undefined) {
    text = text.startsWith('{sight}') ? text.replace('{sight}', capitalise(sight)) : text.replace('{sight}', sight);
  }
  return text.replace('{noun}', '{}');
}

export function makeTurn(word, level, words, frames, rng) {
  const activity = ACTIVITY_FOR_LEVEL[Math.min(level, ACTIVITY_FOR_LEVEL.length - 1)];
  switch (activity) {
    case 'read':
      return { activity, word, options: shuffle([word, ...distractors(word, words, 3)], rng) };
    case 'pick':
      return { activity, word, options: shuffle([word, ...distractors(word, words, 2)], rng) };
    case 'build':
      return { activity, word, tray: letterTray(word, words, rng) };
    case 'sentence':
    default: {
      const usable = framesFor(word, frames);
      const frame = usable[Math.floor(rng() * usable.length)];
      const sight = frame.fits.length ? frame.fits[Math.floor(rng() * frame.fits.length)] : null;
      return {
        activity: 'sentence', word,
        frame: resolveFrame(frame, sight),
        options: shuffle([word, ...distractors(word, words, 2)], rng),
      };
    }
  }
}

/** Re-derive a planned turn if the word's level has moved since planning, so each level is earned
 *  through its own activity rather than being skipped when a word repeats within one round. */
export function refreshTurn(turn, state, words, frames, rng) {
  const level = Math.min(state.levels[turn.word.w] ?? 0, MAX_LEVEL);
  return ACTIVITY_FOR_LEVEL[level] === turn.activity ? turn : makeTurn(turn.word, level, words, frames, rng);
}

export function isCorrect(turn, answer) {
  if (turn.activity === 'build') {
    return Array.isArray(answer) && answer.join('') === turn.word.w;
  }
  return answer === turn.word.w;
}

/** Weighted sample. Without replacement when the pool is at least `count` long; otherwise every pick is
 *  independent (with replacement), so a small pool still fills a round and keeps its weighting. */
function weightedPicks(pool, weightOf, count, rng) {
  if (pool.length === 0) return [];
  const withReplacement = pool.length < count;
  const remaining = [...pool];
  const out = [];
  while (out.length < count) {
    const src = withReplacement ? pool : remaining;
    const total = src.reduce((s, x) => s + weightOf(x), 0);
    let r = rng() * total;
    let idx = src.length - 1;
    for (let i = 0; i < src.length; i++) {
      r -= weightOf(src[i]);
      if (r < 0) { idx = i; break; }
    }
    out.push(src[idx]);
    if (!withReplacement) remaining.splice(idx, 1);
  }
  return out;
}

export function planRound(state, words, frames, rng) {
  const active = activeWords(state, words);
  const learned = learnedWords(state, words);
  if (active.length === 0 && learned.length === 0) return [];
  if (active.length === 0) {
    return weightedPicks(learned, () => 1, ROUND_LENGTH, rng).map(w => makeTurn(w, MAX_LEVEL, words, frames, rng));
  }
  const reviewSlots = learned.length > 0 ? 1 : 0;
  const picks = weightedPicks(active, w => 4 - state.levels[w.w], ROUND_LENGTH - reviewSlots, rng)
    .map(w => makeTurn(w, state.levels[w.w], words, frames, rng));
  if (reviewSlots) {
    const review = learned[Math.floor(rng() * learned.length)];
    const at = Math.floor(rng() * (picks.length + 1));
    picks.splice(at, 0, makeTurn(review, MAX_LEVEL, words, frames, rng));
  }
  return picks;
}
