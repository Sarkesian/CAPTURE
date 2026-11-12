// One timeline per game. Every recorded stretch of every session of the game is laid end to
// end in the order it was played; the time in between (the game was closed, or nothing was
// captured) takes no room on the strip: it is drawn as a small fixed break of a few pixels
// (see ReplayTimeline), whatever the zoom, so the footage itself fills the strip. Positions on that strip ("line time") map both ways to a session
// and its own clock, which is what the engine works in.
import type { HistorySession } from './types';

export interface Piece { session: string; a: number; b: number; v: number; origin: number }
export interface GameLine { pieces: Piece[]; intervals: [number, number][]; total: number; gap: number }
/** Line time between two pieces: just enough that they never touch (a click lands in one). */
const BREAK = .02;
export const emptyLine: GameLine = { pieces: [], intervals: [], total: 0, gap: BREAK };

/** Per piece: whether a visible break follows it. Holes of a second or two inside one session
 * (a dropped segment) are not worth a break on the strip; the end of a session always is. */
export function seams(line: GameLine): boolean[] {
  return line.pieces.map((p, i) => { const n = line.pieces[i + 1]; return !!n && (n.session !== p.session || n.a - p.b >= 2); });
}

/**
 * `previous`: the line built from the last state of the same game. Positions stay where they
 * were (the playhead, the view and a selection must not jump) when footage is added at the
 * end or the oldest footage ages out at the start.
 */
export function buildLine(sessions: HistorySession[], previous?: GameLine): GameLine {
  const stretches = sessions.flatMap(s => s.intervals.map(([a, b]) => ({ session: s.id, a, b, origin: s.created_ms })));
  const total = stretches.reduce((n, p) => n + p.b - p.a, 0);
  const gap = BREAK;
  let v = 0;
  const pieces: Piece[] = stretches.map(p => { const piece = { ...p, v }; v += p.b - p.a + gap; return piece; });
  if (previous) {
    for (const piece of pieces) {
      const old = previous.pieces.find(o => o.session === piece.session && o.a < piece.b && o.b > piece.a);
      if (!old) continue;
      const t = Math.max(old.a, piece.a), shift = old.v + (t - old.a) - (piece.v + (t - piece.a));
      if (shift) for (const p of pieces) p.v += shift;
      break;
    }
  }
  return { pieces, intervals: pieces.map(p => [p.v, p.v + p.b - p.a]), total, gap };
}

/** The session and session time at line time `v`, when footage is there. */
export function toReal(line: GameLine, v: number): { session: string; t: number; piece: Piece } | null {
  const piece = line.pieces.find(p => v >= p.v - 1e-6 && v < p.v + (p.b - p.a) + 1e-6);
  return piece ? { session: piece.session, t: Math.min(piece.b, Math.max(piece.a, piece.a + v - piece.v)), piece } : null;
}

/** Line time of a session moment; null when its footage is no longer kept. */
export function toLine(line: GameLine, session: string, t: number, tolerance = .3): number | null {
  const piece = line.pieces.find(p => p.session === session && t >= p.a - tolerance && t <= p.b + tolerance);
  return piece ? piece.v + Math.min(piece.b, Math.max(piece.a, t)) - piece.a : null;
}

/** Seconds of footage before line time `v` (the clock the player shows: breaks do not count). */
export function footage(line: GameLine, v: number): number {
  let n = 0;
  for (const p of line.pieces) {
    if (v <= p.v) break;
    n += Math.min(p.b - p.a, v - p.v);
  }
  return n;
}

/** Wall-clock time (ms) of line time `v`, when footage is there. */
export function wallClock(line: GameLine, v: number): number | null {
  const real = toReal(line, v);
  return real ? real.piece.origin + real.t * 1000 : null;
}
