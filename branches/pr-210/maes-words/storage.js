// Pure serialisation. The caller owns localStorage so this runs under node.
import { MAX_LEVEL } from './game.js';

export const KEY = 'maes-words:v1';
export const VERSION = 1;

export const serialize = state => JSON.stringify({ version: VERSION, ...state });

/** Parse a stored save, repairing what it can. Returns null when there is nothing usable. Never throws. */
export function load(raw, words) {
  if (!raw) return null;
  let parsed;
  try { parsed = JSON.parse(raw); } catch { return null; }
  if (!parsed || typeof parsed !== 'object' || parsed.version !== VERSION) return null;
  const known = new Set(words.map(x => x.w));
  const levels = {};
  const src = parsed.levels && typeof parsed.levels === 'object' ? parsed.levels : {};
  for (const [w, v] of Object.entries(src)) {
    if (!known.has(w)) continue;
    const n = Number(v);
    if (!Number.isFinite(n)) continue;
    levels[w] = Math.min(MAX_LEVEL, Math.max(0, Math.floor(n)));
  }
  const starsN = Number(parsed.stars);
  const stars = Number.isFinite(starsN) ? Math.max(0, Math.floor(starsN)) : 0;
  const seen = new Set();
  const learnedOrder = (Array.isArray(parsed.learnedOrder) ? parsed.learnedOrder : [])
    .filter(w => typeof w === 'string' && known.has(w) && !seen.has(w) && seen.add(w));
  return { levels, stars, learnedOrder };
}
