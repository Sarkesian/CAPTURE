<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { app, cancelPendingReview, engine, fmtDuration, historyFrames, historyReview, releaseLater, releaseReview, saveTrim, toast } from '../lib/store.svelte';
  import type { FrameTiming, HistorySession, TimelineEvent } from '../lib/types';
  import { clamp, clock, containing, fitView, selectionAround, validSelection, type Range } from '../lib/timeline';
  import { cachedThumb } from '../lib/thumbs';
  import { buildLine, emptyLine, seams, footage, toLine, toReal, wallClock, type GameLine } from '../lib/gameline';
  import ReplayTimeline, { type TimelineAction } from '../components/ReplayTimeline.svelte';
  import Icon from '../components/Icon.svelte';
  // `gameName`: the game whose recording history to open (otherwise the one being recorded, else the most recent).
  let { onclose, compact = false, gameName }: { onclose: () => void; compact?: boolean; gameName?: string } = $props();
  let video = $state<HTMLVideoElement>(null!);
  let root = $state<HTMLElement>(null!);
  let timeline = $state<ReturnType<typeof ReplayTimeline>>();
  let helpOpen = $state(false);
  /** Playback speed, kept across the windows a long recording is played in. */
  const speeds = [0.5, 1, 1.5, 2];
  let speed = $state(1);
  $effect(() => { if (video) video.playbackRate = speed; });
  // Playback runs through short windows of the recording, each its own file. Two players are
  // kept: the one on screen and, underneath it, the next window already loaded and waiting on
  // its first frame. When the one on screen ends, the waiting one starts at once and takes its
  // place when it has drawn a frame, so playback carries on without a pause or a black frame.
  const players = new Map<string, HTMLVideoElement>();
  const windows = $derived(app.review ? [app.review, ...(app.nextReview && app.nextReview.path !== app.review.path ? [app.nextReview] : [])] : []);
  function player(el: HTMLVideoElement, path: string) {
    players.set(path, el);
    el.volume = volume;
    if (path === app.review?.path) video = el;
    return { destroy() { if (players.get(path) === el) players.delete(path); if (video === el) video = null!; } };
  }
  $effect(() => { const el = app.review ? players.get(app.review.path) : undefined; if (el && untrack(() => video) !== el) video = el; });
  $effect(() => { const v = volume; for (const el of players.values()) el.volume = v; });
  /** The window being handed over to (its path), while the old one still shows its last frame. */
  let handing = $state('');
  /** A brief play / pause symbol over the picture when it is clicked. */
  let flash = $state<{ icon: string; n: number } | null>(null);
  const shortcuts: [string, string][] = [
    ['Space', 'Play or pause'], ['← →', 'Back or forward 5 seconds'], [', .', 'Previous or next frame'], ['[ ]', 'Previous or next marker'],
    ['Home End', 'Start or end of the recording'], ['C', 'Clip around the playhead'], ['I O', 'Start or end the clip at the playhead'],
    ['L', 'Preview the clip in a loop'], ['Enter', 'Save the clip'], ['Esc', 'Cancel the clip'], ['+ − 0', 'Zoom in, zoom out, show everything'],
    ['Shift ← →', 'Move along a zoomed timeline'], ['Shift (dragging)', 'Move freely, without snapping'], ['M', 'Mute'], ['F', 'Fullscreen'],
  ];
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
  /** Play the clip selection over and over while trimming. */
  let loop = $state(false);
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
  // While a moment opens, the picture already seen there (the hover preview) stands in for it.
  const poster = $derived.by(() => {
    if(!pending && !app.reviewLoading)return null;
    const at=real(pending?.t ?? position);
    return at ? (cachedThumb(at.session,at.t,true) ?? cachedThumb(at.session,at.t,false))?.image ?? null : null;
  });
  onMount(() => {
    engine('history');
    const timer = setInterval(() => { if (!document.hidden) engine('history'); }, 2500);
    const key = (e: KeyboardEvent) => {
      if (e.defaultPrevented || (e.target as HTMLElement)?.closest('input,select,textarea') || e.ctrlKey || e.metaKey) return;
      if(e.key===' ' && (e.target as HTMLElement)?.closest('button'))return;
      if (e.key==='Enter' && (!selection || (e.target as HTMLElement)?.closest('button,a,summary'))) return;
      if (![' ','arrowleft','arrowright','home','end','[',']',',','.','i','o','f','+','=','-','0','m','c','l','?','enter'].includes(e.key.toLowerCase())) return;
      e.preventDefault();
      if (e.key===' ') toggle();
      else if (e.key==='?') helpOpen=!helpOpen;
      else if (e.key==='Enter') save();
      else if (e.key.toLowerCase()==='m') muted=!muted;
      else if (e.key.toLowerCase()==='c') { if(!selection)makeSelection(); }
      else if (e.key.toLowerCase()==='l') { if(selection)preview(); }
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
      else if(e.key==='0') timeline?.fit();
      else if(['+','=','-'].includes(e.key)) timeline?.zoom(e.key==='-'?1.25:.8);
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
    selected=name;selection=null;error='';frozen=false;pending=null;prefetchedFor='';focusedEvent='';page=0;frameCache.length=0;
    const newest=intervals.at(-1);
    view={a:first,b:last};bounds={id:name,first,last};
    if(!newest){position=first;error='This game’s video is no longer kept. Clips and screenshots you saved are in your Library.';return;}
    // Open on the most recent gameplay.
    const at=Math.max(newest[0],newest[1]-25);
    position=at;pending={t:at,play:false};open(at);
  }
  function pause() { if(handing){players.get(handing)?.pause();handing='';} video?.pause();paused=true;if(pending)pending={...pending,play:false}; }
  /** Playing, or about to once the moment being opened is ready. */
  const playing = () => !paused || !!pending?.play;
  function seek(t: number, play=false) {
    stepGeneration++;resumeLoop=false;
    clearTimeout(seekTimer);cancelAnimationFrame(seekFrame);t=clamp(t,first,Math.max(first,last-.0001));pause();position=t;focusedEvent='';
    if(!available(t)) {
      // A click in a gap lands on the nearest recorded moment instead of on nothing.
      const near = nearest(t);
      if(near===null) { pending=null;releaseReview();error='Nothing was recorded here.';return; }
      t=near;position=t;
    }
    error='';
    if(rv && t>=rv.start && t<rv.end-.0001 && video?.readyState>=1 && !app.reviewLoading) {
      pending=null;video.currentTime=t-rv.start;if(play){paused=false;video.play().catch(()=>{});}return;
    }
    freeze();pending={t,play};prefetchedFor='';open(t);
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
    seek(t, playing());
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
  // Playback runs through short windows of the recording, each its own file. While the next one
  // (or a moment somewhere else) opens, the last picture shown stays up instead of a black frame.
  let still = $state<HTMLCanvasElement>(null!);
  let frozen = $state(false);
  let thaw: ReturnType<typeof setTimeout> | undefined;
  function freeze() {
    const v=video;
    if(!v || !still || v.readyState<2 || !v.videoWidth)return;
    try {
      if(still.width!==v.videoWidth || still.height!==v.videoHeight){still.width=v.videoWidth;still.height=v.videoHeight;}
      still.getContext('2d')!.drawImage(v,0,0);
      frozen=true;clearTimeout(thaw);thaw=setTimeout(()=>frozen=false,4000);
    } catch { frozen=false; }
  }
  /** Let go of the held picture once the player has really drawn the new one. */
  function unfreeze() {
    const v=video;
    if(!frozen || !v)return;
    const done=()=>{if(video===v){frozen=false;clearTimeout(thaw);}};
    v.requestVideoFrameCallback(done);
    clearTimeout(thaw);thaw=setTimeout(done,1500);
  }
  function waiting(el: HTMLVideoElement, path: string) {
    const next=app.nextReview;
    if(!rv || !next || next.path!==path)return;
    const start=toLine(line,next.session,next.start);
    if(start!==null && rv.end-start>.02)el.currentTime=Math.min(rv.end-start,Math.max(0,el.duration-.0001));
  }
  function loaded() {
    if(!video || !rv)return;
    const target=pending;
    if(target && (target.t<rv.start-.1 || target.t>rv.end+.1))return;
    video.currentTime=clamp((target?.t??position)-rv.start,0,Math.max(0,video.duration-.0001));
    position=rv.start+video.currentTime;pending=null;
    unfreeze();
    if(target?.play)video.play().catch(()=>{});
  }
  // A window that cannot be read (its file went away) is rebuilt once, quietly, before saying so.
  let retriedPath = '';
  function playbackError() {
    const path=rv?.path;
    if(path && retriedPath!==path) {
      retriedPath=path;const at=position,play=!!pending?.play;
      releaseReview();pending={t:at,play};open(at);return;
    }
    error='This moment could not be played. Try again, or clip it: the recording itself is intact.';
  }
  function toggle() {
    flash={icon:playing()?'pause':'play',n:(flash?.n??0)+1};
    if(!rv || error || app.reviewLoading){if(playing())pause();else seek(position,true);return;}
    if(video?.paused)video.play().catch(()=>{});else pause();
  }
  /** Play the clip from its start, over and over; again to stop. */
  function preview() {
    if(!selection)return;
    if(loop && playing()){loop=false;pause();return;}
    loop=true;seek(selection.a,true);
  }
  function timelineAction(action: TimelineAction, t: number, ev?: TimelineEvent) {
    if(action==='goto' && ev)eventClick(ev);
    else if(action==='clip'){if(ev)eventClick(ev);cancelTrim();makeSelection(t);}
    else if(action==='start' || action==='end')setEdge(action==='start'?'a':'b',t);
    else if(action==='rename' && ev){momentsOpen=true;query='';eventFilter='all';page=Math.floor(Math.max(0,events.findIndex(e=>e.id===ev.id))/40);renaming=ev.id;markerName=ev.label;}
    else if(action==='delete' && ev)deleteMarker(ev);
  }
  async function deleteMarker(ev: TimelineEvent) {
    await engine('history_marker',{session:'',id:ev.id,label:ev.label,remove:true});
    if(focusedEvent===ev.id)focusedEvent='';
    engine('history');toast('Marker deleted','info',2500);
  }
  /** The zoom slider: 0 shows the whole recording, 1 the closest view (2 seconds). */
  const zoomLevel = $derived.by(() => { const total=last-first, min=Math.min(2,total); return total<=min+.001 ? 0 : clamp(Math.log((view.b-view.a)/total)/Math.log(min/total),0,1); });
  function setZoom(level: number) {
    const total=last-first, min=Math.min(2,total);
    timeline?.zoomTo(total*Math.pow(min/total,level),true);
  }
  const lengthText = (secs: number) => secs<60 ? `${secs.toFixed(1)} s` : `${Math.floor(secs/60)}:${(secs%60).toFixed(1).padStart(4,'0')}`;
  function nextWindow() {
    if(!rv)return null;
    const interval=intervals.find(([,b])=>b>rv.end+.1);
    return interval ? Math.max(rv.end,interval[0]) : null;
  }
  // Looping is checked on every presented frame (timeupdate only fires ~4 times a second), and
  // jumps back one frame before the out point so the frame after it is never shown.
  $effect(() => {
    const v=video, sel=selection, r=rv;
    if(!loop || !sel || !v || !r || paused)return;
    // The range changed under the playhead: start over from its new In point.
    if(untrack(()=>position)<sel.a-.001 || untrack(()=>position)>=sel.b){queueMicrotask(()=>seek(sel.a,true));return;}
    let id=0, done=false, prev=-1, jumped=false;
    const check=(_: number, meta: VideoFrameCallbackMetadata)=>{
      if(done)return;
      // The spacing between presented frames: the next frame would be about that far ahead.
      const step=prev>=0 && meta.mediaTime>prev ? Math.min(meta.mediaTime-prev,.1) : 0;
      prev=meta.mediaTime;
      const past=r.start+meta.mediaTime+step>=sel.b-.0005;
      // Jump once per pass; frames still shown from before the jump landed are not a second pass.
      if(past && !jumped){jumped=true;prev=-1;seek(sel.a,true);}
      else if(!past)jumped=false;
      if(!done)id=v.requestVideoFrameCallback(check);
    };
    id=v.requestVideoFrameCallback(check);
    return ()=>{done=true;v.cancelVideoFrameCallback(id);};
  });
  /** Start the window that is loaded and waiting, if there is one for what comes next. */
  function handOver(): boolean {
    const at=nextWindow();
    if(at==null || handing)return false;
    const queued=app.nextReview, queuedAt=queued ? toLine(line,queued.session,queued.start) : null;
    const ready=queued ? players.get(queued.path) : undefined;
    if(queued && queuedAt!==null && queuedAt<=at+.1 && queuedAt+(queued.end-queued.start)>at && ready && ready.readyState>=3 && !ready.error) {
      // The next window is loaded and sitting on the right frame: start it now. The window that
      // just ended keeps showing its last frame until the new one has drawn its first.
      const old=rv;
      if(Math.abs(ready.currentTime-(at-queuedAt))>.05)ready.currentTime=clamp(at-queuedAt,0,Math.max(0,ready.duration-.0001));
      ready.muted=muted;ready.volume=volume;ready.playbackRate=speed;
      handing=queued.path;prefetchedFor='';
      let done=false;
      const swap=()=>{
        if(done)return;done=true;
        if(handing!==queued.path || app.nextReview?.path!==queued.path || !alive){if(handing===queued.path)handing='';return;}
        handing='';pending=null;
        app.review=queued;app.nextReview=null;video=ready;paused=ready.paused;
        position=queuedAt+ready.currentTime;
        if(old)releaseLater(old.path);
      };
      ready.requestVideoFrameCallback(swap);
      setTimeout(swap,400);
      ready.play().catch(swap);
      return true;
    }
    return false;
  }
  // A player takes a few frames to get going. The waiting one is started that much before the
  // current window runs out, so its first frame arrives as the last one of this window is shown.
  const LEAD=.085;
  $effect(() => {
    const v=video;
    if(!v || paused || !rv || (loop && selection))return;
    let id=0, done=false;
    const check=(_: number, meta: VideoFrameCallbackMetadata)=>{
      if(done)return;
      if(v.duration-meta.mediaTime<=LEAD*v.playbackRate && !handing)handOver();
      if(!done)id=v.requestVideoFrameCallback(check);
    };
    id=v.requestVideoFrameCallback(check);
    return ()=>{done=true;v.cancelVideoFrameCallback(id);};
  });
  function advance() {
    if(handing)return;
    const at=nextWindow();
    // A playback window is shorter than most clips: loop only when this one reaches the Out
    // point (or nothing follows it), otherwise carry on into the next window.
    if(loop && selection && (at==null || !rv || rv.end>=selection.b-.1)){seek(selection.a,true);return;}
    if(at==null){paused=true;return;}
    // The prefetched window is in session time; the playhead is in line time.
    const queued=app.nextReview, queuedAt=queued ? toLine(line,queued.session,queued.start) : null;
    if(handOver())return;
    if(queued && queuedAt!==null && queuedAt<=at+.1 && queuedAt+(queued.end-queued.start)>at) {
      const old=rv;freeze();video?.pause();pending={t:at,play:true};position=at;
      app.review=queued;app.nextReview=null;prefetchedFor='';
      if(old)releaseLater(old.path);
    }else seek(at,true);
  }
  function timeupdate() {
    if(!rv || pending || app.reviewLoading || !video || paused)return;
    position=rv.start+video.currentTime;
    if(!paused && (position<view.a || position>view.b)) view=fitView(position-(view.b-view.a)*.15,view.b-view.a,first,last);
    if(paused || rv.end-position>12 || prefetchedFor===rv.path)return;
    const at=nextWindow();if(at==null)return;
    prefetchedFor=rv.path;open(at,true);
  }
  function eventClick(ev: TimelineEvent) {
    pause();position=ev.t;focusedEvent=ev.id;
    if(!available(ev.t)){pending=null;releaseReview();error='The video around this moment is no longer kept.';return;}
    seek(ev.t,false);focusedEvent=ev.id;
    // Keep the zoom the player chose; only bring the moment into view when it is outside it.
    const size=view.b-view.a;if(ev.t<view.a || ev.t>view.b)view=fitView(ev.t-size/2,size,first,last);
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

  /** Looping playback stopped by a handle edit, to pick up again over the new range. */
  let resumeLoop=false;
  function trimDrag(active: boolean) {
    dragging=active;
    if (active) { resumeLoop ||= loop && playing();pause();trimGeneration++;aligning=false;trimMessage=''; }
    // Restart once the handles have snapped to real frames, so the loop begins at the true In point.
    else alignSelection().then(restartLoop);
  }
  function restartLoop() {
    if(!resumeLoop || dragging || !alive)return;
    resumeLoop=false;
    if(loop && selection)seek(selection.a,true);
  }

  function cancelTrim() {
    trimGeneration++;
    selection=null;
    confirmed=null;
    aligning=false;
    dragging=false;
    trimMessage='';
    resumeLoop=false;
    loop=false;
  }

  async function stepFrame(direction: number, edge?: 'a'|'b') {
    if (edge && (!selection || dragging || saving)) return;
    if (edge) resumeLoop ||= loop && playing();
    pause();
    const generation=++stepGeneration;
    const edit=trimGeneration;
    const id=selected;
    const at=edge ? selection![edge] : position;
    const interval=containing(intervals,edge==='b'?at-.000001:at);
    // A handle that cannot move further still gives the loop back.
    if(!interval){if(edge)restartLoop();return;}
    const frames=await framesAt(at);
    if (!alive || generation!==stepGeneration || id!==selected || (edge && edit!==trimGeneration)) return;
    const times=frames.filter(frame=>frame.start>=interval[0]-.000001 && frame.end<=interval[1]+.05 && (edge!=='a'||frame.key))
      .map(frame=>edge==='b'?frame.end:frame.start);
    const index=times.findLastIndex(t=>t<=at+.000001);
    const next=times[index+direction];
    if (!edge) { if(next!==undefined)seek(next,false);return; }
    if (next!==undefined && selection && (edge==='a' ? next<selection.b : next>selection.a)) {
      selection={...selection,[edge]:next};
      await alignSelection();
    }
    restartLoop();
  }

  function setEdge(edge:'a'|'b', at=position) {
    if(saving||dragging)return;
    if(!selection)makeSelection();
    if(!selection)return;
    if(edge==='a' ? at>=selection.b : at<=selection.a)return;
    // A clip is one stretch of footage: an edge moved into another stretch does nothing.
    const home=containing(intervals,selection.a);
    if(home && (at<home[0]-.001 || at>home[1]+.001)){trimMessage='A clip stays within one recorded section.';return;}
    selection={...selection,[edge]:at};
    alignSelection();
  }

  function escape(event: KeyboardEvent) {
    if(event.key!=='Escape' || document.fullscreenElement)return;
    if((event.target as HTMLElement)?.closest('input,select,textarea'))return;
    if(helpOpen)helpOpen=false;
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
  // Fullscreen: the picture fills the screen and the timeline and controls lie over its lower
  // edge. They appear when the mouse moves or a key is pressed and leave again after a moment,
  // but stay while paused, while a clip is being made, or while the pointer is on them.
  let full = $state(false);
  let idle = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let overDock = false;
  function wake() {
    idle=false;clearTimeout(idleTimer);
    if(full)idleTimer=setTimeout(()=>{if(full && !paused && !selection && !overDock && !momentsOpen && !helpOpen)idle=true;},2200);
  }
  $effect(() => {
    const change=()=>{full=document.fullscreenElement===root;wake();};
    document.addEventListener('fullscreenchange',change);
    window.addEventListener('keydown',wake,true);
    return ()=>{document.removeEventListener('fullscreenchange',change);window.removeEventListener('keydown',wake,true);clearTimeout(idleTimer);};
  });
  // Playback starting (or a clip being finished) lets them leave again without moving the mouse.
  $effect(() => { paused;selection;untrack(wake); });
  async function fullscreen() {
    // The whole viewer goes fullscreen, so the timeline and controls stay with the picture.
    try{if(document.fullscreenElement)await document.exitFullscreen();else await root?.requestFullscreen();}
    catch{error='Fullscreen is unavailable in this view.';}
  }
</script>

<section class="history" class:compact class:full class:idle={full && idle} bind:this={root} aria-label="Replay history" onpointermove={wake} onpointerdown={wake}>
  {#if !selected}
    <div class="empty"><Icon name="replay" size={34}/><h2>{app.history?'Nothing recorded yet':'Loading recordings…'}</h2><p>{app.history?(app.history.reason??'Play a game with recording on and its history appears here.'):''}</p><button class="btn" onclick={onclose}>Back</button></div>
  {:else}
    {#if compact && games.length>1}<div class="session-nav">
      <select class="select grow" aria-label="Game" value={selected} onchange={e=>choose(e.currentTarget.value)}>{#each games as name}<option value={name}>{name}</option>{/each}</select>
    </div>{/if}
    <div class="stage">
      {#if rv}
        {#each windows as w (w.path)}
          {@const front = w.path === app.review?.path}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video use:player={w.path} class:waiting={!front} src={convertFileSrc(w.path)} preload="auto" muted={muted || !front && handing !== w.path}
            onloadedmetadata={e=>{e.currentTarget.playbackRate=speed;if(e.currentTarget===video)loaded();else waiting(e.currentTarget,w.path);}}
            onplay={e=>{if(e.currentTarget===video)paused=false;}}
            onpause={e=>{if(e.currentTarget===video && !(e.currentTarget.ended && nextWindow()!=null))paused=true;}}
            onended={e=>{if(e.currentTarget===video)advance();}} ontimeupdate={e=>{if(e.currentTarget===video)timeupdate();}}
            onclick={toggle} ondblclick={fullscreen} onerror={e=>{if(e.currentTarget===video)playbackError();}}></video>
        {/each}
      {:else}<div class="stage-empty"><Icon name={intervals.length && !error ? 'play' : 'alert'} size={30}/><p>{intervals.length?(error?'Footage unavailable':'Opening gameplay…'):'Video no longer kept'}</p><p class="small muted">{selected}</p></div>{/if}
      <canvas class="still" class:show={frozen && !!rv && !error} bind:this={still}></canvas>
      {#if poster}<img class="poster" src={poster} alt="" />{/if}
      {#if flash}{#key flash.n}<span class="flash" aria-hidden="true"><Icon name={flash.icon} size={30}/></span>{/key}{/if}
      {#if app.reviewLoading}<span class="busy" role="status"><i></i>Opening…</span>{/if}
      {#if error}<div class="stage-message" role="status">{error}</div>{/if}
    </div>
    <div class="viewer-dock" onpointerenter={()=>overDock=true} onpointerleave={()=>{overDock=false;wake();}} role="presentation">
      {#if app.reviewError || trimMessage || failedSave}
        <div class="notices" class:raised={!!selection}>
          {#if app.reviewError}<div class="notice" role="alert"><span>{app.reviewError}</span><button class="btn sm" onclick={()=>seek(position,false)}>Retry</button></div>{/if}
          {#if trimMessage}<p class="notice" role="status">{trimMessage}</p>{/if}
          {#if !selection}
            {#if failedSave}<div class="notice"><span title={failedSave.error}>A clip could not be saved.</span><button class="btn sm" onclick={()=>failedSave&&retry(failedSave.start,failedSave.end)}>Review trim</button></div>{/if}
          {/if}
        </div>
      {/if}
      {#if selection}
        <!-- Everything about the clip being made, in one place right above its handles. -->
        <div class="clipbar" role="toolbar" aria-label="Clip">
          <output class="clip-length" aria-label="Clip length" title={'From '+clock(foot(selection.a),3)+' to '+clock(foot(selection.b),3)}><Icon name="scissors" size={15}/>{lengthText(selection.b-selection.a)}</output>
          <span class="sep"></span>
          <button class="btn ghost sm" title="Start the clip at the playhead (I)" disabled={saving||dragging||position>=selection.b} onclick={()=>setEdge('a')}>Start here</button>
          <button class="btn ghost sm" title="End the clip at the playhead (O)" disabled={saving||dragging||position<=selection.a} onclick={()=>setEdge('b')}>End here</button>
          <button class="btn ghost sm" class:active={loop} aria-pressed={loop} title="Play the clip in a loop (L)" onclick={preview}><Icon name={loop && !paused ? 'pause' : 'play'} size={13}/>Preview</button>
          <span class="sep"></span>
          <button class="btn ghost sm" title="Cancel (Esc)" disabled={saving} onclick={cancelTrim}>Cancel</button>
          <button class="btn primary sm" title="Save the clip to your Library (Enter)" disabled={!valid||saving||dragging||aligning} onclick={save}><Icon name="save" size={15}/>{saving?'Saving…':'Save clip'}</button>
        </div>
      {/if}
      <ReplayTimeline bind:this={timeline} locate={real} clockAt={foot} wallAt={v=>wallClock(line,v)} {labels} {first} {last} {intervals} seams={seams(line)} {events} {clips} {position}
        bind:selection bind:view editable={!saving} focused={focusedEvent} onseek={scrub} onevent={eventClick} ontrim={trimDrag}
        onstep={(edge,direction)=>stepFrame(direction,edge)} onaction={timelineAction} />
      <div class="transport">
        <div class="viewer-tools">
          <button class="btn ghost icon" aria-label={muted?'Unmute':'Mute'} title={muted?'Unmute (M)':'Mute (M)'} aria-pressed={muted}
            onclick={()=>muted=!muted}><Icon name={muted||volume===0?'speaker-off':'speaker'} size={17}/></button>
          <input class="slider volume" type="range" min="0" max="1" step="0.05" aria-label="Volume" title="Volume" style:--fill="{(muted?0:volume)*100}%" value={muted?0:volume} oninput={e=>{volume=Number(e.currentTarget.value);muted=false;}} />
          <button class="btn ghost speed" class:active={speed!==1} aria-label="Playback speed {speed}×" title="Playback speed" onclick={()=>speed=speeds[(speeds.indexOf(speed)+1)%speeds.length]}>{speed}×</button>
          {#if events.length}
            <button class="btn ghost moments-toggle" class:active={momentsOpen} title="Markers and screenshots in this recording" aria-expanded={momentsOpen}
              onclick={()=>{momentsOpen=!momentsOpen;helpOpen=false;}}><Icon name="marker" size={15}/><span>{events.length}</span></button>
          {/if}
        </div>
        <div class="playback">
          {#if markers.length}<button class="btn ghost icon" aria-label="Previous marker" title="Previous marker ( [ )" disabled={!previous} onclick={()=>previous&&eventClick(previous)}><Icon name="skip-back" size={15}/></button>{/if}
          <button class="btn ghost icon" aria-label="Back 10 seconds" title="Back 10 seconds (← skips 5)" disabled={!intervals.length} onclick={()=>skip(-10)}><Icon name="rewind" size={16}/></button>
          <button class="btn play" aria-label={paused?'Play':'Pause'} title={paused?'Play (Space)':'Pause (Space)'} disabled={!intervals.length} onclick={toggle}><Icon name={paused?'play':'pause'} size={18}/></button>
          <button class="btn ghost icon" aria-label="Forward 10 seconds" title="Forward 10 seconds (→ skips 5)" disabled={!intervals.length} onclick={()=>skip(10)}><Icon name="forward" size={16}/></button>
          {#if markers.length}<button class="btn ghost icon" aria-label="Next marker" title="Next marker ( ] )" disabled={!next} onclick={()=>next&&eventClick(next)}><Icon name="skip-fwd" size={15}/></button>{/if}
          <span class="time" title="Time into the recording · comma and period step one frame"><b>{clock(foot(position),selection?true:false)}</b><span class="muted">&nbsp;/ {clock(line.total)}</span></span>
          {#if live}<button class="live" title="Jump to the newest footage" onclick={()=>seek((intervals.at(-1)?.[1]??last)-3,false)}><span></span>Recording</button>{/if}
        </div>
        <div class="transport-end">
          <div class="zoom" role="group" aria-label="Timeline zoom">
            <button class="btn ghost icon" aria-label="Zoom out" title="Zoom out (−)" disabled={!zoomed} onclick={()=>timeline?.zoom(1.25)}>−</button>
            <input class="slider" type="range" min="0" max="1" step="0.002" aria-label="Timeline zoom" title="Timeline zoom · also the mouse wheel over the timeline" style:--fill="{zoomLevel*100}%" value={zoomLevel} oninput={e=>setZoom(Number(e.currentTarget.value))} />
            <button class="btn ghost icon" aria-label="Zoom in" title="Zoom in (+)" disabled={view.b-view.a<=2.001} onclick={()=>timeline?.zoom(.8)}>+</button>
            <button class="btn ghost sm fit" title="Show the whole recording (0)" disabled={!zoomed} onclick={()=>timeline?.fit()}>Fit</button>
          </div>
          <button class="btn ghost icon help-toggle" class:active={helpOpen} aria-label="Keyboard shortcuts" title="Keyboard shortcuts (?)" aria-expanded={helpOpen} onclick={()=>{helpOpen=!helpOpen;momentsOpen=false;}}>?</button>
          <button class="btn ghost icon" aria-label="Fullscreen" title="Fullscreen (F)" onclick={fullscreen}><Icon name="fullscreen" size={17}/></button>
          <button class="btn primary clip" class:hidden={!!selection} title="Clip around the playhead (C)" disabled={!!selection||!available(position)} onclick={()=>makeSelection()}><Icon name="scissors" size={16}/>Clip</button>
        </div>
      </div>
    {#if helpOpen}<div class="help" role="dialog" aria-label="Keyboard shortcuts">
      <div class="moments-header"><strong>Shortcuts</strong><button class="btn ghost sm" onclick={()=>helpOpen=false}>Close</button></div>
      <dl>{#each shortcuts as [keys, what]}<div><dt>{#each keys.split(' ') as k}{#if k.startsWith('(')}<span class="muted small">{k}</span>{:else}<kbd>{k}</kbd>{/if}{/each}</dt><dd>{what}</dd></div>{/each}</dl>
      <p class="muted small">On the timeline: the mouse wheel zooms, Shift + wheel moves along it, and a right-click shows what you can do at that moment.</p>
    </div>{/if}
    {#if momentsOpen}<div class="moments">
      <div class="moments-header"><strong>Moments</strong><input class="input" type="search" aria-label="Find a moment" placeholder="Find a moment…" bind:value={query} oninput={()=>page=0} /><select class="select" aria-label="Moment type" bind:value={eventFilter} onchange={()=>page=0}><option value="all">All moments</option><option value="marker">Markers</option><option value="screenshot">Screenshots</option></select><button class="btn ghost sm" onclick={()=>momentsOpen=false}>Close</button></div>
      <div class="moment-list">{#each filteredEvents.slice(page*40,(page+1)*40) as ev (ev.id)}<div class="moment-row" class:current={ev.id===focusedEvent}>
        <button class="eventrow" onclick={()=>eventClick(ev)}>{#if ev.kind==='screenshot'&&ev.path}<img src={convertFileSrc(ev.path)} alt="Screenshot" loading="lazy" onerror={e=>e.currentTarget.style.visibility='hidden'} />{:else}<span class="marker-symbol"><Icon name="marker" size={16}/></span>{/if}<span class="event-name">{title(ev)}<small>{clock(foot(ev.t),true)} · {ev.kind==='screenshot'?'Screenshot':'Marker'}</small></span></button>
        {#if renaming===ev.id}
          <!-- svelte-ignore a11y_autofocus -->
          <input class="input" aria-label="Marker name" autofocus bind:value={markerName} onkeydown={e=>{if(e.key==='Enter')editMarker(ev);if(e.key==='Escape'){e.stopPropagation();renaming='';}}} /><button class="btn sm" onclick={()=>editMarker(ev)}>Save name</button>
        {:else}
          {#if ev.kind==='marker'}<button class="btn ghost sm icon" aria-label="Rename marker" title="Rename" onclick={()=>{renaming=ev.id;markerName=ev.label;}}><Icon name="pencil" size={15}/></button><button class="btn ghost sm icon" aria-label="Delete marker" title="Delete marker" onclick={()=>deleteMarker(ev)}><Icon name="trash" size={15}/></button>{/if}
          <button class="btn sm" disabled={!available(ev.t)||saving} onclick={()=>{momentsOpen=false;timelineAction('clip',ev.t,ev);}}><Icon name="scissors" size={14}/>Clip</button>
        {/if}
      </div>{:else}<p class="muted small no-moments">No matching moments. Add markers or take screenshots while recording to find gameplay here.</p>{/each}</div>
      {#if filteredEvents.length>40}<div class="pagination"><button class="btn sm" disabled={!page} onclick={()=>page--}>Previous</button><span class="small muted">{page*40+1}–{Math.min((page+1)*40,filteredEvents.length)} of {filteredEvents.length}</span><button class="btn sm" disabled={(page+1)*40>=filteredEvents.length} onclick={()=>page++}>Next</button></div>{/if}
    </div>{/if}
    </div>
  {/if}
</section>

<style>
  .history{display:flex;flex-direction:column;gap:0;flex:1;min-height:0;min-width:0;container-type:inline-size}
  .history:fullscreen{background:#000}
  .full .stage{position:absolute;inset:0}
  .full .viewer-dock{position:absolute;left:0;right:0;bottom:0;z-index:5;border-top:0;padding-top:26px;background:linear-gradient(transparent,color-mix(in srgb,var(--chrome) 88%,transparent) 26px,var(--chrome));transition:opacity 220ms var(--ease),transform 220ms var(--ease)}
  .full.idle .viewer-dock{opacity:0;transform:translateY(10px);pointer-events:none}
  .full.idle{cursor:none}.full.idle .stage video{cursor:none}
  .session-nav{display:flex;align-items:center;gap:6px;flex:none;padding:6px 12px;background:var(--surface);border-bottom:1px solid var(--line)}.session-nav select{min-width:0;height:30px}

  /* The picture. Everything drawn over it is quiet and gets out of the way. */
  .stage{flex:1;min-height:220px;position:relative;overflow:hidden;isolation:isolate;background:#080a0e;}
  .stage video{position:absolute;inset:0;width:100%;height:100%;object-fit:contain;display:block;cursor:pointer;z-index:0}
  /* Loaded and waiting right behind the one on screen. */
  .stage video.waiting{z-index:-1;pointer-events:none}
  .still{position:absolute;inset:0;width:100%;height:100%;object-fit:contain;pointer-events:none;z-index:1;visibility:hidden}.still.show{visibility:visible}
  .poster{position:absolute;inset:0;width:100%;height:100%;object-fit:contain;pointer-events:none;z-index:1}
  .stage-empty{position:absolute;inset:0;display:grid;place-content:center;justify-items:center;gap:6px;color:#a4abb8}.stage-empty :global(svg){color:#727a89;margin-bottom:4px}
  .busy,.stage-message{position:absolute;background:#1c2027ed;color:#e8ebf0;border-radius:var(--r-md);padding:8px 12px;font-size:12px}
  /* Only shown when opening takes long enough to notice. */
  .busy{top:12px;right:12px;z-index:2;display:flex;align-items:center;gap:8px;animation:appear 150ms var(--ease) 250ms backwards}
  .busy i{width:12px;height:12px;border-radius:50%;border:2px solid #ffffff40;border-top-color:#fff;animation:spin .7s linear infinite}
  .stage-message{bottom:12px;left:12px;right:12px;text-align:center}
  .flash{position:absolute;left:50%;top:50%;z-index:2;display:grid;place-items:center;width:68px;height:68px;margin:-34px;border-radius:50%;background:#10151dcc;color:#fff;pointer-events:none;animation:flash 450ms var(--ease) forwards}
  @keyframes flash{from{opacity:.95;transform:scale(.8)}to{opacity:0;transform:scale(1.25)}}
  @keyframes appear{from{opacity:0}}
  @keyframes spin{to{transform:rotate(1turn)}}

  .viewer-dock { position: relative; flex: none; background: var(--surface); border-top: 1px solid var(--line); padding-top: 4px; }
  .transport { display: grid; grid-template-columns: minmax(0,1fr) auto minmax(0,1fr); gap: 8px; align-items: center; padding: 2px 14px 10px; }
  .viewer-tools, .playback, .transport-end, .zoom { display: flex; gap: 2px; align-items: center; min-width: 0; }
  .transport-end { justify-content: flex-end; gap: 4px; }
  .transport .btn { height: 32px; }
  .transport .icon { width: 32px; padding: 0; font-size: 16px; }
  /* The one control used most is the biggest and the easiest to hit. */
  .transport .play { width: 42px; height: 36px; padding: 0; margin: 0 4px; border-radius: var(--r-pill); background: var(--surface-3); border-color: var(--line-strong); color: var(--text);
    transition: transform var(--t-med) var(--ease), background var(--t-fast) var(--ease); }
  .transport .play:hover:not(:disabled) { background: var(--line-strong); transform: scale(1.06); }
  .transport .play:active:not(:disabled) { transform: scale(.94); }
  .speed { width: 44px; padding: 0; font-size: 12px; font-weight: 600; font-variant-numeric: tabular-nums; }
  .moments-toggle { padding: 0 9px; gap: 5px; font-size: 12px; font-variant-numeric: tabular-nums; }
  .help-toggle { font-weight: 650; font-size: 14px !important; }
  .clip { margin-left: 6px; }
  .clip.hidden { visibility: hidden; }
  .time { font-size: 12.5px; font-variant-numeric: tabular-nums; white-space: nowrap; padding-inline-start: 8px; }
  .time b { font-weight: 600; }
  .active { color: var(--accent) !important; background: var(--accent-soft) !important; }
  .live{display:flex;align-items:center;gap:6px;height:26px;margin-left:8px;padding:0 9px;border:0;border-radius:6px;background:var(--rec-soft);color:var(--danger);font-size:12px;font-weight:600;cursor:pointer;white-space:nowrap}
  .live span{width:7px;height:7px;border-radius:50%;background:currentColor;animation:pulse 2s ease-in-out infinite}
  @keyframes pulse{50%{opacity:.35}}

  /* Volume and zoom share one slim slider. */
  .slider { appearance: none; width: 78px; height: 20px; margin: 0 4px; background: transparent; cursor: pointer; }
  .slider::-webkit-slider-runnable-track { height: 4px; border-radius: 2px; background: linear-gradient(to right, var(--text-2) var(--fill, 0%), var(--surface-3) var(--fill, 0%)); }
  .slider::-webkit-slider-thumb { appearance: none; width: 12px; height: 12px; margin-top: -4px; border-radius: 50%; background: var(--text); box-shadow: 0 1px 3px #0007; transition: transform var(--t-med) var(--ease); }
  .slider:hover::-webkit-slider-thumb { transform: scale(1.2); }
  .slider:hover::-webkit-slider-runnable-track { background: linear-gradient(to right, var(--accent) var(--fill, 0%), var(--surface-3) var(--fill, 0%)); }
  .slider:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 4px; }
  .zoom .slider { width: 86px; margin: 0; }
  .fit { margin-left: 2px; }

  /* Clip tools float just above the handles they belong to. */
  .clipbar { position: absolute; left: 50%; bottom: calc(100% + 10px); z-index: 6; transform: translateX(-50%); display: flex; align-items: center; gap: 3px; padding: 5px; max-width: calc(100% - 20px);
    background: var(--surface-2); border: 1px solid var(--line-strong); border-radius: var(--r-lg); box-shadow: var(--shadow-pop); animation: lift 160ms var(--ease); white-space: nowrap; }
  @keyframes lift { from { opacity: 0; transform: translate(-50%, 8px); } }
  .clipbar .clip-length { display: flex; align-items: center; gap: 6px; padding: 0 8px 0 7px; color: var(--accent); font-size: 13px; font-weight: 650; font-variant-numeric: tabular-nums; min-width: 78px; }
  .clipbar .sep { width: 1px; height: 18px; margin: 0 3px; background: var(--line-strong); }
  .clipbar .btn { gap: 6px; }

  .notices { position: absolute; left: 12px; right: 12px; bottom: calc(100% + 6px); z-index: 5; display: grid; justify-items: center; gap: 3px; }
  .notices.raised { bottom: calc(100% + 62px); }
  .notice{display:flex;align-items:center;gap:8px;margin:0;font-size:12px;color:var(--warn);background:var(--surface-2);border:1px solid var(--line-strong);padding:6px 10px;border-radius:var(--r-md)}

  .moments,.help{position:absolute;bottom:calc(100% + 6px);z-index:11;background:var(--surface-2);border:1px solid var(--line-strong);border-radius:var(--r-lg);box-shadow:var(--shadow-pop);padding:12px;animation:lift2 140ms var(--ease)}
  @keyframes lift2{from{opacity:0;transform:translateY(6px)}}
  .moments{left:12px;width:min(560px,calc(100% - 24px))}.help{right:12px;width:min(380px,calc(100% - 24px))}
  .moments-header{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.moments-header strong{flex:1}.moments-header input{width:150px}.moment-list{max-height:210px;overflow:auto;margin-top:8px}
  .moment-row{display:flex;align-items:center;gap:4px;border-top:1px solid var(--line);padding:3px 0}.moment-row.current .eventrow{background:var(--accent-soft)}
  .eventrow{display:flex;align-items:center;gap:12px;flex:1;min-width:0;padding:4px;border:0;background:transparent;color:var(--text);text-align:left;cursor:pointer;border-radius:5px}.eventrow:hover{background:var(--surface-3)}.eventrow img{width:64px;height:36px;object-fit:contain;background:#080a0e;border-radius:4px;flex:none}
  .marker-symbol{width:64px;height:36px;display:grid;place-items:center;border-radius:4px;background:var(--warn-soft);color:var(--warn);flex:none}.event-name{font-size:13px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.event-name small{display:block;color:var(--text-2);font-size:11px;margin-top:2px}.pagination{display:flex;justify-content:flex-end;align-items:center;gap:12px;margin-top:8px}.no-moments{padding:16px}
  .moment-row .input{width:150px;height:28px}
  .help dl{margin:10px 0 8px;display:grid;gap:5px}.help dl div{display:flex;align-items:center;justify-content:space-between;gap:12px;font-size:12.5px}.help dt{display:flex;align-items:center;gap:4px;flex:none}.help dd{margin:0;color:var(--text-2);text-align:right}
  .help kbd{min-width:22px;padding:2px 6px;border:1px solid var(--line-strong);border-radius:5px;background:var(--surface-3);font:inherit;font-size:11px;font-weight:600;text-align:center}
  .help p{margin:0;padding-top:8px;border-top:1px solid var(--line)}
  .empty{padding:40px;text-align:center;display:flex;flex-direction:column;align-items:center;gap:14px;color:var(--text-2)}.empty h2{font-size:18px;font-weight:550;color:var(--text)}
  .compact .stage{min-height:150px;border-radius:8px}

  /* Narrow views (the in-game overlay, a small window) drop the secondary controls first. */
  @container (max-width: 980px) { .zoom .slider { display: none; } }
  @container (max-width: 820px) { .volume, .speed, .help-toggle { display: none; } .transport { padding-inline: 8px; gap: 4px; } .clipbar .sep { display: none; } }
  @container (max-width: 640px) {
    .transport { grid-template-columns: auto 1fr; gap: 2px 6px; }
    .playback { grid-column: 1 / -1; grid-row: 1; justify-content: center; }
    .zoom { display: none; }
    .stage { min-height: 170px; }
    .clipbar { overflow-x: auto; }
  }
  @media (prefers-reduced-motion: reduce) { .flash, .clipbar, .moments, .help, .busy { animation: none; } .flash { display: none; } .transport .play { transition: none; } }
</style>
