<script lang="ts">
  import { onMount } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import type { TimelineEvent } from '../lib/types';
  import { clamp, clock, fitView, zoomView, tickStep, eventGroups, type Range } from '../lib/timeline';
  let { first, last, intervals, events, position, selection = $bindable(null), view = $bindable(), onseek, onevent, onscrub, onrange }: {
    first: number; last: number; intervals: [number, number][]; events: TimelineEvent[]; position: number;
    selection: Range | null; view: Range; onseek: (t: number, final: boolean) => void;
    onevent: (e: TimelineEvent) => void; onscrub: () => void; onrange?: (t: number) => void;
  } = $props();
  let width = $state(800);
  let host: HTMLDivElement;
  let detail: HTMLDivElement;
  let overview: HTMLDivElement;
  let drag = $state<{ mode: string; x: number; t: number; view: Range; selection: Range | null; pointer: number } | null>(null);
  let popup = $state<TimelineEvent[]>([]);
  let hover = $state<TimelineEvent | null>(null);
  let hoverX = $state(0);
  let snapped = $state(false);
  const span = $derived(Math.max(.001, view.b - view.a));
  const pct = (t: number) => clamp((t - view.a) / span * 100, 0, 100);
  const fullPct = (t: number) => clamp((t - first) / (last - first) * 100, 0, 100);
  const step = $derived(tickStep(span, width));
  const ticks = $derived.by(() => {
    const result: number[] = [];
    for (let t = Math.ceil(view.a / step) * step; t <= view.b && result.length < 80; t += step) result.push(t);
    return result;
  });
  const groups = $derived(eventGroups(events, view, width));
  const overviewGroups = $derived(eventGroups(events, { a: first, b: last }, width));
  onMount(() => {
    const observer = new ResizeObserver(entries => width = entries[0].contentRect.width);
    observer.observe(detail);
    // Native listener is explicitly non-passive so trackpad zoom never scrolls the page.
    const wheel = (e: WheelEvent) => {
      e.preventDefault(); popup = [];
      if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) pan((e.deltaX || e.deltaY) / width * span);
      else zoom(Math.exp(clamp(e.deltaY, -100, 100) * .008), timeAt(e.clientX));
    };
    detail.addEventListener('wheel', wheel, { passive: false });
    return () => { observer.disconnect(); detail.removeEventListener('wheel', wheel); };
  });
  function timeAt(x: number, full = false) {
    const box = (full ? overview : detail).getBoundingClientRect();
    return full ? first + clamp((x - box.left) / box.width, 0, 1) * (last - first)
      : view.a + clamp((x - box.left) / box.width, 0, 1) * span;
  }
  function zoom(factor: number, at = position >= view.a && position <= view.b ? position : (view.a + view.b) / 2) {
    view = zoomView(view, factor, at, first, last);
  }
  function pan(delta: number) { view = fitView(view.a + delta, span, first, last); }
  function snap(t: number, excluded?: number) {
    const candidates = [position, ...intervals.flat(), ...events.map(e => e.t)].filter(n => n !== excluded);
    const nearest = candidates.reduce((a, b) => Math.abs(b - t) < Math.abs(a - t) ? b : a, Infinity);
    snapped = Math.abs(nearest - t) <= span * 8 / width;
    return snapped ? nearest : t;
  }
  function start(e: PointerEvent, mode: string) {
    if (e.button !== 0 && e.button !== 1) return;
    e.preventDefault(); e.stopPropagation(); popup = []; hover = null;
    const target = e.currentTarget as HTMLElement;
    if(target.matches('button,[tabindex]'))target.focus({preventScroll:true});
    if (mode === 'window' && span >= last-first-.01) mode = 'overview';
    if (e.button === 1 || e.altKey) mode = 'pan';
    const full = mode === 'overview' || mode === 'window';
    drag = { mode, x: e.clientX, t: timeAt(e.clientX, full), view: { ...view }, selection: selection && { ...selection }, pointer: e.pointerId };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    if (mode === 'scrub') { onscrub(); onseek(timeAt(e.clientX), false); }
    if (mode === 'overview') {
      onscrub(); const t = timeAt(e.clientX, true);
      view = fitView(t - span / 2, span, first, last); onseek(t, false);
    }
  }
  function move(e: PointerEvent) {
    if (!drag || drag.pointer !== e.pointerId) return;
    if (e.buttons === 0) { drag = null; snapped = false; return; }
    const d = drag;
    if (d.mode === 'pan') view = fitView(d.view.a - (e.clientX - d.x) / width * (d.view.b - d.view.a), span, first, last);
    else if (d.mode === 'window') view = fitView(d.view.a + (e.clientX - d.x) / width * (last - first), span, first, last);
    else if (d.mode === 'overview') {
      const t = timeAt(e.clientX, true); view = fitView(t - span / 2, span, first, last); onseek(t, false);
    } else if (d.mode === 'scrub') onseek(timeAt(e.clientX), false);
    else if (selection && d.selection) {
      const t = e.shiftKey ? timeAt(e.clientX) : snap(timeAt(e.clientX), d.mode === 'a' ? selection.b : selection.a);
      if (d.mode === 'a') selection = { a: clamp(t, first, selection.b - .1), b: selection.b };
      else if (d.mode === 'b') selection = { a: selection.a, b: clamp(t, selection.a + .1, last) };
      else {
        const duration = d.selection.b - d.selection.a;
        const a = clamp(d.selection.a + t - d.t, first, last - duration);
        selection = { a, b: a + duration };
      }
    }
  }
  function finish(e: PointerEvent) {
    if (!drag) return;
    const mode = drag.mode, clicked = Math.abs(e.clientX-drag.x)<3; drag = null; snapped = false;
    if ((e.currentTarget as HTMLElement).hasPointerCapture(e.pointerId)) (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    if (mode === 'scrub' || mode === 'overview') onseek(timeAt(e.clientX, mode === 'overview'), true);
    if (mode === 'window' && clicked) { onscrub(); onseek(timeAt(e.clientX,true),true); }
  }
  function key(e: KeyboardEvent) {
    if (!['ArrowLeft','ArrowRight','Home','End','+','=','-','0','Escape'].includes(e.key)) return;
    e.preventDefault(); e.stopPropagation();
    if (e.key === 'Escape') { popup = []; hover = null; return; }
    if (e.key === '+' || e.key === '=') zoom(.5);
    else if (e.key === '-') zoom(2);
    else if (e.key === '0') view = { a: first, b: last };
    else if (e.key === 'Home') onseek(first, true);
    else if (e.key === 'End') onseek(last - .05, true);
    else if (e.shiftKey) pan((e.key === 'ArrowRight' ? 1 : -1) * span * .2);
    else onseek(position + (e.key === 'ArrowRight' ? 1 : -1) * (e.altKey ? .1 : 5), true);
  }
  function handleKey(e: KeyboardEvent, edge: 'a' | 'b') {
    if (!selection || !['ArrowLeft','ArrowRight'].includes(e.key)) return;
    e.preventDefault(); e.stopPropagation();
    const delta = (e.key === 'ArrowRight' ? 1 : -1) * (e.shiftKey ? 1 : .1);
    selection = edge === 'a' ? { a: clamp(selection.a + delta, first, selection.b - .1), b: selection.b }
      : { a: selection.a, b: clamp(selection.b + delta, selection.a + .1, last) };
  }
</script>

<div class="timeline" bind:this={host}>
  <div class="timeline-heading">
    <span class="small muted">Session overview <span class="legend">▰ Footage · ◆ Markers · ▣ Screenshots</span></span>
    <button class="link small" onclick={() => view = { a: first, b: last }}>Full history</button>
  </div>
  <div class="overview" bind:this={overview} role="slider" tabindex="0" aria-label="Full session history" aria-valuemin={first} aria-valuemax={last} aria-valuenow={position} aria-valuetext={clock(position)} onkeydown={key}
    onpointerdown={e => start(e, 'overview')} onpointermove={move} onpointerup={finish} onpointercancel={() => drag = null} onlostpointercapture={() => drag = null}>
    {#each intervals as [a,b]}<div class="overview-footage" style:left="{fullPct(a)}%" style:width="{fullPct(b)-fullPct(a)}%"></div>{/each}
    {#each overviewGroups as group}<span class="overview-event" style:left="{fullPct(group.t)}%" class:shot={group.events.every(e => e.kind === 'screenshot')}></span>{/each}
    <div class="viewport" style:left="{fullPct(view.a)}%" style:width="{fullPct(view.b)-fullPct(view.a)}%" onpointerdown={e => start(e, 'window')} role="presentation"></div>
    <span class="overview-head" style:left="{fullPct(position)}%"></span>
  </div>
  <div class="overview-labels"><span>{clock(first)}</span><span>{clock(last)}</span></div>
  <div class="detail" bind:this={detail} class:dragging={!!drag} class:snapped role="slider" tabindex="0" aria-label="Recording timeline" aria-valuemin={view.a} aria-valuemax={view.b} aria-valuenow={clamp(position,view.a,view.b)} aria-valuetext={clock(position,true)} onkeydown={key}
    onpointerdown={e => start(e, 'scrub')} onpointermove={move} onpointerup={finish} onpointercancel={() => { drag = null; snapped = false; }} onlostpointercapture={() => { drag = null; snapped = false; }}>
    <div class="ruler">{#each ticks as t}<span style:left="{pct(t)}%">{clock(t, step < 1)}</span>{/each}</div>
    <div class="footage-lane">
      {#each intervals as [a,b]}{#if b > view.a && a < view.b}<div class="footage" style:left="{pct(a)}%" style:width="{pct(b)-pct(a)}%"></div>{/if}{/each}
    </div>
    {#if selection && selection.b >= view.a && selection.a <= view.b}
      <div class="clip-range" style:left="{pct(selection.a)}%" style:width="{pct(selection.b)-pct(selection.a)}%" onpointerdown={e => start(e,'selection')} role="presentation"><span>{(selection.b-selection.a).toFixed(1)}s</span></div>
      {#if selection.a >= view.a && selection.a <= view.b}<button class="handle start" aria-label="Clip start handle" title="Drag clip start · Arrow keys adjust 0.1s" style:left="{pct(selection.a)}%" onpointerdown={e => start(e,'a')} onkeydown={e => handleKey(e,'a')}></button>{/if}
      {#if selection.b >= view.a && selection.b <= view.b}<button class="handle end" aria-label="Clip end handle" title="Drag clip end · Arrow keys adjust 0.1s" style:left="{pct(selection.b)}%" onpointerdown={e => start(e,'b')} onkeydown={e => handleKey(e,'b')}></button>{/if}
    {/if}
    <div class="event-lane">
      {#each groups as group}
        <button class="moment" class:shot={group.events.every(e => e.kind === 'screenshot')} style:left="{clamp(pct(group.t), 1.5, 98.5)}%" aria-label={group.events.length > 1 ? `${group.events.length} moments at ${clock(group.t)}` : `${group.events[0].label || group.events[0].kind} at ${clock(group.t)}`} title={group.events.length > 1 ? `${group.events.length} moments · click to browse` : `${group.events[0].label || group.events[0].kind} · ${clock(group.t)}`}
          onpointerdown={e => e.stopPropagation()} onpointerenter={() => { hover=group.events.length===1?group.events[0]:null; hoverX=clamp(pct(group.t),18,82); }} onpointerleave={() => hover=null} onfocus={() => { hover=group.events.length===1?group.events[0]:null; hoverX=clamp(pct(group.t),18,82); }} onblur={() => hover=null}
          onclick={e => { e.stopPropagation(); hover=null; if(group.events.length===1) onevent(group.events[0]); else popup=group.events; }}>
          {group.events.length>1 ? group.events.length : group.events[0].kind==='screenshot' ? '▣' : '◆'}
        </button>
      {/each}
    </div>
    {#if position >= view.a && position <= view.b}<div class="playhead" style:left="{pct(position)}%"><span></span></div>{/if}
  </div>
  {#if hover}<div class="hover-card" style:left="{hoverX}%" role="tooltip">{#if hover.kind==='screenshot' && hover.path}<img src={convertFileSrc(hover.path)} alt="Screenshot preview" onerror={e => e.currentTarget.style.display='none'} />{/if}<strong>{hover.label || hover.kind}</strong><span>{clock(hover.t,true)}{!intervals.some(([a,b])=>hover!.t>=a&&hover!.t<b)?' · Video expired':''}</span></div>{/if}
  {#if popup.length}<div class="cluster" role="region" aria-label="Grouped moments"><div class="row"><strong class="grow">{popup.length} nearby moments</strong><button class="btn ghost sm" onclick={() => popup=[]}>Close</button></div>{#each popup as ev}<button class="cluster-event" onclick={() => { onevent(ev); popup=[]; }}>{ev.kind==='screenshot'?'▣':'◆'} <span>{ev.label || ev.kind}</span><span class="mono">{clock(ev.t,true)}</span></button>{/each}</div>{/if}
  <div class="timeline-footer">
    <span class="hint">Scroll to zoom · Shift + scroll to pan <span class="extra">· Alt + drag to pan · Shift to bypass snapping</span></span>
    <div class="zoom-controls"><button class="btn sm icon" aria-label="Zoom out" onclick={() => zoom(2)}>−</button><span class="small mono">{span>=3600?`${(span/3600).toFixed(span%3600<1?0:1)}h`:span>=60?`${Math.round(span/60)}m`:`${span.toFixed(1)}s`}</span><button class="btn sm icon" aria-label="Zoom in" onclick={() => zoom(.5)}>+</button>{#if position<view.a || position>view.b}<button class="btn sm" onclick={() => view=fitView(position-span/2,span,first,last)}>Find playhead</button>{/if}</div>
  </div>
</div>

<style>
  .timeline{position:relative;min-width:0;flex:none;padding:10px 16px 8px;border:1px solid var(--line);border-radius:12px;background:var(--surface)}
  .timeline-heading,.overview-labels,.timeline-footer,.zoom-controls{display:flex;align-items:center;justify-content:space-between;gap:12px}.timeline-heading{margin-bottom:6px}.legend{font-size:11px;color:var(--text-3);margin-left:14px}.overview{height:24px;position:relative;border-radius:5px;background:repeating-linear-gradient(135deg,var(--bg) 0 4px,var(--surface-2) 4px 5px);touch-action:none;cursor:crosshair}.overview-footage{position:absolute;top:6px;bottom:6px;background:var(--line-strong);border-radius:2px}.viewport{position:absolute;inset-block:0;border:1px solid var(--accent);background:var(--accent-soft);min-width:4px;cursor:grab;border-radius:4px}.viewport:active{cursor:grabbing}.overview-event{position:absolute;top:4px;bottom:4px;width:2px;background:var(--warn);pointer-events:none}.overview-event.shot{background:var(--accent)}.overview-head{position:absolute;top:1px;bottom:1px;width:2px;background:var(--text);pointer-events:none}.overview-labels{font-variant-numeric:tabular-nums;color:var(--text-3);font-size:11px;margin:2px 0 6px}
  .detail{height:94px;position:relative;touch-action:none;cursor:crosshair;user-select:none}.ruler{position:absolute;inset:0 0 auto;height:25px;overflow:hidden}.ruler span{position:absolute;font-size:11px;color:var(--text-2);transform:translateX(-50%);padding-bottom:6px;border-bottom:1px solid var(--line-strong);font-variant-numeric:tabular-nums;white-space:nowrap}.ruler span:first-child{transform:none}.ruler span:last-child{transform:translateX(-100%)}.footage-lane{position:absolute;top:27px;height:38px;left:0;right:0;border-radius:5px;overflow:hidden;background:repeating-linear-gradient(135deg,var(--bg) 0 5px,var(--surface-2) 5px 6px)}.footage{position:absolute;height:100%;background:var(--surface-3);border-inline:1px solid var(--line-strong)}.clip-range{position:absolute;top:27px;height:38px;background:var(--accent-soft);border-block:2px solid var(--accent);cursor:grab;display:grid;place-items:center;overflow:hidden}.clip-range span{font-size:11px;color:var(--accent);font-variant-numeric:tabular-nums;white-space:nowrap}.handle{position:absolute;top:24px;height:44px;width:14px;border:0;background:var(--accent);border-radius:4px;padding:0;cursor:ew-resize;z-index:3;transform:translateX(-50%);touch-action:none}.handle::after{content:'';display:block;width:2px;height:16px;background:var(--text-on-accent);margin:auto;border-radius:2px}.snapped .handle{background:var(--accent-hover)}.event-lane{position:absolute;inset:72px 0 0;pointer-events:none}.moment{position:absolute;transform:translateX(-50%);min-width:24px;height:24px;border:1px solid transparent;background:var(--warn-soft);color:var(--warn);border-radius:5px;font-size:11px;font-weight:600;padding:0 4px;cursor:pointer;pointer-events:auto}.moment:hover,.moment:focus-visible{border-color:currentColor;z-index:5}.moment.shot{color:var(--accent);background:var(--accent-soft)}.playhead{position:absolute;top:22px;bottom:32px;width:2px;background:var(--text);pointer-events:none;z-index:2}.playhead span{position:absolute;top:-2px;left:-4px;width:10px;height:8px;clip-path:polygon(0 0,100% 0,100% 50%,50% 100%,0 50%);background:var(--text)}
  .timeline-footer{margin-top:2px}.hint{font-size:11px;color:var(--text-3)}.zoom-controls{gap:8px;flex:none}.zoom-controls>span{min-width:44px;text-align:center}.hover-card{position:absolute;bottom:120px;width:220px;transform:translateX(-50%);padding:8px;background:var(--surface-2);border:1px solid var(--line-strong);border-radius:8px;box-shadow:var(--shadow-pop);z-index:8;pointer-events:none;display:flex;flex-direction:column;font-size:12px;gap:4px}.hover-card img{width:100%;max-height:124px;object-fit:contain;background:#000;border-radius:4px}.hover-card strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.hover-card span{color:var(--text-2)}.cluster{position:absolute;bottom:45px;right:18px;width:min(420px,calc(100% - 36px));max-height:240px;overflow:auto;background:var(--surface-2);border:1px solid var(--line-strong);border-radius:8px;padding:10px;z-index:9;box-shadow:var(--shadow-pop)}.cluster-event{display:flex;align-items:center;gap:10px;width:100%;padding:10px 6px;background:transparent;border:0;text-align:left;cursor:pointer;border-radius:5px;color:var(--text)}.cluster-event:hover{background:var(--surface-3)}.cluster-event span:first-child{flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  @media(max-width:850px){.extra,.legend{display:none}.timeline{padding:10px 12px}.hint{max-width:190px}.overview-labels{margin-bottom:8px}}
</style>
