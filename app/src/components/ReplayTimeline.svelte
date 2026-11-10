<script lang="ts">
  import { onMount } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import type { TimelineEvent } from '../lib/types';
  import { clamp, clock, fitView, zoomView, tickStep, eventGroups, type Range } from '../lib/timeline';
  let { first, last, intervals, events, position, selection = $bindable(null), view = $bindable(), onseek, onevent, onscrub }: {
    first: number; last: number; intervals: [number, number][]; events: TimelineEvent[]; position: number;
    selection: Range | null; view: Range; onseek: (t: number, final: boolean) => void;
    onevent: (e: TimelineEvent) => void; onscrub: () => void;
  } = $props();
  let width = $state(800);
  let detail: HTMLDivElement;
  let drag = $state<{ mode: string; x: number; t: number; view: Range; selection: Range | null; pointer: number } | null>(null);
  let popup = $state<TimelineEvent[]>([]);
  let hover = $state<TimelineEvent | null>(null);
  let hoverX = $state(0);
  let snapped = $state(false);
  let edgeFrame = 0;
  let edgePointer: {x: number; shift: boolean} | null = null;
  const span = $derived(Math.max(.001, view.b - view.a));
  const pct = (t: number) => clamp((t - view.a) / span * 100, 0, 100);
  const step = $derived(tickStep(span, width));
  const ticks = $derived.by(() => {
    const result: number[] = [];
    for (let t = Math.ceil(view.a / step) * step; t <= view.b && result.length < 80; t += step) result.push(t);
    return result;
  });
  const groups = $derived(eventGroups(events, view, width));
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
    return () => { stopDrag(); observer.disconnect(); detail.removeEventListener('wheel', wheel); };
  });
  function timeAt(x: number) {
    const box = detail.getBoundingClientRect();
    return view.a + clamp((x - box.left) / box.width, 0, 1) * span;
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
    if (e.button === 1 || e.altKey) mode = 'pan';
    drag = { mode, x: e.clientX, t: timeAt(e.clientX), view: { ...view }, selection: selection && { ...selection }, pointer: e.pointerId };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    if (mode === 'scrub') { onscrub(); onseek(timeAt(e.clientX), false); }
  }
  function move(e: PointerEvent) {
    if (!drag || drag.pointer !== e.pointerId) return;
    if (e.buttons === 0) { stopDrag(); return; }
    const d = drag;
    if (d.mode === 'pan') view = fitView(d.view.a - (e.clientX - d.x) / width * (d.view.b - d.view.a), span, first, last);
    else if (d.mode === 'scrub') onseek(timeAt(e.clientX), false);
    else if (selection && d.selection) {
      const t = e.shiftKey ? timeAt(e.clientX) : snap(timeAt(e.clientX), d.mode === 'a' ? selection.b : selection.a);
      if (d.mode === 'a' || d.mode === 'b') {
        resize(d.mode,t);
        edgePointer={x:e.clientX,shift:e.shiftKey};
        if(!edgeFrame)edgeFrame=requestAnimationFrame(followHandle);
      }
      else {
        const duration = d.selection.b - d.selection.a;
        const a = clamp(d.selection.a + t - d.t, first, last - duration);
        selection = { a, b: a + duration };
      }
    }
  }
  function resize(edge:'a'|'b',t:number) {
    if(!selection)return;
    selection=edge==='a'?{a:clamp(t,first,selection.b-.1),b:selection.b}:{a:selection.a,b:clamp(t,selection.a+.1,last)};
  }
  function followHandle() {
    edgeFrame=0;
    if(!edgePointer || !drag || (drag.mode!=='a' && drag.mode!=='b'))return;
    const box=detail.getBoundingClientRect();
    const distance=edgePointer.x<box.left+12 ? edgePointer.x-(box.left+12) : edgePointer.x>box.right-12 ? edgePointer.x-(box.right-12) : 0;
    if(!distance)return;
    const old=view.a;
    pan(clamp(distance/40,-1,1)*span*.018);
    if(old===view.a)return;
    const t=timeAt(edgePointer.x);
    resize(drag.mode,edgePointer.shift?t:clamp(snap(t),view.a,view.b));
    edgeFrame=requestAnimationFrame(followHandle);
  }
  function stopDrag() {
    drag=null;snapped=false;edgePointer=null;
    cancelAnimationFrame(edgeFrame);edgeFrame=0;
  }
  function finish(e: PointerEvent) {
    if (!drag) return;
    const mode = drag.mode; stopDrag();
    if ((e.currentTarget as HTMLElement).hasPointerCapture(e.pointerId)) (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    if (mode === 'scrub') onseek(timeAt(e.clientX), true);
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

<div class="timeline">
  <div class="detail" bind:this={detail} class:dragging={!!drag} class:snapped role="slider" tabindex="0" aria-label="Recording timeline" aria-valuemin={view.a} aria-valuemax={view.b} aria-valuenow={clamp(position,view.a,view.b)} aria-valuetext={clock(position,true)} onkeydown={key}
    onpointerdown={e => start(e, 'scrub')} onpointermove={move} onpointerup={finish} onpointercancel={stopDrag} onlostpointercapture={stopDrag}>
    <div class="ruler">{#each ticks as t}<span style:left="{pct(t)}%">{clock(t, step < 1)}</span>{/each}</div>
    <div class="footage-lane">
      {#each intervals as [a,b]}{#if b > view.a && a < view.b}<div class="footage" style:left="{pct(a)}%" style:width="{pct(b)-pct(a)}%"></div>{/if}{/each}
    </div>
    {#if selection && selection.b >= view.a && selection.a <= view.b}
      <div class="clip-range" style:left="{pct(selection.a)}%" style:width="{pct(selection.b)-pct(selection.a)}%" onpointerdown={e => start(e,'selection')} role="presentation"></div>
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
    <span class="view-label">{span < last-first-.01 ? clock(view.a) + " – " + clock(view.b) : "Full recording"}</span>
    <div class="zoom-controls" aria-label="Timeline zoom and pan">
      {#if span < last-first-.01}
        <button class="btn ghost sm icon" aria-label="Pan earlier" title="Pan earlier" onclick={() => pan(-span*.65)}>‹</button>
        <button class="btn ghost sm icon" aria-label="Pan later" title="Pan later" onclick={() => pan(span*.65)}>›</button>
        <button class="btn ghost sm" onclick={() => view={a:first,b:last}}>Fit</button>
      {/if}
      <button class="btn ghost sm icon" aria-label="Zoom out" title="Zoom out" disabled={span>=last-first-.01} onclick={() => zoom(2)}>−</button>
      <button class="btn ghost sm icon" aria-label="Zoom in" title="Zoom in · or scroll over the timeline" disabled={span<=2.001} onclick={() => zoom(.5)}>+</button>
    </div>
  </div>
</div>

<style>
  .timeline{position:relative;min-width:0;flex:none;padding:12px 14px 3px;border:1px solid var(--line);border-radius:10px;background:var(--surface)}
  .detail{height:90px;position:relative;touch-action:none;cursor:crosshair;user-select:none}.ruler{position:absolute;inset:0 0 auto;height:25px;overflow:hidden}.ruler span{position:absolute;font-size:12px;color:var(--text-2);transform:translateX(-50%);padding-bottom:6px;border-bottom:1px solid var(--line-strong);font-variant-numeric:tabular-nums;white-space:nowrap}.ruler span:first-child{transform:none}.ruler span:last-child{transform:translateX(-100%)}.footage-lane{position:absolute;top:27px;height:38px;left:0;right:0;border-radius:5px;overflow:hidden;background:repeating-linear-gradient(135deg,var(--bg) 0 5px,var(--surface-2) 5px 6px)}.footage{position:absolute;height:100%;background:color-mix(in srgb,var(--accent) 21%,var(--surface-3));border-inline:1px solid var(--line-strong)}.clip-range{position:absolute;top:27px;height:38px;background:var(--accent-soft);border-block:2px solid var(--accent);cursor:grab;display:grid;place-items:center;overflow:hidden}.clip-range span{font-size:11px;color:var(--accent);font-variant-numeric:tabular-nums;white-space:nowrap}.handle{position:absolute;top:24px;height:44px;width:14px;border:0;background:var(--accent);border-radius:4px;padding:0;cursor:ew-resize;z-index:3;transform:translateX(-50%);touch-action:none}.handle::after{content:'';display:block;width:2px;height:16px;background:var(--text-on-accent);margin:auto;border-radius:2px}.snapped .handle{background:var(--accent-hover)}.event-lane{position:absolute;inset:65px 0 0;pointer-events:none}.moment{position:absolute;transform:translateX(-50%);min-width:25px;height:25px;border:1px solid transparent;background:var(--warn-soft);color:var(--warn);border-radius:5px;font-size:11px;font-weight:600;padding:0 4px;cursor:pointer;pointer-events:auto}.moment:hover,.moment:focus-visible{border-color:currentColor;z-index:5}.moment.shot{color:var(--accent);background:var(--accent-soft)}.playhead{position:absolute;top:22px;bottom:32px;width:2px;background:var(--text);pointer-events:none;z-index:2}.playhead span{position:absolute;top:-2px;left:-4px;width:10px;height:8px;clip-path:polygon(0 0,100% 0,100% 50%,50% 100%,0 50%);background:var(--text)}
  .timeline-footer,.zoom-controls{display:flex;align-items:center;gap:4px}.timeline-footer{justify-content:space-between;min-height:30px}.view-label{font-size:12px;color:var(--text-2);font-variant-numeric:tabular-nums}.hover-card{position:absolute;bottom:120px;width:220px;transform:translateX(-50%);padding:8px;background:var(--surface-2);border:1px solid var(--line-strong);border-radius:8px;box-shadow:var(--shadow-pop);z-index:8;pointer-events:none;display:flex;flex-direction:column;font-size:12px;gap:4px}.hover-card img{width:100%;max-height:124px;object-fit:contain;background:#000;border-radius:4px}.hover-card strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.hover-card span{color:var(--text-2)}.cluster{position:absolute;bottom:45px;right:18px;width:min(420px,calc(100% - 36px));max-height:240px;overflow:auto;background:var(--surface-2);border:1px solid var(--line-strong);border-radius:8px;padding:10px;z-index:9;box-shadow:var(--shadow-pop)}.cluster-event{display:flex;align-items:center;gap:10px;width:100%;padding:10px 6px;background:transparent;border:0;text-align:left;cursor:pointer;border-radius:5px;color:var(--text)}.cluster-event:hover{background:var(--surface-3)}.cluster-event span:first-child{flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
</style>
