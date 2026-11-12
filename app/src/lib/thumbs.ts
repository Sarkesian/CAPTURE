// Frame thumbnails of background recordings, made by the engine straight from the recorded
// footage (any position, not just the part loaded in the player). This side only remembers
// recent ones so moving back over the same stretch of timeline is instant.
import { historyThumb } from './store.svelte';

export interface Thumb { t: number; image: string }

const LIMIT = 320;
const cache = new Map<string, Thumb>();
const running = new Map<string, Promise<Thumb | null>>();

/** Keyframe pictures are shared per second; exact ones per 1/20 s. */
const keyOf = (session: string, at: number, exact: boolean) => `${session}|${exact ? 'x' + Math.round(at * 20) : Math.floor(at)}`;

export function cachedThumb(session: string, at: number, exact: boolean): Thumb | undefined {
  const key = keyOf(session, at, exact);
  const hit = cache.get(key) ?? (exact ? cache.get(keyOf(session, at, false)) : undefined);
  if (hit && cache.has(key)) { cache.delete(key); cache.set(key, hit); }
  return hit;
}

/**
 * `as`: who is asking. The engine keeps only the newest pending request per asker, so a view
 * that fires many (hovering) never queues stale work, and independent askers (the timeline,
 * the library cards) do not cancel each other.
 */
/** Thumbnails for many cards at once: one at a time, in the order asked, without superseding each other. */
let chain: Promise<unknown> = Promise.resolve();
export function queuedThumb(session: string, at: number): Promise<Thumb | null> {
  const hit = cache.get(keyOf(session, at, false));
  if (hit) return Promise.resolve(hit);
  const job = chain.then(() => thumbAt(session, at, false, 'library'));
  chain = job.catch(() => null);
  return job;
}

export function thumbAt(session: string, at: number, exact: boolean, as?: string): Promise<Thumb | null> {
  const key = keyOf(session, at, exact);
  const hit = cache.get(key);
  if (hit) return Promise.resolve(hit);
  const pending = running.get(key);
  if (pending) return pending;
  const job = historyThumb(session, at, exact, as).then(thumb => {
    if (thumb) {
      cache.set(key, thumb);
      if (cache.size > LIMIT) cache.delete(cache.keys().next().value!);
    }
    return thumb;
  }).finally(() => running.delete(key));
  running.set(key, job);
  return job;
}
