<script lang="ts">
  import { onMount } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { app, cancelPendingReview, engine, fmtDuration, historyFrames, historyReview, releaseReview, saveTrim, toast } from '../lib/store.svelte';
  import type { FrameTiming, HistorySession, TimelineEvent } from '../lib/types';
  import { clamp, clock, containing, fitView, selectionAround, validSelection, zoomView, type Range } from '../lib/timeline';
  import { buildLine, emptyLine, footage, toLine, toReal, wallClock, type GameLine } from '../lib/gameline';
  import ReplayTimeline from '../components/ReplayTimeline.svelte';
  import Icon from '../components/Icon.svelte';
  // `gameName`: the game whose recording history to open (otherwise the one being recorded, else the most recent).
  let { onclose, compact = false, gameName }: { onclose: () => void; compact?: boolean; gameName?: string } = $props();
  let video = $state<HTMLVideoElement>(null!);
  let stage = $state<HTMLDivElement>(null!);
  let timeline = $state<ReturnType<typeof ReplayTimeline>>();
  let zoomMenu = $state<HTMLDetailsElement>();
  /** The game on screen. Its whole recording history is one timeline. */
  let selected = $state('');
  let position = $state(0);
  let paused = $state(true);
  let muted = $state(false);
  const storedVolume = Number(globalThis.localStorage?.getItem('capture.volume') ?? 1);
  let volume = $state(Number.isFinite(storedVolume) ? Math.min(1, Math.max(0, storedVolume)) : 1);
  $effect(() => { try { localStorage.setItem('capture.volume', String(volume)); } catch { /* private mode */ } });
  let view = $state<Range>({ a: 0, b: 1 });
  let selection = $state<Range | null>(null);
  let confirmed = $state<Range | null>(null);
  let aligning = $state(false);
  let dragging = $state(false);
  let trimMessage = $state('');
  let error = $state('');
  let momentsOpen = $state(false);
  let query = $state('');
  let eventFilter = $state('all');
  let page = $state(0);
  let renaming = $state('');
  let markerName = $state('');
  let saving = $state(false);
  let focusedEvent = $state('');
  let pending = $state<{ t: number; play: boolean } | null>(null);
  let prefetchedFor = '';
  let seekTimer: ReturnType<typeof setTimeout> | undefined;
  let seekFrame = 0;
  let trimGeneration = 0;
  let stepGeneration = 0;
  const frameCache: { session: string; frames: FrameTiming[] }[] = [];
  let alive = true;
  const sessions = $derived(app.history?.sessions ?? []);
  const games = $derived([...new Set([...sessions].reverse().filter(s => s.intervals.length).map(s => s.game))]);
  const gameSessions = $derived(sessions.filter(s => s.game === selected));
  // All of the game's footage on one line (see lib/gameline). Everything below is in line time;
  // `real()` turns a position into the session and session time the engine works in.
  let lineOf: { game: string; line: GameLine } | null = null;
  const line = $derived.by(() => {
    const built = buildLine(gameSessions, lineOf?.game === selected ? lineOf.line : undefined);
    lineOf = { game: selected, line: built };
    return built;
  });
  const real = (v: number) => toReal(line, v);
  const foot = (v: number) => footage(line, v);
  const intervals = $derived(line.intervals);
  const rv = $derived.by(() => {
    const r = app.review;
    const start = r ? toLine(line, r.session, r.start) : null;
    return r && start !== null ? { ...r, start, end: start + (r.end - r.start) } : null;
  });
  const onLine = (kinds: string[]) => gameSessions.flatMap(s => s.events.filter(e => kinds.includes(e.kind) && Number.isFinite(e.t)).flatMap(e => {
    const t = toLine(line, s.id, e.t, e.kind === 'clip' ? 2.5 : .3);
    return t === null ? [] : [{ ...e, t, end: e.end == null ? null : Math.min(t + (e.end - e.t), (containing(line.intervals, t) ?? [0, t])[1]) }];
  }));
  // Markers and screenshots whose footage is still kept, and the clips already saved from it.
  const events = $derived(onLine(['marker', 'screenshot']).sort((a,b) => a.t-b.t || a.id.localeCompare(b.id)));
  const clips = $derived(onLine(['clip']));
  const live = $derived(gameSessions.some(s => !s.ended));
  const first = $derived(intervals[0]?.[0] ?? 0);
  const last = $derived(Math.max(first+.001, intervals.at(-1)?.[1] ?? 0));
  // Where each session begins, for the date labels above the timeline.
  const day = (ms: number) => {
    const d = new Date(ms), today = new Date(), yesterday = new Date(Date.now() - 86400000);
    const name = d.toDateString() === today.toDateString() ? 'Today' : d.toDateString() === yesterday.toDateString() ? 'Yesterday' : d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    return `${name} ${d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}`;
  };
  const labels = $derived(line.pieces.filter((p, i) => i === 0 || line.pieces[i-1].session !== p.session).map(p => ({ t: p.v, text: day(p.origin + p.a * 1000) })));
  const markers = $derived(events.filter(e => e.kind === 'marker'));
  const markerIndex = $derived(markers.findIndex(e=>e.id===focusedEvent && Math.abs(e.t-position)<.001));
  const previous = $derived(markerIndex>=0 ? markers[markerIndex-1] : [...markers].reverse().find(e => e.t < position-.0001));
  const next = $derived(markerIndex>=0 ? markers[markerIndex+1] : markers.find(e => e.t > position+.0001));
  const filteredEvents = $derived(events.filter(e => (eventFilter==='all' || e.kind===eventFilter) && `${e.label} ${clock(foot(e.t))}`.toLowerCase().includes(query.toLowerCase())));
  const valid = $derived(validSelection(intervals, selection) && selection?.a===confirmed?.a && selection?.b===confirmed?.b);
  const zoomed = $derived(view.b-view.a < last-first-.001);
  const title = (ev: TimelineEvent) => ev.label || (ev.kind==='screenshot'?'Screenshot':'Marker');
  const available = (t: number) => !!containing(intervals,t);
  onMount(() => {
    engine('history');
    const timer = setInterval(() => engine('history'), 2500);
    const key = (e: KeyboardEvent) => {
      if (e.defaultPrevented || (e.target as HTMLElement)?.closest('input,select,textarea') || e.ctrlKey || e.metaKey) return;
      if(e.key===' ' && (e.target as HTMLElement)?.closest('button'))return;
      if (![' ','ArrowLeft','ArrowRight','Home','End','[',']',',','.','i','o','f','+','=','-','0'].includes(e.key.toLowerCase())) return;
      e.preventDefault();
      if (e.key===' ') toggle();
      else if (e.key==='ArrowLeft' || e.key==='ArrowRight') {
        const delta = e.key==='ArrowRight'?1:-1;
        if(e.shiftKey) view=fitView(view.a+delta*(view.b-view.a)*.2,view.b-view.a,first,last);
        else if (e.altKey) stepFrame(delta);
        else skip(delta*5);
      } else if(e.key==='Home') seek(intervals[0]?.[0]??first,false);
      else if(e.key==='End') seek((intervals.at(-1)?.[1]??last)-.05,false);
      else if(e.key==='[' && previous) eventClick(previous);
      else if(e.key===']' && next) eventClick(next);
      else if(e.key===',' || e.key==='.') stepFrame(e.key==='.'?1:-1);
      else if(e.key.toLowerCase()==='i') setEdge('a');
      else if(e.key.toLowerCase()==='o') setEdge('b');
      else if(e.key.toLowerCase()==='f') fullscreen();
      else if(e.key==='0') view={a:first,b:last};
      else if(['+','=','-'].includes(e.key)) view=zoomView(view,e.key==='-'?2:.5,position,first,last);
    };
    window.addEventListener('keydown',key);
    window.addEventListener('keydown',escape,true);
    return () => { alive=false;clearTimeout(seekTimer);cancelAnimationFrame(seekFrame);clearInterval(timer);window.removeEventListener('keydown',key);window.removeEventListener('keydown',escape,true);video?.pause();releaseReview(); };
  });
  $effect(() => {
    if (selected || !games.length) return;
    // Asked for by name (Library), else the game being recorded right now (overlay), else the newest.
    const recording = [...sessions].reverse().find(s => !s.ended && s.intervals.length)?.game;
    choose([gameName, recording, app.engine?.target_name, games[0]].find(name => name && games.includes(name)) ?? games[0]);
  });
  // Reused cached paths do not fire loadedmetadata a second time.
  $effect(() => { if(rv && pending && video?.readyState>=1 && !app.reviewLoading) loaded(); });
  let bounds: {id:string;first:number;last:number} | null = null;
  $effect(() => {
    if (!selected) return;
    const old=bounds;
    if(old?.id===selected && (old.first!==first || old.last!==last)) {
      // Showing everything: keep showing everything as footage arrives. Zoomed: stay put.
      if(Math.abs(view.a-old.first)<.01 && Math.abs(view.b-old.last)<.01) view={a:first,b:last};
      else view=fitView(view.a,view.b-view.a,first,last);
    }
    bounds={id:selected,first,last};
  });
  /** Open the playback window at line time `v`. */
  function open(v: number, prefetch = false) {
    const at = real(v);
    if (at) historyReview(at.session, at.t, prefetch);
  }
  function choose(name: string) {
    clearTimeout(seekTimer);cancelAnimationFrame(seekFrame);video?.pause();releaseReview();cancelTrim();
    selected=name;selection=null;error='';pending=null;prefetchedFor='';focusedEvent='';page=0;frameCache.length=0;
    const newest=intervals.at(-1);
    view={a:first,b:last};bounds={id:name,first,last};
    if(!newest){position=first;error='This game’s video is no longer kept. Clips and screenshots you saved are in your Library.';return;}
    // Open on the most recent gameplay.
    const at=Math.max(newest[0],newest[1]-25);
    position=at;pending={t:at,play:false};open(at);
  }
  function pause() { video?.pause();paused=true;if(pending)pending={...pending,play:false}; }
  function seek(t: number, play=false) {
    stepGeneration++;
    clearTimeout(seekTimer);cancelAnimationFrame(seekFrame);t=clamp(t,first,Math.max(first,last-.0001));pause();position=t;focusedEvent='';
    if(!available(t)) {
      // A click in a gap lands on the nearest recorded moment instead of on nothing.
      const near = nearest(t);
      if(near===null) { pending=null;releaseReview();error='Nothing was recorded here.';return; }
      t=near;position=t;
    }
    error='';
    if(rv && t>=rv.start && t<rv.end-.0001 && video?.readyState>=1 && !app.reviewLoading) {
      pending=null;video.currentTime=t-rv.start;if(play)video.play().catch(()=>{});return;
    }
    pending={t,play};prefetchedFor='';open(t);
  }
  /** The recorded moment closest to `t` (the start of the next section, or the end of the previous one). */
  function nearest(t: number): number | null {
    let best: number | null = null;
    for (const [a, b] of intervals) {
      for (const edge of [a, Math.max(a, b - .05)]) if (best === null || Math.abs(edge - t) < Math.abs(best - t)) best = edge;
    }
    return best;
  }
  /** Jump by `delta` seconds of footage, crossing gaps instead of stopping in them. */
  function skip(delta: number) {
    let t = position + delta;
    if (!available(t)) {
      const next = delta > 0 ? intervals.find(([a]) => a > position)?.[0] : [...intervals].reverse().find(([, b]) => b <= position)?.[1];
      t = next === undefined ? (delta > 0 ? (intervals.at(-1)?.[1] ?? last) - .05 : intervals[0]?.[0] ?? first) : delta > 0 ? next : next - Math.min(-delta, 5);
    }
    seek(t, !paused);
  }
  function scrub(t: number, final: boolean) {
    pause();
    position=clamp(t,first,last-.0001);
    clearTimeout(seekTimer);
    cancelAnimationFrame(seekFrame);
    if(final) { seek(position,false); return; }
    if (pending && Math.abs(pending.t-position)>.0001) {
      pending=null;
      cancelPendingReview();
    }
    if (rv && position>=rv.start && position<rv.end-.0001 && available(position)) {
      seekFrame=requestAnimationFrame(()=>seek(position,false));
    } else {
      seekTimer=setTimeout(()=>{if(alive)seek(position,false);},85);
    }
  }
  function loaded() {
    if(!video || !rv)return;
    const target=pending;
    if(target && (target.t<rv.start-.1 || target.t>rv.end+.1))return;
    video.currentTime=clamp((target?.t??position)-rv.start,0,Math.max(0,video.duration-.0001));
    position=rv.start+video.currentTime;pending=null;
    if(target?.play)video.play().catch(()=>{});
  }
  function toggle() {
    if(!rv || error || app.reviewLoading){seek(position,true);return;}
    if(video?.paused)video.play().catch(()=>{});else pause();
  }
  function nextWindow() {
    if(!rv)return null;
    const interval=intervals.find(([,b])=>b>rv.end+.1);
    return interval ? Math.max(rv.end,interval[0]) : null;
  }
  function advance() {
    const at=nextWindow();if(at==null){paused=true;return;}
    if(app.nextReview?.session===selected && app.nextReview.start<=at+.1 && app.nextReview.end>at) {
      const old=rv;video?.pause();pending={t:at,play:true};position=at;
      app.review=app.nextReview;app.nextReview=null;prefetchedFor='';
      if(old)setTimeout(()=>engine('release_review',{path:old.path}),300);
    }else seek(at,true);
  }
  function timeupdate() {
    if(!rv || pending || app.reviewLoading || !video || paused)return;
    position=rv.start+video.currentTime;
    if(!paused && (position<view.a || position>view.b)) view=fitView(position-(view.b-view.a)*.15,view.b-view.a,first,last);
    if(paused || rv.end-position>8 || prefetchedFor===rv.path)return;
    const at=nextWindow();if(at==null)return;
    prefetchedFor=rv.path;open(at,true);
  }
  function eventClick(ev: TimelineEvent) {
    pause();position=ev.t;focusedEvent=ev.id;
    if(!available(ev.t)){pending=null;releaseReview();error='The video around this moment is no longer kept.';return;}
    seek(ev.t,false);focusedEvent=ev.id;const size=Math.min(view.b-view.a,120);view=fitView(ev.t-size/2,size,first,last);
  }
  function makeSelection(t=position) {
    if (saving) return;
    pause();
    confirmed=null;
    selection=selectionAround(intervals,t,app.settings?.markers.protect_before_secs??30,app.settings?.markers.protect_after_secs??15);
    if(!selection){error='Choose a moment with available footage to start a clip.';return;}
    error='';
    const length=selection.b-selection.a;
    if(selection.a<view.a || selection.b>view.b || length<(view.b-view.a)*.12) {
      view=fitView(selection.a-5,Math.max(20,length+10),first,last);
    }
    alignSelection();
  }

  async function framesAt(at: number) {
    const id=selected;
    const cached=frameCache.find(entry=>entry.session===id && entry.frames[0].start<at-.05 && entry.frames.at(-1)!.end>at+.05);
    if (cached) return cached.frames;
    const from=real(at);
    if(!from)return [];
    // The engine answers in session time; shift into line time and keep this stretch only.
    const shift=at-from.t, [a,b]=[from.piece.v,from.piece.v+from.piece.b-from.piece.a];
    const frames=(await historyFrames(from.session,from.t)).map(f=>({...f,start:f.start+shift,end:f.end+shift})).filter(f=>f.start>=a-.000001 && f.end<=b+.05);
    if (frames.length && id===selected) {
      frameCache.unshift({session:id,frames});
      if(frameCache.length>12)frameCache.pop();
    }
    return frames;
  }

  async function alignSelection() {
    if (!selection) return;
    const generation=++trimGeneration;
    const requested={...selection};
    const interval=intervals.find(([a,b])=>requested.a>=a-.000001 && requested.b<=b+.000001);
    trimMessage='';
    aligning=false;
    if (!interval) { trimMessage='Trim within a single recorded section.';return; }
    aligning=true;
    const [starts, ends]=await Promise.all([
      requested.a===confirmed?.a ? [] : framesAt(requested.a),
      requested.b===confirmed?.b ? [] : framesAt(requested.b),
    ]);
    if (!alive || generation!==trimGeneration) return;
    aligning=false;
    const a=requested.a===confirmed?.a ? requested.a : starts.findLast(f=>f.key && f.start<=requested.a+.000001 && f.start>=interval[0]-.000001)?.start;
    const eligible=ends.filter(f=>f.start>=(a??Infinity)-.000001 && f.end<=interval[1]+.000001);
    const closest=eligible.reduce<FrameTiming | undefined>((best, frame)=>!best || Math.abs(frame.end-requested.b)<Math.abs(best.end-requested.b)?frame:best,undefined);
    const b=requested.b===confirmed?.b ? requested.b : closest?.end;
    if (a===undefined || b===undefined || b<=a) {
      trimMessage='Move the handles to available frames.';
      return;
    }
    confirmed={a,b};
    selection={a,b};
  }

  function trimDrag(active: boolean) {
    dragging=active;
    if (active) { pause();trimGeneration++;aligning=false;trimMessage=''; }
    else alignSelection();
  }

  function cancelTrim() {
    trimGeneration++;
    selection=null;
    confirmed=null;
    aligning=false;
    dragging=false;
    trimMessage='';
  }

  async function stepFrame(direction: number, edge?: 'a'|'b') {
    if (edge && (!selection || dragging || saving)) return;
    pause();
    const generation=++stepGeneration;
    const edit=trimGeneration;
    const id=selected;
    const at=edge ? selection![edge] : position;
    const interval=containing(intervals,edge==='b'?at-.000001:at);
    if(!interval)return;
    const frames=await framesAt(at);
    if (!alive || generation!==stepGeneration || id!==selected || (edge && edit!==trimGeneration)) return;
    const times=frames.filter(frame=>frame.start>=interval[0]-.000001 && frame.end<=interval[1]+.05 && (edge!=='a'||frame.key))
      .map(frame=>edge==='b'?frame.end:frame.start);
    const index=times.findLastIndex(t=>t<=at+.000001);
    const next=times[index+direction];
    if(next===undefined)return;
    if (!edge) { seek(next,false);return; }
    if (!selection || (edge==='a' ? next>=selection.b : next<=selection.a)) return;
    selection={...selection,[edge]:next};
    alignSelection();
  }

  function setEdge(edge:'a'|'b') {
    if(saving||dragging)return;
    if(!selection)makeSelection();
    if(!selection)return;
    if(edge==='a' ? position>=selection.b : position<=selection.a)return;
    selection={...selection,[edge]:position};
    alignSelection();
  }

  function escape(event: KeyboardEvent) {
    if(event.key!=='Escape' || document.fullscreenElement)return;
    if((event.target as HTMLElement)?.closest('input,select,textarea'))return;
    if(zoomMenu?.open)zoomMenu.open=false;
    else if(momentsOpen)momentsOpen=false;
    else if(selection && !saving)cancelTrim();
    else return;
    event.preventDefault();
    event.stopPropagation();
  }

  // A save that failed, in line time (null when it was another game's or its footage is gone).
  const failedSave = $derived.by(() => {
    for (const job of app.history?.failed_saves ?? []) { const start = toLine(line, job.session, job.start); if (start !== null) return { error: job.error, start, end: start + (job.end - job.start) }; }
    return null;
  });
  async function retry(start:number,end:number) {
    pause();
    cancelTrim();
    selection={a:start,b:end};
    view=fitView(start-5,Math.max(20,end-start+10),first,last);
    await alignSelection();
  }
  async function save() {
    if(!selection || !valid || saving || dragging || aligning)return;
    saving=true;
    const from=real(selection.a);
    if(!from){saving=false;return;}
    const finished=await saveTrim(from.session,from.t,from.t+(selection.b-selection.a));
    if(!alive)return;
    saving=false;
    if(finished){cancelTrim();toast('Clip saved to your Library','ok',2500);}
  }
  async function editMarker(ev:TimelineEvent) {
    await engine('history_marker',{session:'',id:ev.id,label:markerName,remove:false});renaming='';engine('history');
  }
  async function fullscreen() {
    try{if(document.fullscreenElement)await document.exitFullscreen();else await stage?.requestFullscreen();}
    catch{error='Fullscreen is unavailable in this view.';}
  }
