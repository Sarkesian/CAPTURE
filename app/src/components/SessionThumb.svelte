<script lang="ts">
  // A real frame of a background-recording session for its Library card, so sessions of the
  // same game are told apart at a glance (game art alone made every card look identical).
  import { queuedThumb } from '../lib/thumbs';
  import type { HistorySession } from '../lib/types';
  let { session }: { session: HistorySession } = $props();
  let host: HTMLSpanElement;
  let image = $state('');
  let visible = $state(false);
  // A moment that stays put while the session grows (live) or its oldest footage ages out.
  const at = $derived.by(() => {
    const longest = session.intervals.reduce<[number, number] | null>((best, i) => !best || i[1] - i[0] > best[1] - best[0] ? i : best, null);
    if (!longest) return null;
    const [a, b] = longest;
    return Math.floor(session.ended ? Math.max(a, b - Math.min(30, (b - a) / 2)) : a + Math.min(30, (b - a) * .4));
  });
  $effect(() => {
    const observer = new IntersectionObserver(entries => { if (entries.some(e => e.isIntersecting)) { visible = true; observer.disconnect(); } }, { rootMargin: '200px' });
    observer.observe(host);
    return () => observer.disconnect();
  });
  $effect(() => {
    const t = at, id = session.id;
    if (!visible || t === null) return;
    let current = true;
    queuedThumb(id, t).then(thumb => { if (current && thumb) image = thumb.image; });
    return () => { current = false; };
  });
</script>

<span class="thumb" bind:this={host}>{#if image}<img src={image} alt="" />{/if}</span>

<style>
  .thumb { position: absolute; inset: 0; border-radius: inherit; overflow: hidden; pointer-events: none; }
  img { width: 100%; height: 100%; object-fit: cover; display: block; }
</style>
