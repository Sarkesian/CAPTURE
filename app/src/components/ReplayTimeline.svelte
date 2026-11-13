<script lang="ts">
  // The recording timeline: one strip for seeking, markers, clip trimming, zooming and panning.
  //
  // Interaction model
  // - Hover: a ghost line and a card with the picture and time under the cursor.
  // - Drag on the strip: scrub. Drag a clip handle: trim. Drag the clip's length label: move it.
  // - Wheel: zoom around the cursor. Shift + wheel, a sideways wheel, middle-drag or the
  //   scrollbar: pan. Dragging near an edge of a zoomed view pans along.
  // - Everything dragged snaps to nearby markers, clip edges, the playhead and the ends of
  //   footage (a guide line shows what it caught). Hold Shift to move freely.
  // - Right-click: actions for that moment or marker.
  import { onMount, untrack } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import type { TimelineEvent } from '../lib/types';
  import { clamp, clock, containing, eventGroups, fitView, tickStep, zoomView, type Range } from '../lib/timeline';
  import { cachedThumb, thumbAt, type Thumb } from '../lib/thumbs';
  import { viewer } from '../lib/store.svelte';
  import Icon from './Icon.svelte';

  type Edge = 'a' | 'b';
  export type TimelineAction = 'clip' | 'start' | 'end' | 'goto' | 'rename' | 'delete';
  /** Width of the break between two stretches of footage, at every zoom (like a seam, not a span of time). */
  const BREAK_PX = 3;
  /** How close (in pixels) something dragged has to come to a snap point to catch on it. */
  const SNAP_PX = 8;
  const MIN_CLIP = 0.1;
  type DragMode = 'scrub' | 'pan' | 'move' | 'bar' | Edge;
  let { locate, clockAt, wallAt, labels = [], first, last, intervals, seams = [], events, clips = [], position, selection = $bindable(null), view = $bindable(),
    editable = true, focused = '', onseek, onevent, ontrim, onstep, onaction }: {
    /** Timeline time to the session and session time there (null where nothing was recorded). */
    locate: (time: number) => { session: string; t: number } | null;
    /** Seconds of footage before a timeline time (the clock shown), and its wall-clock time in ms. */
    clockAt: (time: number) => number; wallAt: (time: number) => number | null;
    /** Where each gameplay session begins. */
    labels?: { t: number; text: string }[];
    first: number; last: number; intervals: [number, number][]; events: TimelineEvent[];
    /** Per interval: a break follows it (drawn a fixed few pixels wide, whatever the zoom). */
    seams?: boolean[];
    /** Clips already saved from this session (shown at their place on the timeline). */
    clips?: TimelineEvent[];
    position: number; selection: Range | null; view: Range;
    editable?: boolean;
    /** The marker the playhead was last sent to. */
    focused?: string;
    onseek: (time: number, final: boolean) => void; onevent: (event: TimelineEvent) => void;
    ontrim: (active: boolean, edge: Edge) => void; onstep: (edge: Edge, direction: number) => void;
    /** Something chosen from the right-click menu. */
    onaction: (action: TimelineAction, time: number, event?: TimelineEvent) => void;
  } = $props();

  let detail: HTMLDivElement;
  let width = $state(800);
  let drag = $state<{ mode: DragMode; pointer: number; x: number; viewStart: number; offset: number; length: number; bounds: [number, number]; last: number } | null>(null);
  let edgeX: number | null = null;
  let edgeFrame = 0;
  let free = false;
  let hover = $state<{ time: number; event?: TimelineEvent; label?: string } | null>(null);
  let snapped = $state<{ t: number; what: string } | null>(null);
  let popup = $state<TimelineEvent[]>([]);
  let menu = $state<{ x: number; y: number; time: number; event?: TimelineEvent } | null>(null);
  let thumb = $state<Thumb | null>(null);
  const total = $derived(Math.max(.001, last - first));
  const span = $derived(Math.max(.001, view.b - view.a));
  const pct = (time: number) => clamp((time - view.a) / span * 100, 0, 100);
  const zoomed = $derived(span < total - .001);
  const groups = $derived(eventGroups(events, view, width));
  // The ruler under the footage: labelled major ticks with finer ones between them, at every
  // zoom. Times are footage times (breaks do not count), so they agree with the clock shown.
  const ticks = $derived.by(() => {
    const major = tickStep(span, width);
    const minor = major / ([.2, 2, 120, 7200, 14400].includes(major) ? 4 : [15, 900].includes(major) ? 3 : 5);
    const marks: { t: number; label: string }[] = [];
    let labelled = -Infinity;
    for (const [a, b] of intervals) {
      if (b <= view.a || a >= view.b) continue;
      const base = clockAt(a), from = Math.max(a, view.a), to = Math.min(b, view.b);
      for (let n = Math.ceil((base + from - a) / minor - 1e-6); marks.length < 400; n++) {
        const f = n * minor, t = a + f - base;
        if (t >= to) break;
        const isMajor = Math.abs(f / major - Math.round(f / major)) < 1e-6;
        const x = (t - view.a) / span * width;
        // A number only where it has room (stretches of footage can start close together).
        const show = isMajor && x - labelled > 64 && x > 22 && x < width - 22;
        if (show) labelled = x;
        marks.push({ t, label: show ? (major < 1 ? clock(f, true) : clock(f)) : isMajor ? ' ' : '' });
      }
    }
    return marks;
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
  const momentTitle = (event: TimelineEvent) => event.label || (event.kind === 'screenshot' ? 'Screenshot' : 'Marker');
  /** A clip length the way people say it: "12.4 s", "1:05.2". */
  const lengthText = (secs: number) => secs < 60 ? `${secs.toFixed(1)} s` : `${Math.floor(secs / 60)}:${(secs % 60).toFixed(1).padStart(4, '0')}`;

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
    // Zoomed in, a keyframe picture from the same second only stands in until the very frame arrives.
    if (hit && (!exact || Math.abs(hit.t - from.t) < .06)) return;
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
      popup = [];
      menu = null;
      if (event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) pan((event.deltaX || event.deltaY) / width * span);
      else zoom(Math.exp(clamp(event.deltaY, -100, 100) * .0022));
      // The cursor has not moved, but what is under it may have.
      pointerX = event.clientX;
      hover = { time: timeAt(event.clientX) };
    }
    // A drag never outlives the button that started it, wherever the release happened.
    const drop = () => end(null);
    const away = (event: PointerEvent) => { if (menu && !(event.target as HTMLElement)?.closest('.context')) menu = null; };
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape' && (menu || popup.length)) { menu = null; popup = []; event.stopPropagation(); } };
    detail.addEventListener('wheel', wheel, { passive: false });
    window.addEventListener('blur', drop);
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('keydown', key, true);
    return () => {
      cancelAnimationFrame(edgeFrame);
      cancelAnimationFrame(glide);
      observer.disconnect();
      detail.removeEventListener('wheel', wheel);
      window.removeEventListener('blur', drop);
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('keydown', key, true);
    };
  });

  function timeAt(x: number) {
    const box = detail.getBoundingClientRect();
    // Always a time inside the session: the cursor, the playhead and the media share one clock.
    return clamp(view.a + clamp((x - box.left) / Math.max(1, box.width), 0, 1) * span, first, last);
  }
  // Zooming always closes in on the playhead, and glides there instead of jumping. `goal` is
  // where the view is heading; anything else that moves the view (a drag, playback following
  // the playhead) simply takes over.
  let goal: Range | null = null;
  let glide = 0;
  let glided: Range | null = null;
  let pointerX: number | null = null;
  function step() {
    glide = 0;
    if (!goal || !glided || view.a !== glided.a || view.b !== glided.b) { goal = null; return; }
    const da = goal.a - view.a, db = goal.b - view.b;
    const done = Math.abs(da) + Math.abs(db) < span * .002 || matchMedia('(prefers-reduced-motion: reduce)').matches;
    view = glided = done ? goal : { a: view.a + da * .3, b: view.b + db * .3 };
    if (hover && !hover.event && !hover.label && pointerX !== null) hover = { time: timeAt(pointerX) };
    if (done) goal = null;
    else glide = requestAnimationFrame(step);
  }
  /** Show `size` seconds around the playhead (it keeps its place on screen; off screen, it is centred). */
  export function zoomTo(size: number, instant = false) {
    const from = goal ?? view, width = from.b - from.a;
    size = clamp(size, Math.min(2, total), total);
    const inside = position >= from.a && position <= from.b;
    const target = fitView(inside ? position - (position - from.a) / width * size : position - size / 2, size, first, last);
    if (instant) { goal = null; view = target; return; }
    goal = target;
    glided = { a: view.a, b: view.b };
    if (!glide) glide = requestAnimationFrame(step);
  }
  export function zoom(factor: number) { zoomTo(((goal ?? view).b - (goal ?? view).a) * factor); }
  export function pan(delta: number) {
    goal = null;
    view = fitView(view.a + delta, span, first, last);
  }
  export function fit() { zoomTo(total); }

  /** The nearest meaningful point within reach of `time`, or `time` itself. */
  function snap(time: number, mode: 'scrub' | 'edge'): number {
    snapped = null;
    if (free) return time;
    const reach = perPixel * SNAP_PX;
    let best: { t: number; what: string } | null = null;
    const offer = (t: number, what: string) => { if (Math.abs(t - time) <= reach && (!best || Math.abs(t - time) < Math.abs(best.t - time))) best = { t, what }; };
    for (const e of events) offer(e.t, momentTitle(e));
    for (const c of clips) { offer(c.t, 'Saved clip'); if (c.end != null) offer(c.end, 'Saved clip'); }
    if (mode === 'edge') {
      offer(position, 'Playhead');
      for (const [a, b] of intervals) { offer(a, 'Start of footage'); offer(b, 'End of footage'); }
    } else if (selection) { offer(selection.a, 'Clip start'); offer(selection.b, 'Clip end'); }
    snapped = best;
    return best ? (best as { t: number }).t : time;
  }

  function start(event: PointerEvent, mode: DragMode) {
    if (drag || (event.button !== 0 && event.button !== 1)) return;
    if (mode !== 'bar' && (event.button === 1 || event.altKey)) mode = 'pan';
    const edge = mode === 'a' || mode === 'b' ? mode : null;
    if ((edge || mode === 'move') && (!editable || !selection)) return;
    event.preventDefault();
    event.stopPropagation();
    (event.currentTarget as HTMLElement).focus({ preventScroll: true });
    popup = [];
    menu = null;
    goal = null;
    free = event.shiftKey;
    const time = timeAt(event.clientX);
    // A clip stays inside the stretch of footage it is in.
    const home = selection ? containing(intervals, mode === 'b' ? selection.a : Math.max(selection.a, selection.b - .001)) ?? containing(intervals, selection.a) : undefined;
    drag = {
      mode, pointer: event.pointerId, x: event.clientX, viewStart: view.a, last: time,
      offset: selection && (edge || mode === 'move') ? selection[edge ?? 'a'] - time : 0,
      length: selection ? selection.b - selection.a : 0,
      bounds: home ?? [first, last],
    };
    // The strip stays mounted if a moving handle reaches the edge of the viewport.
    detail.setPointerCapture(event.pointerId);
    if (edge || mode === 'move') {
      ontrim(true, edge ?? 'a');
      hover = { time: selection![edge ?? 'a'], label: edge === 'b' ? 'Clip end' : 'Clip start' };
    } else if (mode === 'scrub') apply(event.clientX);
  }

  /** Put whatever is being dragged where the pointer (at screen x) says. */
  function apply(x: number) {
    if (!drag) return;
    const time = timeAt(x);
    if (drag.mode === 'scrub') {
      const t = snap(time, 'scrub');
      drag.last = t;
      hover = { time: t };
      onseek(t, false);
    } else if (drag.mode === 'move' && selection) {
      const [lo, hi] = drag.bounds;
      const a = clamp(snap(time + drag.offset, 'edge'), lo, Math.max(lo, hi - drag.length));
      selection = { a, b: a + drag.length };
      hover = { time: a, label: 'Clip start' };
    } else if ((drag.mode === 'a' || drag.mode === 'b') && selection) {
      const [lo, hi] = drag.bounds;
      const t = snap(time + drag.offset, 'edge');
      const value = drag.mode === 'a' ? clamp(t, lo, selection.b - MIN_CLIP) : clamp(t, selection.a + MIN_CLIP, hi);
      selection = { ...selection, [drag.mode]: value };
      hover = { time: value, label: drag.mode === 'a' ? 'Clip start' : 'Clip end' };
    }
  }

  // Dragging to the edge of a zoomed view pans it along, faster the further out the pointer is.
  function follow() {
    edgeFrame = 0;
    if (!drag || edgeX === null || drag.mode === 'pan' || drag.mode === 'bar') return;
    const box = detail.getBoundingClientRect();
    const distance = edgeX < box.left + 12 ? edgeX - box.left - 12 : Math.max(0, edgeX - box.right + 12);
    if (!distance) return;
    const previous = view.a;
    pan(clamp(distance / 40, -1, 1) * span * .018);
    if (previous === view.a) return;
    apply(edgeX);
    edgeFrame = requestAnimationFrame(follow);
  }

  function move(event: PointerEvent) {
    pointerX = event.clientX;
    if (!drag) {
      if (!popup.length && !menu && event.pointerType !== 'touch') hover = { time: timeAt(event.clientX) };
      return;
    }
    if (drag.pointer !== event.pointerId) return;
    if (!event.buttons) { end(event.clientX); return; }
    free = event.shiftKey;
    if (drag.mode === 'pan') view = fitView(drag.viewStart - (event.clientX - drag.x) / width * span, span, first, last);
    else if (drag.mode === 'bar') view = fitView(drag.viewStart + (event.clientX - drag.x) / width * total, span, first, last);
    else {
      apply(event.clientX);
      edgeX = event.clientX;
      if (!edgeFrame) edgeFrame = requestAnimationFrame(follow);
    }
  }

  /** The drag is over: released (`x` = where), or cancelled (`null`). */
  function end(x: number | null) {
    if (!drag) return;
    const { mode, pointer, last: at } = drag;
    drag = null;
    hover = null;
    snapped = null;
    edgeX = null;
    cancelAnimationFrame(edgeFrame);
    edgeFrame = 0;
    if (detail.hasPointerCapture(pointer)) detail.releasePointerCapture(pointer);
    if (mode === 'a' || mode === 'b' || mode === 'move') ontrim(false, mode === 'b' ? 'b' : 'a');
    else if (mode === 'scrub' && x !== null) onseek(at, true);
  }
  const release = (event: PointerEvent, cancelled = false) => { if (drag?.pointer === event.pointerId) end(cancelled ? null : event.clientX); };

  function barDown(event: PointerEvent) {
    // A click on the track brings that part of the recording into view, then drags from there.
    const box = detail.getBoundingClientRect();
    const at = first + clamp((event.clientX - box.left) / Math.max(1, box.width), 0, 1) * total;
    if (at < view.a || at > view.b) view = fitView(at - span / 2, span, first, last);
    start(event, 'bar');
  }

  function handleKey(event: KeyboardEvent, edge: Edge) {
    if (!editable || (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight')) return;
    event.preventDefault();
    event.stopPropagation();
    onstep(edge, event.key === 'ArrowRight' ? 1 : -1);
  }
  function clearHover() { if (!drag) hover = null; }
  function context(event: MouseEvent, target?: TimelineEvent) {
    event.preventDefault();
    event.stopPropagation();
    if (drag) return;
    const box = detail.getBoundingClientRect();
    hover = null;
    popup = [];
    menu = { x: clamp(event.clientX - box.left, 0, box.width), y: event.clientY - box.top, time: target?.t ?? timeAt(event.clientX), event: target };
  }
  function act(action: TimelineAction) {
    const m = menu;
    menu = null;
    if (m) onaction(action, m.time, m.event);
  }
</script>

<div class="timeline" class:dragging={!!drag}>
  <div class="seek-strip">
    <div class="detail" bind:this={detail} role="slider" tabindex="0" aria-label="Recording timeline"
      aria-valuemin={view.a} aria-valuemax={view.b} aria-valuenow={clamp(position,view.a,view.b)} aria-valuetext={clock(clockAt(position),3)}
      onpointerdown={event => start(event, 'scrub')} onpointermove={move} onpointerleave={clearHover} oncontextmenu={event => context(event)}
      onpointerup={event => release(event)} onpointercancel={event => release(event, true)} onlostpointercapture={event => release(event, true)}>
      <div class="footage-lane">
        {#each intervals as [a,b], i}
          {#if b > view.a && a < view.b}
            {@const cut = seams[i] && b <= view.b ? BREAK_PX : 0}
            <div class="footage" style:left="{pct(a)}%" style:width="max(1px, calc({pct(b)-pct(a)}% - {cut}px))"></div>
            {#if !selection && position > a}<div class="played" style:left="{pct(a)}%" style:width="max(0px, calc({pct(Math.min(b,position))-pct(a)}% - {position >= b ? cut : 0}px))"></div>{/if}
          {/if}
        {/each}
        {#each clips as clip (clip.id)}
          {#if (clip.end ?? clip.t) > view.a && clip.t < view.b}<div class="saved-clip" style:left="{pct(clip.t)}%" style:width="{pct(clip.end ?? clip.t)-pct(clip.t)}%"></div>{/if}
        {/each}
        {#if selection}
          <div class="dim" style:width="{pct(selection.a)}%"></div>
          <div class="dim" style:left="{pct(selection.b)}%" style:width="{100-pct(selection.b)}%"></div>
          <div class="clip-range" style:left="{pct(selection.a)}%" style:width="{pct(selection.b)-pct(selection.a)}%"></div>
        {/if}
      </div>
      {#each ticks as tick}<span class="tick" class:major={!!tick.label} style:left="{pct(tick.t)}%">{#if tick.label.trim()}<i>{tick.label}</i>{/if}</span>{/each}
      {#each shownLabels as label (label.t)}<span class="session-label" style:left="{pct(label.t)}%">{label.text}</span>{/each}
      {#if hover && !drag && hoverFootage && !hover.event}<div class="ghost" style:left="{pct(hover.time)}%"></div>{/if}
      {#if snapped && snapped.t >= view.a && snapped.t <= view.b}<div class="snap-line" style:left="{pct(snapped.t)}%"></div>{/if}
      {#if position >= view.a && position <= view.b}<div class="playhead" style:left="{pct(position)}%"></div>{/if}
    </div>
    {#each groups as group}
      {@const event = group.events[0]}
      {@const count = group.events.length}
      <button class="moment" class:shot={group.events.every(e => e.kind === 'screenshot')} class:grouped={count > 1} class:focused={count === 1 && event.id === focused}
        style:left="{pct(group.t)}%" aria-label={count>1 ? `${count} moments at ${clock(clockAt(group.t))}` : `${momentTitle(event)} at ${clock(clockAt(group.t))}`}
        onpointerenter={() => { if (!drag && !menu) hover={time:group.t,event:count===1?event:undefined,label:count>1?`${count} moments`:undefined}; }} onpointerleave={clearHover}
        onfocus={() => hover={time:group.t,event:count===1?event:undefined}} onblur={clearHover}
        oncontextmenu={e => context(e, count === 1 ? event : undefined)}
        onclick={() => { hover=null; if(count===1)onevent(event);else popup=group.events; }}><span>{count > 1 ? count : ''}</span></button>
    {/each}
    {#if selection}
      {@const mid = (pct(selection.a) + pct(selection.b)) / 2}
      {#if selection.b > view.a && selection.a < view.b}
        <button class="clip-length" class:dragging={drag?.mode==='move'} style:left="clamp(38px, {mid}%, calc(100% - 38px))" disabled={!editable}
          aria-label="Clip length {lengthText(selection.b-selection.a)}. Drag to move the clip." title="Drag to move the whole clip"
          onpointerdown={event => start(event, 'move')}>{lengthText(selection.b-selection.a)}</button>
      {/if}
      {#each (['a','b'] as const) as edge}
        {#if selection[edge] >= view.a-.000001 && selection[edge] <= view.b+.000001}
          <button class="handle" class:start={edge==='a'} class:end={edge==='b'} class:dragging={drag?.mode===edge}
            style:left="{pct(selection[edge])}%" disabled={!editable} aria-label={edge==='a'?'Clip start handle':'Clip end handle'}
            title={`${edge==='a'?'Clip start':'Clip end'} ${clock(clockAt(selection[edge]),3)} · drag, or use the arrow keys to step ${edge==='a'?'keyframes':'frames'}`}
            onpointerdown={event => start(event, edge)} onkeydown={event => handleKey(event, edge)}></button>
        {/if}
      {/each}
    {/if}
    {#if zoomed}
      <div class="scroll" role="presentation" onpointerdown={barDown}>
        <span class:dragging={drag?.mode==='bar'} style:left="{(view.a-first)/total*100}%" style:width="max(24px, {span/total*100}%)"></span>
        {#if position < view.a || position > view.b}<i style:left="{(position-first)/total*100}%"></i>{/if}
      </div>
    {/if}
    {#if hover && !popup.length && !menu}
      <div class="hover-card" class:snapped={!!snapped} style:left="clamp(112px, {pct(hover.time)}%, calc(100% - 112px))" role="tooltip">
        {#if hover.event?.kind==='screenshot' && hover.event.path}
          <img src={convertFileSrc(hover.event.path)} alt="Screenshot preview" />
        {:else if hoverFootage}<div class="frame">{#if thumb}<img src={thumb.image} alt="Video frame preview" />{/if}</div>{/if}
        {#if hover.event}<strong class:shot={hover.event.kind==='screenshot'}>{momentTitle(hover.event)}</strong>{:else if hover.label}<strong class="plain">{hover.label}</strong>{/if}
        <span class="when"><b>{clock(clockAt(hover.time), drag && drag.mode !== 'scrub' ? 3 : perPixel < 0.05 ? 3 : perPixel < 1.5)}</b><small>{hoverFootage ? timeOfDay(hover.time) : 'Not recorded'}</small></span>
        {#if snapped}<span class="snap-note">Snapped to {snapped.what === 'Playhead' ? 'the playhead' : snapped.what} · hold Shift to move freely</span>{/if}
      </div>
    {/if}
    {#if popup.length}
      <div class="cluster" role="region" aria-label="Grouped moments">
        <div class="row"><strong class="grow">{popup.length} moments here</strong><button class="btn ghost sm" onclick={() => popup=[]}>Close</button></div>
        {#each popup as event (event.id)}
          <button class="cluster-event" onclick={() => { onevent(event); popup=[]; }}>
            <span class="pip" class:shot={event.kind==='screenshot'}></span><span class="grow">{momentTitle(event)}</span><time>{clock(clockAt(event.t),true)}</time>
          </button>
        {/each}
      </div>
    {/if}
    {#if menu}
      {@const footage = !!containing(intervals, menu.time)}
      <div class="context" role="menu" style:left="clamp(0px, {menu.x}px, calc(100% - 208px))" style:bottom="calc(100% - {menu.y}px + 6px)">
        <p>{menu.event ? momentTitle(menu.event) : clock(clockAt(menu.time), true)}{#if menu.event}<small>{clock(clockAt(menu.time), true)}</small>{/if}</p>
        {#if menu.event}<button role="menuitem" onclick={() => act('goto')}><Icon name="play" size={14}/>Go to this moment</button>{/if}
        <button role="menuitem" disabled={!footage || !editable} onclick={() => act('clip')}><Icon name="scissors" size={15}/>Clip around {menu.event ? 'it' : 'here'}</button>
        {#if selection && !menu.event}
          <button role="menuitem" disabled={!footage || !editable || menu.time >= selection.b - MIN_CLIP} onclick={() => act('start')}><span class="glyph">[</span>Start the clip here</button>
          <button role="menuitem" disabled={!footage || !editable || menu.time <= selection.a + MIN_CLIP} onclick={() => act('end')}><span class="glyph">]</span>End the clip here</button>
        {/if}
        {#if menu.event?.kind === 'marker'}
          <hr/>
          <button role="menuitem" onclick={() => act('rename')}><Icon name="pencil" size={15}/>Rename marker</button>
          <button role="menuitem" class="danger" onclick={() => act('delete')}><Icon name="trash" size={15}/>Delete marker</button>
        {/if}
        {#if zoomed && !menu.event}<hr/><button role="menuitem" onclick={() => { menu = null; fit(); }}><Icon name="fullscreen" size={15}/>Show the whole recording</button>{/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .timeline { position: relative; flex: none; padding: 0 18px; }
  .seek-strip { position: relative; height: 82px; margin-top: 2px; }
  .detail { position: absolute; inset: 0 0 12px; touch-action: none; cursor: pointer; user-select: none; outline: none; }
  .dragging .detail { cursor: grabbing; }
  .detail:focus-visible .footage-lane { box-shadow: 0 0 0 2px var(--accent-soft), 0 0 0 1px var(--accent); }

  /* The lane: recorded footage as solid blocks; the part already watched is brighter. */
  .footage-lane { position: absolute; inset: 36px 0 auto; height: 10px; overflow: hidden; border-radius: 3px; background: color-mix(in srgb, var(--text) 4%, transparent);
    transition: box-shadow var(--t-med) var(--ease); }
  .footage, .played, .dim, .clip-range, .saved-clip { position: absolute; top: 0; height: 100%; }
  .footage { background: color-mix(in srgb, var(--accent) 34%, var(--surface-3)); min-width: 1px; border-radius: 2px; }
  .played { background: color-mix(in srgb, var(--accent) 78%, var(--surface)); border-radius: 2px; }
  .dim { background: #080b12b3; }
  .clip-range { background: color-mix(in srgb, var(--accent) 30%, transparent); box-shadow: inset 0 1px 0 var(--accent), inset 0 -1px 0 var(--accent); }
  .saved-clip { top: auto; bottom: 0; height: 2px; min-width: 3px; background: var(--ok); pointer-events: none; }

  .session-label { position: absolute; top: 0; padding-left: 1px; font-size: 10.5px; line-height: 13px; color: var(--text-3); white-space: nowrap; pointer-events: none; letter-spacing: .01em; }
  .tick { position: absolute; top: 49px; height: 3px; width: 1px; background: var(--line-strong); pointer-events: none; }
  .tick.major { height: 7px; background: var(--text-3); }
  .tick i { position: absolute; top: 10px; left: 0; transform: translateX(-50%); font-style: normal; font-size: 10px; line-height: 1; color: var(--text-3);
    font-variant-numeric: tabular-nums; white-space: nowrap; }

  /* Three vertical lines, told apart at a glance: faint (cursor), blue (snap), white with a knob (playhead). */
  .ghost, .snap-line, .playhead { position: absolute; pointer-events: none; transform: translateX(-50%); }
  .ghost { top: 32px; height: 24px; width: 1px; background: color-mix(in srgb, var(--text) 55%, transparent); }
  .snap-line { top: 14px; height: 42px; width: 2px; border-radius: 1px; background: var(--accent-hover); box-shadow: 0 0 10px var(--accent); z-index: 3; animation: snap 140ms var(--ease); }
  @keyframes snap { from { transform: translateX(-50%) scaleY(.5); opacity: 0; } }
  .playhead { top: 30px; height: 26px; width: 2px; border-radius: 1px; background: #fff; box-shadow: 0 0 0 1px #0006; z-index: 2; }
  .playhead::before { content: ''; position: absolute; top: -5px; left: -4px; width: 10px; height: 10px; border-radius: 50%; background: #fff; box-shadow: 0 1px 4px #0009; }

  .moment, .handle, .clip-length { position: absolute; border: 0; padding: 0; background: transparent; touch-action: none; font: inherit; }
  /* Markers: a pin above the lane with a stem through it. Amber = marker, blue = screenshot. */
  .moment { top: 12px; width: 24px; height: 24px; transform: translateX(-50%); cursor: pointer; color: var(--warn); z-index: 1; }
  .moment::before { content: ''; position: absolute; left: 11px; top: 18px; width: 2px; height: 16px; border-radius: 1px; background: currentColor; opacity: .55; pointer-events: none; }
  .moment span { position: absolute; left: 6px; top: 6px; width: 12px; height: 12px; border-radius: 50% 50% 50% 2px; transform: rotate(-45deg); background: currentColor;
    box-shadow: 0 1px 3px #0008; transition: transform var(--t-med) var(--ease); }
  .moment.shot { color: var(--accent); }
  .moment.shot span { border-radius: 3px; transform: none; }
  .moment.shot::before { height: 6px; }
  .moment.grouped span { left: 3px; top: 4px; width: 18px; height: 15px; border-radius: 8px; transform: none; display: grid; place-items: center;
    color: #14171c; font-size: 10px; font-weight: 700; line-height: 1; background: var(--warn); }
  .moment.grouped.shot span { background: var(--accent); }
  .moment:hover span, .moment:focus-visible span { transform: rotate(-45deg) scale(1.3); }
  .moment.shot:hover span, .moment.grouped:hover span, .moment.shot:focus-visible span, .moment.grouped:focus-visible span { transform: scale(1.2); }
  .moment:hover::before, .moment.focused::before { opacity: 1; }
  .moment.focused span { box-shadow: 0 0 0 2px var(--surface), 0 0 0 4px currentColor; }
  .moment:focus-visible { outline: none; }

  /* Clip handles sit just outside the range they bound, so they never cover the frames they pick. */
  .handle { top: 28px; width: 20px; height: 26px; cursor: ew-resize; z-index: 4; }
  .handle.start { transform: translateX(-100%); }
  .handle::after { content: ''; position: absolute; top: 4px; bottom: 4px; width: 11px; background: var(--accent);
    background-image: linear-gradient(#fff, #fff); background-size: 2px 8px; background-position: center; background-repeat: no-repeat;
    box-shadow: 0 1px 4px #0007; transition: transform var(--t-med) var(--ease), background-color var(--t-fast) var(--ease); }
  .handle.start::after { right: 0; border-radius: 6px 0 0 6px; transform-origin: right center; }
  .handle.end::after { left: 0; border-radius: 0 6px 6px 0; transform-origin: left center; }
  .handle:hover::after, .handle.dragging::after, .handle:focus-visible::after { background-color: var(--accent-hover); transform: scale(1.18, 1.1); }
  .handle:focus-visible { outline: none; }
  .handle:disabled, .clip-length:disabled { opacity: .5; cursor: default; }
  .clip-length { top: 11px; height: 20px; padding: 0 9px; transform: translateX(-50%); z-index: 3; border-radius: 10px; cursor: grab; white-space: nowrap;
    background: var(--accent); color: var(--text-on-accent); font-size: 11px; font-weight: 650; font-variant-numeric: tabular-nums; box-shadow: 0 2px 8px #0006;
    transition: transform var(--t-med) var(--ease), background var(--t-fast) var(--ease); }
  .clip-length:hover { background: var(--accent-hover); transform: translateX(-50%) scale(1.06); }
  .clip-length.dragging { cursor: grabbing; transform: translateX(-50%) scale(1.06); }

  /* Shown only while zoomed in: where the view is in the whole recording, and the playhead if it is off screen. */
  .scroll { position: absolute; left: 0; right: 0; bottom: 0; height: 12px; cursor: pointer; touch-action: none; animation: fade 160ms var(--ease); }
  .scroll::before { content: ''; position: absolute; inset: 4px 0; border-radius: 2px; background: color-mix(in srgb, var(--text) 8%, transparent); }
  .scroll span { position: absolute; top: 4px; height: 4px; border-radius: 2px; background: var(--text-3); cursor: grab; transition: top var(--t-fast) var(--ease), height var(--t-fast) var(--ease), background var(--t-fast) var(--ease); }
  .scroll:hover span, .scroll span.dragging { top: 3px; height: 6px; border-radius: 3px; background: var(--text-2); }
  .scroll i { position: absolute; top: 2px; width: 2px; height: 8px; margin-left: -1px; border-radius: 1px; background: #fff; pointer-events: none; }
  @keyframes fade { from { opacity: 0; } }

  .hover-card, .cluster, .context { position: absolute; z-index: 8; border-radius: var(--r-md); border: 1px solid var(--line-strong);
    background: var(--surface-2); box-shadow: var(--shadow-pop); }
  .hover-card, .cluster { bottom: calc(100% - 6px); }
  .hover-card { display: grid; gap: 5px; width: max-content; max-width: 224px; padding: 6px 6px 7px; transform: translateX(-50%); pointer-events: none; font-size: 12px; font-variant-numeric: tabular-nums;
    animation: rise 110ms var(--ease); transition: border-color var(--t-fast) var(--ease); }
  @keyframes rise { from { opacity: 0; transform: translate(-50%, 4px); } }
  .hover-card.snapped { border-color: var(--accent); }
  .hover-card img { width: 210px; max-height: 118px; object-fit: contain; background: #000; border-radius: 6px; display: block; }
  .hover-card .frame { width: 210px; aspect-ratio: 16/9; background: #05070a; border-radius: 6px; overflow: hidden; display: grid; place-items: center; }
  .hover-card .frame img { width: 100%; height: 100%; max-height: none; }
  .when { display: flex; justify-content: space-between; align-items: baseline; gap: 10px; padding: 0 3px; }
  .when b { font-weight: 650; } .when small { color: var(--text-2); font-size: 11px; }
  .hover-card strong { padding: 0 3px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--warn); font-weight: 600; }
  .hover-card strong.shot { color: var(--accent); } .hover-card strong.plain { color: var(--text); }
  .snap-note { padding: 0 3px; max-width: 210px; color: var(--accent-hover); font-size: 11px; line-height: 1.3; }
  .cluster { right: 0; width: min(360px,100%); max-height: 240px; overflow: auto; padding: 8px; }
  .cluster .row { padding: 2px 2px 4px 8px; }
  .cluster-event, .context button { display: flex; align-items: center; gap: 9px; width: 100%; padding: 7px 9px; background: transparent; border: 0; text-align: left; cursor: pointer; border-radius: 6px; color: var(--text); font: inherit; font-size: 13px; }
  .cluster-event:hover, .context button:hover:not(:disabled) { background: color-mix(in srgb, var(--text) 9%, transparent); }
  .cluster-event time { font-variant-numeric: tabular-nums; color: var(--text-2); font-size: 12px; }
  .pip { width: 8px; height: 8px; border-radius: 50%; background: var(--warn); flex: none; } .pip.shot { border-radius: 2px; background: var(--accent); }
  .context { width: 208px; padding: 5px; animation: pop 120ms var(--ease); transform-origin: bottom left; }
  @keyframes pop { from { opacity: 0; transform: scale(.96); } }
  .context p { display: flex; justify-content: space-between; gap: 8px; margin: 0; padding: 4px 9px 6px; font-size: 11px; font-weight: 600; color: var(--text-2); font-variant-numeric: tabular-nums; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .context p small { font-weight: 400; color: var(--text-3); }
  .context button:disabled { opacity: .4; cursor: default; }
  .context .danger { color: var(--danger); }
  .context hr { border: 0; border-top: 1px solid color-mix(in srgb, var(--text) 9%, transparent); margin: 4px 0; }
  .glyph { width: 15px; text-align: center; font-weight: 700; color: var(--accent); }
  @media (prefers-reduced-motion: reduce) { .hover-card, .context, .snap-line, .scroll { animation: none; } .moment span, .handle::after, .clip-length { transition: none; } }
</style>
