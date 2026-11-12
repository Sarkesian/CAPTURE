<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import type { TimelineEvent } from '../lib/types';
  import { clamp, clock, containing, eventGroups, fitView, tickStep, zoomView, type Range } from '../lib/timeline';
  import { cachedThumb, thumbAt, type Thumb } from '../lib/thumbs';
  import { viewer } from '../lib/store.svelte';

  type Edge = 'a' | 'b';
  type DragMode = 'scrub' | 'pan' | Edge;
  let { locate, clockAt, wallAt, labels = [], first, last, intervals, events, clips = [], position, selection = $bindable(null), view = $bindable(),
    editable = true, onseek, onevent, ontrim, onstep }: {
    /** Timeline time to the session and session time there (null where nothing was recorded). */
    locate: (time: number) => { session: string; t: number } | null;
    /** Seconds of footage before a timeline time (the clock shown), and its wall-clock time in ms. */
    clockAt: (time: number) => number; wallAt: (time: number) => number | null;
    /** Where each gameplay session begins. */
    labels?: { t: number; text: string }[];
    first: number; last: number; intervals: [number, number][]; events: TimelineEvent[];
    /** Clips already saved from this session (shown at their place on the timeline). */
    clips?: TimelineEvent[];
    position: number; selection: Range | null; view: Range;
    editable?: boolean; onseek: (time: number, final: boolean) => void; onevent: (event: TimelineEvent) => void;
    ontrim: (active: boolean, edge: Edge) => void; onstep: (edge: Edge, direction: number) => void;
  } = $props();

  let detail: HTMLDivElement;
  let width = $state(800);
  let drag = $state<{ mode: DragMode; pointer: number; x: number; viewStart: number; offset: number } | null>(null);
  let edgeX: number | null = null;
  let edgeFrame = 0;
  let hover = $state<{ time: number; event?: TimelineEvent } | null>(null);
  let popup = $state<TimelineEvent[]>([]);
  let thumb = $state<Thumb | null>(null);
  const span = $derived(Math.max(.001, view.b - view.a));
  const pct = (time: number) => clamp((time - view.a) / span * 100, 0, 100);
  const groups = $derived(eventGroups(events, view, width));
  // Tick marks everywhere; numbers only when the view is inside one stretch of footage (round
  // footage times there). Zoomed out over several stretches, the session dates orient instead.
  const ticks = $derived.by(() => {
    const step = tickStep(span, width);
    const inside = intervals.find(([a, b]) => view.a >= a - .001 && view.b <= b + .001);
    const times: { t: number; label: string }[] = [];
    if (inside) {
      const base = clockAt(inside[0]);
      for (let f = Math.ceil((base + view.a - inside[0]) / step) * step; times.length < 80; f += step) {
        const t = inside[0] + f - base;
        if (t >= view.b) break;
        times.push({ t, label: step < 1 ? clock(f, true) : clock(f) });
      }
    } else {
      for (let t = Math.ceil(view.a / step) * step; t < view.b && times.length < 80; t += step) if (containing(intervals, t)) times.push({ t, label: '' });
    }
    return times;
  });
  // Session labels that have room (a label is dropped when it would run into the one before it).
  const shownLabels = $derived.by(() => {
    let edge = -Infinity;
    return labels.filter(l => l.t >= view.a - .001 && l.t < view.b).filter(l => {
      const x = (l.t - view.a) / span * width;
      if (x < edge || x > width - 60) return false;
      edge = x + 110;
      return true;
    });
  });
  /** Seconds of recording under one pixel: decides how precise hovering needs to be. */
  const perPixel = $derived(span / Math.max(1, width));
  const hoverFootage = $derived(!!hover && !!containing(intervals, hover.time));
  const timeOfDay = (t: number) => { const ms = wallAt(t); return ms === null ? '' : new Date(ms).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit', second: perPixel < 2 ? '2-digit' : undefined }); };

  // The picture under the cursor comes from the engine, for any recorded position. Zoomed out,
  // the keyframe at that second is exact to the pixel; zoomed in, the very frame is asked for.
  // A picture of somewhere else is never left on screen: it is cleared as soon as the cursor
  // is further from it than the precision in use, and a late answer for an old position is dropped.
  $effect(() => {
    const at = hover?.time;
    const from = at === undefined || hover?.event?.kind === 'screenshot' ? null : locate(at);
    if (at === undefined || !from) { thumb = null; return; }
    const exact = perPixel < 0.12;
    const tolerance = exact ? 0.12 : Math.max(1.05, perPixel * 6);
    // Pictures are kept by session time; `thumb.t` is on the timeline's own clock.
    const place = (found: Thumb): Thumb => ({ image: found.image, t: at - (from.t - found.t) });
    const hit = cachedThumb(from.session, from.t, exact);
    untrack(() => {
      if (hit) thumb = place(hit);
      else if (thumb && (at - thumb.t > tolerance || thumb.t - at > 0.12)) thumb = null;
    });
    if (hit) return;
    let current = true;
    const timer = setTimeout(() => {
      thumbAt(from.session, from.t, exact, viewer + '-timeline').then(result => { if (current && result) thumb = place(result); });
    }, 25);
    return () => { current = false; clearTimeout(timer); };
  });

  onMount(() => {
    const observer = new ResizeObserver(() => width = detail.getBoundingClientRect().width);
    observer.observe(detail);
    function wheel(event: WheelEvent) {
      event.preventDefault();
      if (drag) return;
      hover = null;
      popup = [];
      if (event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) pan((event.deltaX || event.deltaY) / width * span);
      else zoom(Math.exp(clamp(event.deltaY, -100, 100) * .008), timeAt(event.clientX));
    }
    detail.addEventListener('wheel', wheel, { passive: false });
    return () => {
      cancelAnimationFrame(edgeFrame);
      observer.disconnect();
      detail.removeEventListener('wheel', wheel);
    };
  });

  function timeAt(x: number) {
    const box = detail.getBoundingClientRect();
    // Always a time inside the session: the cursor, the playhead and the media share one clock.
    return clamp(view.a + clamp((x - box.left) / Math.max(1, box.width), 0, 1) * span, first, last);
  }
  export function zoom(factor: number, at = position >= view.a && position <= view.b ? position : (view.a + view.b) / 2) {
    view = zoomView(view, factor, at, first, last);
  }
  export function pan(delta: number) {
    view = fitView(view.a + delta, span, first, last);
  }
  export function fit() { view = { a: first, b: last }; }

  function start(event: PointerEvent, mode: DragMode) {
    if (drag || (event.button !== 0 && event.button !== 1)) return;
    if (event.button === 1 || event.altKey) mode = 'pan';
    const edge = mode === 'a' || mode === 'b' ? mode : null;
    if (edge && (!editable || !selection)) return;
    event.preventDefault();
    (event.currentTarget as HTMLElement).focus({ preventScroll: true });
    popup = [];
    const time = timeAt(event.clientX);
    drag = {
      mode, pointer: event.pointerId, x: event.clientX, viewStart: view.a,
      offset: edge && selection ? selection[edge] - time : 0,
    };
    // The strip stays mounted if a moving handle reaches the edge of the viewport.
    detail.setPointerCapture(event.pointerId);
    if (edge) {
      ontrim(true, edge);
      hover = { time: selection![edge] };
    } else if (mode === 'scrub') {
      hover = { time };
      onseek(time, false);
    }
  }

  function resize(edge: Edge, time: number) {
    if (!selection) return;
    const value = edge === 'a' ? clamp(time, first, selection.b - .001) : clamp(time, selection.a + .001, last);
    selection = { ...selection, [edge]: value };
    hover = { time: value };
  }

  function followHandle() {
    edgeFrame = 0;
    if (!drag || edgeX === null || (drag.mode !== 'a' && drag.mode !== 'b')) return;
    const box = detail.getBoundingClientRect();
    const distance = edgeX < box.left + 12 ? edgeX - box.left - 12 : Math.max(0, edgeX - box.right + 12);
    if (!distance) return;
    const previous = view.a;
    pan(clamp(distance / 40, -1, 1) * span * .018);
    if (previous === view.a) return;
    resize(drag.mode, timeAt(edgeX) + drag.offset);
    edgeFrame = requestAnimationFrame(followHandle);
  }

  function move(event: PointerEvent) {
    if (!drag) {
      if (!popup.length && event.pointerType !== 'touch') hover = { time: timeAt(event.clientX) };
      return;
    }
    if (drag.pointer !== event.pointerId) return;
    if (!event.buttons) { finish(event); return; }
    const time = timeAt(event.clientX);
    if (drag.mode === 'pan') {
      view = fitView(drag.viewStart - (event.clientX - drag.x) / width * span, span, first, last);
    } else if (drag.mode === 'scrub') {
      hover = { time };
      onseek(time, false);
    } else {
      resize(drag.mode, time + drag.offset);
      edgeX = event.clientX;
      if (!edgeFrame) edgeFrame = requestAnimationFrame(followHandle);
    }
  }

  function finish(event: PointerEvent, cancelled = false) {
    if (!drag || drag.pointer !== event.pointerId) return;
    const { mode, pointer } = drag;
    drag = null;
    hover = null;
    edgeX = null;
    cancelAnimationFrame(edgeFrame);
    edgeFrame = 0;
    if (detail.hasPointerCapture(pointer)) detail.releasePointerCapture(pointer);
    if (mode === 'a' || mode === 'b') ontrim(false, mode);
    else if (mode === 'scrub' && !cancelled) onseek(timeAt(event.clientX), true);
  }

  function handleKey(event: KeyboardEvent, edge: Edge) {
    if (!editable || (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight')) return;
    event.preventDefault();
    event.stopPropagation();
    onstep(edge, event.key === 'ArrowRight' ? 1 : -1);
  }
  function clearHover() { if (!drag) hover = null; }
  const momentTitle = (event: TimelineEvent) => event.label || (event.kind === 'screenshot' ? 'Screenshot' : 'Marker');
</script>

<div class="timeline">
  <div class="seek-strip">
    <div class="detail" bind:this={detail} role="slider" tabindex="0" aria-label="Recording timeline"
      aria-valuemin={view.a} aria-valuemax={view.b} aria-valuenow={clamp(position,view.a,view.b)} aria-valuetext={clock(clockAt(position),3)}
      onpointerdown={event => start(event, 'scrub')} onpointermove={move} onpointerleave={clearHover}
      onpointerup={event => finish(event)} onpointercancel={event => finish(event, true)} onlostpointercapture={event => finish(event, true)}>
      <div class="footage-lane">
        {#each intervals as [a,b]}
          {#if b > view.a && a < view.b}
            <div class="footage" style:left="{pct(a)}%" style:width="{pct(b)-pct(a)}%"></div>
            {#if !selection && position > a}<div class="played" style:left="{pct(a)}%" style:width="{pct(Math.min(b,position))-pct(a)}%"></div>{/if}
          {/if}
        {/each}
        {#if selection}
          <div class="dim" style:width="{pct(selection.a)}%"></div>
          <div class="dim" style:left="{pct(selection.b)}%" style:width="{100-pct(selection.b)}%"></div>
          <div class="clip-range" style:left="{pct(selection.a)}%" style:width="{pct(selection.b)-pct(selection.a)}%"></div>
        {/if}
      </div>
      {#each clips as clip (clip.id)}
        {#if (clip.end ?? clip.t) > view.a && clip.t < view.b}<div class="saved-clip" style:left="{pct(clip.t)}%" style:width="{pct(clip.end ?? clip.t)-pct(clip.t)}%"></div>{/if}
      {/each}
      {#each ticks as tick}<span class="tick" style:left="{pct(tick.t)}%">{#if tick.label && pct(tick.t) > 1 && pct(tick.t) < 95}<i>{tick.label}</i>{/if}</span>{/each}
      {#each shownLabels as label (label.t)}<span class="session-label" style:left="{pct(label.t)}%">{label.text}</span>{/each}
      {#if position >= view.a && position <= view.b}<div class="playhead" style:left="{pct(position)}%"></div>{/if}
    </div>
    {#each groups as group}
      {@const event = group.events[0]}
      {@const count = group.events.length}
      <button class="moment" class:shot={group.events.every(e => e.kind === 'screenshot')} class:grouped={count > 1}
        style:left="{pct(group.t)}%" aria-label={count>1 ? `${count} moments at ${clock(clockAt(group.t))}` : `${momentTitle(event)} at ${clock(clockAt(group.t))}`}
        onpointerenter={() => hover={time:group.t,event:count===1?event:undefined}} onpointerleave={clearHover}
        onfocus={() => hover={time:group.t,event:count===1?event:undefined}} onblur={clearHover}
        onclick={() => { hover=null; if(count===1)onevent(event);else popup=group.events; }}></button>
    {/each}
    {#if selection}
      {#each (['a','b'] as const) as edge}
        {#if selection[edge] >= view.a-.000001 && selection[edge] <= view.b+.000001}
          <button class="handle" class:start={edge==='a'} class:end={edge==='b'} class:dragging={drag?.mode===edge}
            style:left="{pct(selection[edge])}%" disabled={!editable} aria-label={edge==='a'?'Clip start handle':'Clip end handle'}
            title={`${edge==='a'?'In':'Out'} ${clock(clockAt(selection[edge]),3)} · Arrow keys step ${edge==='a'?'keyframes':'frames'}`}
            onpointerdown={event => start(event, edge)} onkeydown={event => handleKey(event, edge)}></button>
        {/if}
      {/each}
    {/if}
    {#if hover && !popup.length}
      <div class="hover-card" style:left="clamp(106px, {pct(hover.time)}%, calc(100% - 106px))" role="tooltip">
        {#if hover.event?.kind==='screenshot' && hover.event.path}
          <img src={convertFileSrc(hover.event.path)} alt="Screenshot preview" />
        {:else if hoverFootage}<div class="frame">{#if thumb}<img src={thumb.image} alt="Video frame preview" />{/if}</div>{/if}
        {#if hover.event}<strong>{momentTitle(hover.event)}</strong>{/if}
        <span class="when"><b>{clock(clockAt(hover.time), perPixel < 0.05 ? 3 : perPixel < 1.5)}</b><small>{hoverFootage ? timeOfDay(hover.time) : 'Not recorded'}</small></span>
      </div>
    {/if}
    {#if popup.length}
      <div class="cluster" role="region" aria-label="Grouped moments">
        <div class="row"><strong class="grow">{popup.length} nearby moments</strong><button class="btn ghost sm" onclick={() => popup=[]}>Close</button></div>
        {#each popup as event (event.id)}
          <button class="cluster-event" onclick={() => { onevent(event); popup=[]; }}>
            <span class="grow">{momentTitle(event)}</span><time>{clock(clockAt(event.t),true)}</time>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .timeline { position: relative; flex: none; padding: 0 18px; background: var(--surface); }
  .seek-strip { position: relative; height: 58px; margin-top: 4px; }
  .detail { position: absolute; inset: 0; touch-action: none; cursor: crosshair; user-select: none; }
  .footage-lane { position: absolute; inset: 21px 0 auto; height: 9px; overflow: hidden; border-radius: 3px;
    background: repeating-linear-gradient(135deg, var(--bg) 0 4px, var(--surface-3) 4px 5px); }
  .footage, .played, .dim, .clip-range { position: absolute; height: 100%; }
  .footage { background: color-mix(in srgb, var(--accent) 34%, var(--surface-3)); min-width: 1px; }
  .played { background: color-mix(in srgb, var(--accent) 80%, var(--surface)); }
  .dim { background: #080b1299; }
  .clip-range { background: color-mix(in srgb, var(--accent) 25%, transparent); border-block: 1px solid var(--accent); }
  .session-label { position: absolute; top: 0; padding-left: 1px; font-size: 10px; line-height: 12px; color: var(--text-2); white-space: nowrap; pointer-events: none; }
  .tick { position: absolute; top: 33px; height: 4px; width: 1px; background: var(--line-strong); pointer-events: none; }
  .tick i { position: absolute; top: 6px; left: 0; transform: translateX(-50%); font-style: normal; font-size: 10px; line-height: 1; color: var(--text-2);
    font-variant-numeric: tabular-nums; white-space: nowrap; }
  .saved-clip { position: absolute; top: 16px; height: 3px; min-width: 3px; border-radius: 2px; background: var(--ok, #4cc38a); pointer-events: none; }
  .playhead { position: absolute; top: 15px; height: 21px; width: 2px; background: var(--text); pointer-events: none; z-index: 2; }
  .playhead::before { content: ''; position: absolute; top: -3px; left: -3px; width: 8px; height: 5px; background: var(--text); clip-path: polygon(0 0,100% 0,50% 100%); }
  .moment, .handle { position: absolute; border: 0; padding: 0; background: transparent; touch-action: none; }
  .moment { top: 4px; width: 22px; height: 17px; transform: translateX(-50%); cursor: pointer; color: var(--warn); }
  .moment::after { content: ''; position: absolute; width: 5px; height: 5px; top: 6px; left: 8px; transform: rotate(45deg); background: currentColor; }
  .moment.shot { color: var(--accent); }
  .moment.shot::after { transform: none; border-radius: 1px; }
  .moment.grouped::after { box-shadow: 3px -3px 0 color-mix(in srgb, currentColor 65%, transparent); }
  .moment:hover::after, .moment:focus-visible::after { outline: 1px solid currentColor; outline-offset: 2px; }
  .handle { top: 12px; width: 24px; height: 29px; cursor: ew-resize; z-index: 3; }
  .handle.start { transform: translateX(-85%); }
  .handle.end { transform: translateX(-15%); }
  .handle::after { content: ''; position: absolute; top: 5px; left: calc(15% - 2px); width: 4px; height: 17px; border-radius: 2px; background: var(--accent); }
  .handle.start::after { left: calc(85% - 2px); }
  .handle:hover::after, .handle.dragging::after, .handle:focus-visible::after { background: var(--accent-hover); box-shadow: 0 0 0 2px var(--accent-soft); }
  .hover-card, .cluster { position: absolute; bottom: calc(100% + 5px); z-index: 8; background: var(--surface-2); border: 1px solid var(--line-strong); border-radius: 8px; box-shadow: var(--shadow-pop); }
  .hover-card { display: grid; gap: 4px; width: max-content; max-width: 212px; padding: 6px 9px; transform: translateX(-50%); pointer-events: none; font-size: 12px; font-variant-numeric: tabular-nums; }
  .hover-card img { width: 192px; max-height: 108px; object-fit: contain; background: #000; border-radius: 4px; display: block; }
  .hover-card .frame { width: 192px; aspect-ratio: 16/9; background: #05070a; border-radius: 4px; overflow: hidden; display: grid; place-items: center; }
  .hover-card .frame img { width: 100%; height: 100%; max-height: none; }
  .when { display: flex; justify-content: space-between; align-items: baseline; gap: 10px; }
  .when b { font-weight: 600; } .when small { color: var(--text-2); font-size: 11px; }
  .hover-card strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cluster { right: 0; width: min(390px,100%); max-height: 240px; overflow: auto; padding: 10px; }
  .cluster-event { display: flex; align-items: center; gap: 10px; width: 100%; padding: 8px; background: transparent; border: 0; text-align: left; cursor: pointer; border-radius: 5px; color: var(--text); }
  .cluster-event:hover { background: var(--surface-3); }
  .cluster-event time { font-variant-numeric: tabular-nums; color: var(--text-2); }
</style>
