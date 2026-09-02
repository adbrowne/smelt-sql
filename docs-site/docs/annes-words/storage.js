// docs-site/docs/annes-words/storage.js
// Pure serialization + stats arithmetic. The caller owns localStorage so this
// module stays testable under node.

export const KEY = 'annes-words:v1';
export const VERSION = 1;

export const DEFAULTS = Object.freeze({
  version: VERSION,
  daily: null,
  stats: Object.freeze({ played: 0, wins: 0, streak: 0, maxStreak: 0, lastPuzzle: null, dist: [0, 0, 0, 0, 0, 0] }),
});

const clone = v => JSON.parse(JSON.stringify(v));

/** Coerce to a finite number, falling back when it is not one. */
const toFiniteNumber = (v, fallback) => {
  const n = Number(v);
  return Number.isFinite(n) ? n : fallback;
};

/** Same, but a `null` input stays `null` rather than becoming a number. */
const toFiniteOrNull = v => (v === null ? null : toFiniteNumber(v, null));

const isValidGuessEntry = g => typeof g === 'string' && g.length === 5;

/** Parse stored JSON, repairing anything missing. Never throws. */
export function load(raw) {
  if (!raw) return clone(DEFAULTS);
  let parsed;
  try { parsed = JSON.parse(raw); } catch { return clone(DEFAULTS); }
  if (!parsed || parsed.version !== VERSION) return clone(DEFAULTS);
  const base = clone(DEFAULTS);
  const stats = { ...base.stats, ...(parsed.stats ?? {}) };
  stats.played = toFiniteNumber(stats.played, 0);
  stats.wins = toFiniteNumber(stats.wins, 0);
  stats.streak = toFiniteNumber(stats.streak, 0);
  stats.maxStreak = toFiniteNumber(stats.maxStreak, 0);
  stats.lastPuzzle = toFiniteOrNull(stats.lastPuzzle);
  stats.dist = Array.isArray(stats.dist) && stats.dist.length === 6
    ? stats.dist.map(n => toFiniteNumber(n, 0))
    : [0, 0, 0, 0, 0, 0];
  const rawGuesses = parsed.daily && Array.isArray(parsed.daily.guesses) ? parsed.daily.guesses : null;
  const guessesOk = rawGuesses !== null && rawGuesses.length <= 6 && rawGuesses.every(isValidGuessEntry);
  const daily = parsed.daily && typeof parsed.daily.puzzle === 'number' && guessesOk
    ? { puzzle: parsed.daily.puzzle, guesses: rawGuesses, status: parsed.daily.status ?? 'playing' }
    : null;
  return { version: VERSION, daily, stats };
}

/**
 * A skipped day (a gap of more than one puzzle since the stored streak was
 * last extended) resets the streak. Pure: takes today's puzzle number as an
 * argument rather than reading the clock itself.
 */
export function expireStreak(stats, todayPuzzle) {
  if (stats.lastPuzzle !== null && stats.lastPuzzle < todayPuzzle - 1) {
    return { ...stats, streak: 0 };
  }
  return stats;
}

export const serialize = data => JSON.stringify(data);

/** Fold one finished daily game into the stats. Returns a new object. */
export function recordResult(stats, { won, guesses, puzzle, lastPuzzle }) {
  const next = { ...stats, dist: [...stats.dist] };
  next.played += 1;
  const consecutive = lastPuzzle !== null && lastPuzzle !== undefined && puzzle === lastPuzzle + 1;
  if (won) {
    next.wins += 1;
    next.dist[guesses - 1] += 1;
    next.streak = consecutive ? stats.streak + 1 : 1;
    next.maxStreak = Math.max(stats.maxStreak, next.streak);
  } else {
    next.streak = 0;
  }
  next.lastPuzzle = puzzle;
  return next;
}
