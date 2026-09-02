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

/** Parse stored JSON, repairing anything missing. Never throws. */
export function load(raw) {
  if (!raw) return clone(DEFAULTS);
  let parsed;
  try { parsed = JSON.parse(raw); } catch { return clone(DEFAULTS); }
  if (!parsed || parsed.version !== VERSION) return clone(DEFAULTS);
  const base = clone(DEFAULTS);
  const stats = { ...base.stats, ...(parsed.stats ?? {}) };
  if (!Array.isArray(stats.dist) || stats.dist.length !== 6) stats.dist = [0, 0, 0, 0, 0, 0];
  const daily = parsed.daily && typeof parsed.daily.puzzle === 'number'
    ? { puzzle: parsed.daily.puzzle,
        guesses: Array.isArray(parsed.daily.guesses) ? parsed.daily.guesses : [],
        status: parsed.daily.status ?? 'playing' }
    : null;
  return { version: VERSION, daily, stats };
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
