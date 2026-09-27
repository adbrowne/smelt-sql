// Pure game logic. No DOM, no localStorage, no implicit clock reads:
// every function that depends on "now" takes the Date as an argument.

export const WORD_LENGTH = 5;
export const MAX_GUESSES = 6;
export const EPOCH = new Date(2026, 0, 1);

const RANK = { absent: 0, present: 1, correct: 2 };

/**
 * Two-pass scoring. Pass 1 claims exact matches and decrements a pool of the
 * answer's unmatched letters; pass 2 hands out "present" only while the pool
 * still has that letter. Doing it in one pass is the classic clone bug: it
 * over-reports presents when the guess repeats a letter the answer has once.
 */
export function scoreGuess(guess, answer) {
  const marks = new Array(WORD_LENGTH).fill('absent');
  const pool = new Map();
  for (let i = 0; i < WORD_LENGTH; i++) {
    if (guess[i] === answer[i]) marks[i] = 'correct';
    else pool.set(answer[i], (pool.get(answer[i]) ?? 0) + 1);
  }
  for (let i = 0; i < WORD_LENGTH; i++) {
    if (marks[i] === 'correct') continue;
    const left = pool.get(guess[i]) ?? 0;
    if (left > 0) { marks[i] = 'present'; pool.set(guess[i], left - 1); }
  }
  return marks;
}

/** Best-result-wins merge of per-letter keyboard states. Returns a new object. */
export function mergeKeyStates(prev, guess, marks) {
  const next = { ...prev };
  for (let i = 0; i < guess.length; i++) {
    const letter = guess[i];
    const current = next[letter];
    if (current === undefined || RANK[marks[i]] > RANK[current]) next[letter] = marks[i];
  }
  return next;
}

const startOfDay = d => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

/** Whole local days elapsed since the epoch. */
export function puzzleNumber(date) {
  return Math.round((startOfDay(date) - startOfDay(EPOCH)) / 86400000);
}

export function dailyIndex(date, listLength) {
  return ((puzzleNumber(date) % listLength) + listLength) % listLength;
}

export function msUntilNextPuzzle(date) {
  const nextMidnight = new Date(date.getFullYear(), date.getMonth(), date.getDate() + 1);
  return nextMidnight.getTime() - date.getTime();
}

export function isValidGuess(word, allowedSet) {
  return word.length === WORD_LENGTH && allowedSet.has(word);
}

const EMOJI = { correct: '\u{1F7E9}', present: '\u{1F7E8}', absent: '⬜' };

export function shareText(puzzleNo, marks2d, won) {
  const score = won ? `${marks2d.length}/${MAX_GUESSES}` : `X/${MAX_GUESSES}`;
  const grid = marks2d.map(row => row.map(m => EMOJI[m]).join('')).join('\n');
  return `Anne's Words ${puzzleNo} ${score}\n\n${grid}`;
}