</script>

<section class="history" class:compact aria-label="Replay history">
  {#if !selected}
    <div class="empty"><h2>{app.history?'Nothing recorded yet':'Loading recordings…'}</h2><p>{app.history?(app.history.reason??'Play a game with recording on and its history appears here.'):''}</p><button class="btn" onclick={onclose}>Back</button></div>
  {:else}
    {#if compact && games.length>1}<div class="session-nav">
      <select class="select grow" aria-label="Game" value={selected} onchange={e=>choose(e.currentTarget.value)}>{#each games as name}<option value={name}>{name}</option>{/each}</select>
    </div>{/if}
    <div class="stage" bind:this={stage}>
      {#if rv}
        {#key rv.path}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video bind:this={video} src={convertFileSrc(rv.path)} preload="auto" {muted} bind:volume bind:paused onloadedmetadata={loaded} onended={advance} ontimeupdate={timeupdate} onclick={toggle} ondblclick={fullscreen} onerror={()=>error='This moment could not be played. Try again, or clip it: the recording itself is intact.'}></video>
        {/key}
      {:else}<div class="stage-empty"><span>▷</span><p>{intervals.length?(error?'Footage unavailable':'Opening gameplay…'):'Video no longer kept'}</p><p class="small muted">{selected}</p></div>{/if}
      {#if app.reviewLoading}<span class="busy" role="status">Opening moment…</span>{/if}
      {#if error}<div class="stage-message" role="status">{error}</div>{/if}
    </div>
    <div class="viewer-dock">
      {#if app.reviewError || trimMessage || failedSave}
        <div class="notices">
          {#if app.reviewError}<div class="notice" role="alert"><span>{app.reviewError}</span><button class="btn sm" onclick={()=>seek(position,false)}>Retry</button></div>{/if}
          {#if trimMessage}<p class="notice" role="status">{trimMessage}</p>{/if}
          {#if !selection}
            {#if failedSave}<div class="notice"><span title={failedSave.error}>A clip could not be saved.</span><button class="btn sm" onclick={()=>failedSave&&retry(failedSave.start,failedSave.end)}>Review trim</button></div>{/if}
          {/if}
        </div>
      {/if}
      <ReplayTimeline bind:this={timeline} locate={real} clockAt={foot} wallAt={v=>wallClock(line,v)} {labels} {first} {last} {intervals} {events} {clips} {position}
        bind:selection bind:view editable={!saving} onseek={scrub} onevent={eventClick} ontrim={trimDrag}
        onstep={(edge,direction)=>stepFrame(direction,edge)} />
      <div class="transport">
        <div class="viewer-tools">
          {#if events.length}
            <button class="btn ghost icon" class:active={momentsOpen} aria-label="Moments" title="Moments" aria-expanded={momentsOpen}
              onclick={()=>{momentsOpen=!momentsOpen;if(zoomMenu)zoomMenu.open=false;}}><Icon name="marker" size={16}/></button>
          {/if}
          <button class="btn ghost icon" aria-label="Fullscreen" title="Fullscreen (F)" onclick={fullscreen}><Icon name="fullscreen" size={17}/></button>
          <button class="btn ghost icon" aria-label={muted?'Unmute':'Mute'} title={muted?'Unmute':'Mute'} aria-pressed={muted}
            onclick={()=>muted=!muted}><Icon name={muted||volume===0?'speaker-off':'speaker'} size={17}/></button>
          <input class="volume" type="range" min="0" max="1" step="0.05" aria-label="Volume" title="Volume" bind:value={volume} oninput={()=>muted=false} />
          <details class="zoom-menu" bind:this={zoomMenu}>
            <summary class="btn ghost icon" title="Timeline zoom" aria-label="Timeline zoom" onclick={()=>momentsOpen=false}><Icon name="search" size={16}/></summary>
            <div class="zoom-tools" aria-label="Zoom and pan">
              <button class="btn ghost icon" aria-label="Pan earlier" title="Pan earlier" disabled={!zoomed} onclick={()=>timeline?.pan(-(view.b-view.a)*.65)}>‹</button>
              <button class="btn ghost icon" aria-label="Pan later" title="Pan later" disabled={!zoomed} onclick={()=>timeline?.pan((view.b-view.a)*.65)}>›</button>
              <button class="btn ghost icon" aria-label="Zoom out" title="Zoom out" disabled={!zoomed} onclick={()=>timeline?.zoom(2)}>−</button>
              <button class="btn ghost icon" aria-label="Zoom in" title="Zoom in" disabled={view.b-view.a<=2.001} onclick={()=>timeline?.zoom(.5)}>+</button>
              <button class="btn ghost sm" onclick={()=>timeline?.fit()}>Fit</button>
            </div>
          </details>
        </div>
        <div class="playback">
          {#if markers.length}<button class="btn ghost icon" aria-label="Previous marker" title="Previous marker [" disabled={!previous} onclick={()=>previous&&eventClick(previous)}><Icon name="skip-back" size={15}/></button>{/if}
          <button class="btn ghost icon" aria-label="Back 10 seconds" title="Back 10 seconds" disabled={!intervals.length} onclick={()=>skip(-10)}><Icon name="rewind" size={16}/></button>
          <button class="btn play" aria-label={paused?'Play':'Pause'} disabled={!intervals.length} onclick={toggle}><Icon name={paused?'play':'pause'} size={17}/></button>
          <button class="btn ghost icon" aria-label="Forward 10 seconds" title="Forward 10 seconds" disabled={!intervals.length} onclick={()=>skip(10)}><Icon name="forward" size={16}/></button>
          {#if markers.length}<button class="btn ghost icon" aria-label="Next marker" title="Next marker ]" disabled={!next} onclick={()=>next&&eventClick(next)}><Icon name="skip-fwd" size={15}/></button>{/if}
          <span class="time" title="Alt + arrow or comma / period to step a frame">{clock(foot(position),selection?3:false)} <span class="muted">/ {clock(line.total)}</span></span>
          {#if live}<button class="live" title="Jump to the newest footage" onclick={()=>seek((intervals.at(-1)?.[1]??last)-3,false)}><span></span>Recording</button>{/if}
        </div>
        <div class="transport-end">
          {#if selection}
            <output class="selected-duration" aria-label="Clip duration" title={'In '+clock(foot(selection.a),3)+' · Out '+clock(foot(selection.b),3)}>{(selection.b-selection.a).toFixed(3)}s</output>
            <button class="btn ghost cancel" aria-label="Cancel clip" disabled={saving} onclick={cancelTrim}>Cancel</button>
            <button class="btn primary" disabled={!valid||saving||dragging||aligning} onclick={save}>{saving?'Saving…':'Save Clip'}</button>
          {:else}
            <button class="btn primary" disabled={!available(position)} onclick={()=>makeSelection()}><Icon name="scissors" size={16}/>Clip</button>
          {/if}
        </div>
      </div>
    {#if momentsOpen}<div class="moments">
      <div class="moments-header"><strong>Moments</strong><input class="input" type="search" aria-label="Find a moment" placeholder="Find a moment…" bind:value={query} oninput={()=>page=0} /><select class="select" aria-label="Moment type" bind:value={eventFilter} onchange={()=>page=0}><option value="all">All moments</option><option value="marker">Markers</option><option value="screenshot">Screenshots</option></select><button class="btn ghost sm" onclick={()=>momentsOpen=false}>Close</button></div>
      <div class="moment-list">{#each filteredEvents.slice(page*40,(page+1)*40) as ev (ev.id)}<div class="moment-row">
        <button class="eventrow" onclick={()=>eventClick(ev)}>{#if ev.kind==='screenshot'&&ev.path}<img src={convertFileSrc(ev.path)} alt="Screenshot" loading="lazy" onerror={e=>e.currentTarget.style.visibility='hidden'} />{:else}<span class="marker-symbol">◆</span>{/if}<span class="event-name">{title(ev)}<small>{clock(foot(ev.t),true)} · {ev.kind==='screenshot'?'Screenshot':'Marker'}</small></span></button>
        {#if renaming===ev.id}<input class="input" aria-label="Marker name" bind:value={markerName} onkeydown={e=>{if(e.key==='Enter')editMarker(ev);if(e.key==='Escape')renaming='';}} /><button class="btn sm" onclick={()=>editMarker(ev)}>Save name</button>{:else}{#if ev.kind==='marker'}<button class="btn ghost sm" onclick={()=>{renaming=ev.id;markerName=ev.label;}}>Rename</button>{/if}<button class="btn sm" disabled={!available(ev.t)} onclick={()=>{momentsOpen=false;eventClick(ev);makeSelection(ev.t);}}>Clip around</button>{/if}
      </div>{:else}<p class="muted small no-moments">No matching moments. Add markers or take screenshots while recording to find gameplay here.</p>{/each}</div>
      {#if filteredEvents.length>40}<div class="pagination"><button class="btn sm" disabled={!page} onclick={()=>page--}>Previous</button><span class="small muted">{page*40+1}–{Math.min((page+1)*40,filteredEvents.length)} of {filteredEvents.length}</span><button class="btn sm" disabled={(page+1)*40>=filteredEvents.length} onclick={()=>page++}>Next</button></div>{/if}
    </div>{/if}
    </div>
  {/if}
</section>

<style>
  .history{display:flex;flex-direction:column;gap:0;flex:1;min-height:0;min-width:0}
  .session-nav{display:flex;align-items:center;gap:6px;flex:none;padding:6px 12px;background:var(--surface);border-bottom:1px solid var(--line)}.session-nav select{min-width:0;height:30px}
  .live{display:flex;align-items:center;gap:6px;height:26px;margin-left:8px;padding:0 9px;border:0;border-radius:6px;background:var(--danger-soft,rgba(240,90,90,.14));color:var(--danger);font-size:12px;font-weight:600;cursor:pointer;white-space:nowrap}.live span{width:7px;height:7px;border-radius:50%;background:currentColor}
  .volume{width:72px;accent-color:var(--accent)}
  .stage{flex:1;min-height:220px;position:relative;overflow:hidden;isolation:isolate;background:#080a0e;}
  .stage video{position:absolute;inset:0;width:100%;height:100%;object-fit:contain;display:block;cursor:pointer}.stage:fullscreen{border:0;border-radius:0}
  .stage-empty{position:absolute;inset:0;display:grid;place-content:center;justify-items:center;gap:5px;color:#a4abb8}.stage-empty>span{font-size:34px;color:#727a89}
  .busy,.stage-message{position:absolute;background:#1c2027ed;color:#e8ebf0;border-radius:7px;padding:8px 12px;font-size:12px}.busy{top:12px;right:12px}.stage-message{bottom:12px;left:12px;right:12px;text-align:center}
  .viewer-dock { position: relative; flex: none; background: var(--surface); }
  .transport { display: grid; grid-template-columns: minmax(0,1fr) auto minmax(0,1fr); gap: 6px; align-items: center; padding: 0 14px 8px; }
  .viewer-tools, .playback, .transport-end { display: flex; gap: 3px; align-items: center; }
  .transport-end { justify-content: flex-end; }
  .transport .icon { width: 29px; padding: 0; }
  .transport .btn { min-height: 30px; }
  .play { width: 37px; padding: 0; }
  .time, .selected-duration { font-size: 12px; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .time { padding-inline-start: 4px; }
  .selected-duration { color: var(--accent); padding-inline: 4px; }
  .active { color: var(--accent); background: var(--accent-soft); }
  .compact { gap: 7px; }
  .compact .stage { border-radius: 8px; }
  .zoom-menu { position: relative; }
  summary { cursor: pointer; list-style: none; }
  summary::-webkit-details-marker { display: none; }
  .zoom-tools { position: absolute; left: 0; bottom: calc(100% + 5px); z-index: 10; display: flex; align-items: center;
    gap: 3px; padding: 5px; border: 1px solid var(--line-strong); border-radius: 7px; background: var(--surface-2); box-shadow: var(--shadow-pop); }
  .notices { position: absolute; left: 12px; right: 12px; bottom: calc(100% + 4px); z-index: 5; display: grid; justify-items: center; gap: 3px; }
  .notice{display:flex;align-items:center;gap:8px;font-size:12px;color:var(--warn);background:var(--surface-2);padding:6px 10px;border-radius:6px}
  .moments{position:absolute;bottom:calc(100% + 5px);left:12px;width:min(540px,calc(100% - 24px));z-index:11;background:var(--surface);border:1px solid var(--line);border-radius:10px;padding:12px}.moments-header{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.moments-header strong{flex:1}.moments-header input{width:150px}.moment-list{max-height:180px;overflow:auto;margin-top:8px}.moment-row{display:flex;align-items:center;gap:6px;border-top:1px solid var(--line);padding:4px 0}
  .eventrow{display:flex;align-items:center;gap:12px;flex:1;min-width:0;padding:4px;border:0;background:transparent;color:var(--text);text-align:left;cursor:pointer;border-radius:5px}.eventrow:hover{background:var(--surface-2)}.eventrow img{width:64px;height:36px;object-fit:contain;background:#080a0e;border-radius:4px;flex:none}.marker-symbol{width:64px;text-align:center;color:var(--warn)}.event-name{font-size:13px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.event-name small{display:block;color:var(--text-2);font-size:11px;margin-top:2px}.pagination{display:flex;justify-content:flex-end;align-items:center;gap:12px;margin-top:8px}.no-moments{padding:16px}
  .empty{padding:40px;text-align:center;display:flex;flex-direction:column;align-items:center;gap:16px}.empty h2{font-size:18px;font-weight:550}.compact .stage{min-height:150px}
  @media (max-width: 720px) {
    .volume { display: none; }
    .transport { padding-inline: 7px; }
    .transport .btn:not(.icon):not(.play) { padding-inline: 7px; }
    .stage { min-height: 170px; }
  }
  @media (max-width: 520px) {
    .transport { grid-template-columns: auto 1fr; gap: 2px 6px; }
    .playback { grid-column: 1 / -1; grid-row: 1; justify-content: center; }
  }
</style>
