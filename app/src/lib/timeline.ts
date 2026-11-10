import type { TimelineEvent } from './types';

export type Range = { a: number; b: number };
export const clamp = (n: number, a: number, b: number) => Math.max(a, Math.min(b, n));
export function clock(t: number, precise = false): string {
  const n = Math.max(0, t), tenths = Math.round(n * 10), s = precise ? Math.floor(tenths / 10) : Math.floor(n);
  const text = `${Math.floor(s / 3600)}:${String(Math.floor(s / 60) % 60).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
  return precise ? `${text}.${tenths % 10}` : text;
}
export function parseClock(value: string): number | null {
  const parts = value.trim().split(':');
  if (!parts.length || parts.length > 3 || parts.some(p => !/^\d+(\.\d+)?$/.test(p))) return null;
  const nums = parts.map(Number);
  if (nums.some(n => !Number.isFinite(n)) || nums.slice(1).some(n => n >= 60)) return null;
  return nums.reduce((total, n) => total * 60 + n, 0);
}
export function fitView(a: number, span: number, first: number, last: number): Range {
  span = clamp(span, Math.min(2, last - first), last - first);
  a = clamp(a, first, last - span);
  return { a, b: a + span };
}
export function zoomView(view: Range, factor: number, anchor: number, first: number, last: number): Range {
  const span = view.b - view.a;
  const size = clamp(span * factor, Math.min(2, last - first), last - first);
  return fitView(anchor - (anchor - view.a) / span * size, size, first, last);
}
export function tickStep(span: number, width: number): number {
  const target = span / Math.max(2, width / 90);
  return [0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 14400].find(s => s >= target) ?? 21600;
}
// Cluster in screen space, rather than dropping events or placing buttons over one another.
export function eventGroups(events: TimelineEvent[], view: Range, width: number) {
  const groups: { t: number; events: TimelineEvent[] }[] = [];
  const distance = (view.b - view.a) * 28 / Math.max(1, width - 28);
  for (const ev of events.filter(e => e.t >= view.a && e.t <= view.b).sort((a, b) => a.t - b.t)) {
    const group = groups.at(-1);
    if (group && ev.t - group.t < distance) group.events.push(ev);
    else groups.push({ t: ev.t, events: [ev] });
  }
  return groups;
}
export function containing(intervals: [number, number][], t: number) {
  return intervals.find(([a, b]) => t >= a && t < b);
}
export function validSelection(intervals: [number, number][], range: Range | null) {
  return !!range && range.b - range.a >= 0.1 && intervals.some(([a, b]) => range.a >= a - 0.001 && range.b <= b + 0.001);
}
export function selectionAround(intervals: [number, number][], t: number, before: number, after: number): Range | null {
  const interval = containing(intervals, t);
  return interval ? { a: Math.max(interval[0], t - before), b: Math.min(interval[1], t + after) } : null;
}
