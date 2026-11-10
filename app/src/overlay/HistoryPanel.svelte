<script lang="ts">
  import { onMount } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { app, engine, historyReview, historySave, releaseReview, showMain } from '../lib/store.svelte';
  import type { HistorySession, TimelineEvent } from '../lib/types';
  import { clamp, clock, parseClock, containing, fitView, selectionAround, validSelection, zoomView, type Range } from '../lib/timeline';
  import ReplayTimeline from '../components/ReplayTimeline.svelte';
  let { onclose, compact = false, onlibrary }: { onclose: () => void; compact?: boolean; onlibrary?: () => void } = $props();
  let video = $state<HTMLVideoElement>(null!);
  let stage = $state<HTMLDivElement>(null!);
  let selected = $state('');
  let game = $state('');
  let position = $state(0);
  let paused = $state(true);
  let view = $state<Range>({ a: 0, b: 1 });
  let selection = $state<Range | null>(null);
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
  let alive = true;
  const rv = $derived(app.review?.session === selected ? app.review : null);
  const sessions = $derived(app.history?.sessions ?? []);
  const games = $derived([...new Set([...sessions].reverse().map(s => s.game))]);
  const visibleSessions = $derived([...sessions].reverse().filter(s => !game || s.game === game));
  const session = $derived(sessions.find(s => s.id === selected));
  const intervals = $derived(session?.intervals ?? []);
  const events = $derived((session?.events ?? []).filter(e => ['marker','screenshot'].includes(e.kind) && Number.isFinite(e.t)).sort((a,b) => a.t-b.t || a.id.localeCompare(b.id)));
  // Keep original media timestamps, including expired events before the retained origin.
  const first = $derived(Math.min(intervals[0]?.[0] ?? events[0]?.t ?? 0, events[0]?.t ?? Infinity));
  const last = $derived(Math.max(first+1, intervals.at(-1)?.[1] ?? 0, events.at(-1)?.t ?? 0));
  const markers = $derived(events.filter(e => e.kind === 'marker'));
  const markerIndex = $derived(markers.findIndex(e=>e.id===focusedEvent && Math.abs(e.t-position)<.001));
  const previous = $derived(markerIndex>=0 ? markers[markerIndex-1] : [...markers].reverse().find(e => e.t < position-.0001));
  const next = $derived(markerIndex>=0 ? markers[markerIndex+1] : markers.find(e => e.t > position+.0001));
  const filteredEvents = $derived(events.filter(e => (eventFilter==='all' || e.kind===eventFilter) && `${e.label} ${clock(e.t)}`.toLowerCase().includes(query.toLowerCase())));
  const valid = $derived(validSelection(intervals, selection));
  const title = (ev: TimelineEvent) => ev.label || (ev.kind==='screenshot'?'Screenshot':'Marker');
  const date = (ms: number) => new Date(ms).toLocaleString(undefined, {month:'short',day:'numeric',hour:'numeric',minute:'2-digit'});
  const duration = (n: number) => n>=3600?`${Math.floor(n/3600)}h ${Math.floor(n%3600/60)}m`:n>=60?`${Math.floor(n/60)}m ${Math.floor(n%60)}s`:`${n.toFixed(1)}s`;
  const available = (t: number) => !!containing(intervals,t);
  onMount(() => {
    engine('history');
    const timer = setInterval(() => engine('history'), 2500);
    const key = (e: KeyboardEvent) => {
      if (e.defaultPrevented || (e.target as HTMLElement)?.closest('input,select,textarea') || e.ctrlKey || e.metaKey) return;
      if(e.key===' ' && (e.target as HTMLElement)?.closest('button'))return;
      if (![' ','ArrowLeft','ArrowRight','Home','End','[',']','i','o','f','+','=','-','0'].includes(e.key) && !['I','O','F'].includes(e.key)) return;
      e.preventDefault();
      if (e.key===' ') toggle();
      else if (e.key==='ArrowLeft' || e.key==='ArrowRight') {
        const delta = e.key==='ArrowRight'?1:-1;
        if(e.shiftKey) view=fitView(view.a+delta*(view.b-view.a)*.2,view.b-view.a,first,last);
        else seek(position+delta*(e.altKey ? .1 : 5),false);
      } else if(e.key==='Home') seek(intervals[0]?.[0]??first,false);
      else if(e.key==='End') seek((intervals.at(-1)?.[1]??last)-.05,false);
      else if(e.key==='[' && previous) eventClick(previous);
      else if(e.key===']' && next) eventClick(next);
      else if(e.key.toLowerCase()==='i') setEdge('a',position);
      else if(e.key.toLowerCase()==='o') setEdge('b',position);
      else if(e.key.toLowerCase()==='f') fullscreen();
      else if(e.key==='0') view={a:first,b:last};
      else if(['+','=','-'].includes(e.key)) view=zoomView(view,e.key==='-'?2:.5,position,first,last);
    };
    window.addEventListener('keydown',key);
    return () => { alive=false;clearTimeout(seekTimer);clearInterval(timer);window.removeEventListener('keydown',key);video?.pause();releaseReview(); };
  });
  $effect(() => {
    if (!selected && sessions.length) choose([...sessions].reverse().find(s => s.intervals.length) ?? sessions.at(-1)!, true);
  });
  // Reused cached paths do not fire loadedmetadata a second time.
  $effect(() => { if(rv && pending && video?.readyState>=1 && !app.reviewLoading) loaded(); });
  let bounds: {id:string;first:number;last:number} | null = null;
  $effect(() => {
    if (!session) return;
    const old=bounds;
    if(old?.id===session.id && (old.first!==first || old.last!==last)) {
      if(Math.abs(view.a-old.first)<.01 && Math.abs(view.b-old.last)<.01) view={a:first,b:last};
      else view=fitView(view.a,view.b-view.a,first,last);
    }
    bounds={id:session.id,first,last};
  });
  function choose(s: HistorySession, latest=false) {
    clearTimeout(seekTimer);video?.pause();releaseReview();
    selected=s.id;selection=null;error='';pending=null;prefetchedFor='';focusedEvent='';page=0;
    const moments=s.events.filter(e=>['marker','screenshot'].includes(e.kind)&&Number.isFinite(e.t));
    const a=Math.min(s.intervals[0]?.[0]??moments[0]?.t??0,...moments.map(e=>e.t));
    const b=Math.max(a+1,s.intervals.at(-1)?.[1]??0,...moments.map(e=>e.t));
    view={a,b};bounds={id:s.id,first:a,last:b};
    if(!s.intervals.length){position=a;error='This session’s footage has expired. Your markers and screenshots are kept.';return;}
    const at=latest?Math.max(s.intervals.at(-1)![0],s.intervals.at(-1)![1]-25):s.intervals[0][0];
    position=at;pending={t:at,play:false};historyReview(s.id,at);
  }
  function pause() { video?.pause();paused=true;if(pending)pending={...pending,play:false}; }
  function seek(t: number, play=false) {
    clearTimeout(seekTimer);t=clamp(t,first,Math.max(first,last-.01));pause();position=t;focusedEvent='';
    if(!available(t)) { pending=null;releaseReview();error='No footage here. Choose a recorded section of the timeline.';return; }
    error='';
    if(rv && t>=rv.start && t<rv.end-.02 && video?.readyState>=1 && !app.reviewLoading) {
      pending=null;video.currentTime=t-rv.start;if(play)video.play().catch(()=>{});return;
    }
    pending={t,play};prefetchedFor='';historyReview(selected,t);
  }
  function scrub(t: number, final: boolean) {
    pause();position=clamp(t,first,last-.01);clearTimeout(seekTimer);
    if(final)seek(position,false);
    else seekTimer=setTimeout(()=>{if(alive)seek(t,false);},90);
  }
  function loaded() {
    if(!video || !rv)return;
    const target=pending;
    if(target && (target.t<rv.start-.1 || target.t>rv.end+.1))return;
    video.currentTime=clamp((target?.t??position)-rv.start,0,Math.max(0,video.duration-.01));
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
    prefetchedFor=rv.path;historyReview(selected,at,true);
  }
  function eventClick(ev: TimelineEvent) {
    pause();position=ev.t;focusedEvent=ev.id;
    if(!available(ev.t)){pending=null;releaseReview();error='This moment is kept, but its video has expired.';return;}
    seek(ev.t,false);focusedEvent=ev.id;const size=Math.min(view.b-view.a,120);view=fitView(ev.t-size/2,size,first,last);
  }
  function makeSelection(t=position) {
    selection=selectionAround(intervals,t,app.settings?.markers.protect_before_secs??30,app.settings?.markers.protect_after_secs??15);
    if(!selection){error='Choose a moment with available footage to start a clip.';return;}
    error='';view=fitView(selection.a-5,Math.max(20,selection.b-selection.a+10),first,last);
  }
  function setEdge(edge:'a'|'b',value:number) {
    if(!Number.isFinite(value))return;if(!selection)makeSelection();if(!selection)return;
    selection=edge==='a'?{a:clamp(value,first,selection.b-.1),b:selection.b}:{a:selection.a,b:clamp(value,selection.a+.1,last)};
  }
  function editTime(edge:'a'|'b',input:HTMLInputElement) {
    const t=parseClock(input.value);
    if(t==null){error='Enter a time as hours:minutes:seconds, for example 1:24:08.5.';input.value=clock(selection?.[edge]??0,true);return;}
    error='';setEdge(edge,t);
    input.value=clock(selection?.[edge]??0,true);
  }
  async function save() {
    if(!selection || !valid || saving)return;saving=true;
    // Preserve the range for adjustment/retry; shared engine feedback acknowledges immediately.
    await historySave(selected,selection.a,selection.b);setTimeout(()=>{if(alive)saving=false;},650);
  }
  async function editMarker(ev:TimelineEvent) {
    await engine('history_marker',{session:selected,id:ev.id,label:markerName,remove:false});renaming='';engine('history');
  }
  async function fullscreen() {
    try{if(document.fullscreenElement)await document.exitFullscreen();else await stage?.requestFullscreen();}
    catch{error='Fullscreen is unavailable in this view.';}
  }
</script>

<section class="history" class:compact aria-label="Replay history">
  {#if !sessions.length}
    <div class="empty"><h2>{app.history?'Your gameplay starts here':'Loading history…'}</h2><p>{app.history?.reason??'Recorded sessions, markers and screenshots will appear here.'}</p><button class="btn" onclick={onclose}>Back</button></div>
  {:else}
    <div class="session-nav">
      <label><span>Game</span><select class="select" aria-label="Game" bind:value={game} onchange={() => {const s=[...sessions].reverse().find(s=>!game||s.game===game);if(s)choose(s);}}><option value="">All games</option>{#each games as name}<option value={name}>{name}</option>{/each}</select></label>
      <label class="grow"><span>Background history</span><select class="select" aria-label="Game and recording" value={selected} onchange={e=>{const s=sessions.find(s=>s.id===e.currentTarget.value);if(s)choose(s);}}>{#each visibleSessions as s (s.id)}<option value={s.id}>{s.game} · {date(s.created_ms)}{!s.ended?' · Recording':''}{!s.intervals.length?' · Video expired':''}</option>{/each}</select></label>
      <button class="btn ghost library-link" onclick={() => onlibrary ? onlibrary() : showMain('library')}>Library ↗</button>
    </div>
    {#each app.history?.failed_saves??[] as job (job.id)}<div class="notice"><span title={job.error}>Clip not saved. Its available footage is protected.</span><button class="btn sm" onclick={()=>historySave(job.session,job.start,job.end)}>Retry save</button></div>{/each}
    <div class="stage" bind:this={stage}>
      {#if rv}
        {#key rv.path}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video bind:this={video} src={convertFileSrc(rv.path)} preload="auto" bind:paused onloadedmetadata={loaded} onended={advance} ontimeupdate={timeupdate} onclick={toggle} ondblclick={fullscreen} onerror={()=>error='Preview unavailable. Retry opening this moment, or save the original footage to the Library.'}></video>
        {/key}
      {:else}<div class="stage-empty"><span>▷</span><p>{intervals.length?'Opening gameplay…':'Video expired'}</p><p class="small muted">{session?.game}</p></div>{/if}
      <div class="stage-caption"><span>{session?.game}</span><span>{session?.ended?'Background history':'Recording continues'}</span></div>
      {#if app.reviewLoading}<span class="busy" role="status">Opening moment…</span>{/if}
      {#if error}<div class="stage-message" role="status">{error}</div>{/if}
    </div>
    <div class="transport">
      <div class="play-controls"><button class="btn ghost icon" aria-label="Previous marker" title="Previous marker [" disabled={!previous} onclick={()=>previous&&eventClick(previous)}>│‹</button><button class="btn ghost icon" aria-label="Back 5 seconds" title="Back 5 seconds ←" onclick={()=>seek(position-5,false)}>−5</button><button class="btn play" aria-label={paused?'Play':'Pause'} disabled={!intervals.length} onclick={toggle}>{paused?'▶':'Ⅱ'}</button><button class="btn ghost icon" aria-label="Forward 5 seconds" title="Forward 5 seconds →" onclick={()=>seek(position+5,false)}>+5</button><button class="btn ghost icon" aria-label="Next marker" title="Next marker ]" disabled={!next} onclick={()=>next&&eventClick(next)}>›│</button></div>
      <span class="time mono">{clock(position,true)} <span class="muted">/ {clock(last)}</span></span>
      <div class="transport-end"><button class="btn ghost sm" class:active={momentsOpen} onclick={()=>momentsOpen=!momentsOpen}>Moments <span class="count">{events.length}</span></button><button class="btn ghost icon" aria-label="Fullscreen" title="Fullscreen F" onclick={fullscreen}>⛶</button></div>
    </div>
    {#if app.reviewError}<div class="notice" role="alert"><span>{app.reviewError}</span><button class="btn sm" onclick={()=>seek(position,false)}>Retry</button></div>{/if}
    <ReplayTimeline {first} {last} {intervals} {events} {position} bind:selection bind:view onseek={scrub} onevent={eventClick} onscrub={pause} />
    <div class="clip-bar">
      {#if selection}
        <div class="selection-fields"><label>In <input class="input mono" aria-label="Clip start" title="Hours:minutes:seconds.tenths" data-seconds={selection.a} value={clock(selection.a,true)} onchange={e=>editTime('a',e.currentTarget)} /></label><span class="muted">→</span><label>Out <input class="input mono" aria-label="Clip end" title="Hours:minutes:seconds.tenths" data-seconds={selection.b} value={clock(selection.b,true)} onchange={e=>editTime('b',e.currentTarget)} /></label><strong>{duration(selection.b-selection.a)}</strong></div>
        <div class="clip-actions"><button class="btn ghost sm" onclick={()=>selection=null}>Cancel</button><button class="btn primary" disabled={!valid||saving} onclick={save}>{saving?'Saving…':'Save Clip'}</button></div>
        {#if !valid}<p class="range-warning" role="status">This selection includes a gap or expired footage. Keep both handles within one recorded section.</p>{/if}
      {:else}<div class="clip-prompt"><strong>Found your moment?</strong><span class="muted small">Select a clip, adjust the handles, save it to your Library.</span></div><button class="btn primary" disabled={!available(position)} onclick={()=>makeSelection()}>Select clip</button>{/if}
    </div>
    {#if momentsOpen}<div class="moments">
      <div class="moments-header"><strong>Moments in this session</strong><input class="input" type="search" aria-label="Find a moment" placeholder="Find a moment…" bind:value={query} oninput={()=>page=0} /><select class="select" aria-label="Moment type" bind:value={eventFilter} onchange={()=>page=0}><option value="all">All moments</option><option value="marker">Markers</option><option value="screenshot">Screenshots</option></select><button class="btn ghost sm" onclick={()=>momentsOpen=false}>Close</button></div>
      <div class="moment-list">{#each filteredEvents.slice(page*40,(page+1)*40) as ev (ev.id)}<div class="moment-row">
        <button class="eventrow" onclick={()=>eventClick(ev)}>{#if ev.kind==='screenshot'&&ev.path}<img src={convertFileSrc(ev.path)} alt="Screenshot" loading="lazy" onerror={e=>e.currentTarget.style.visibility='hidden'} />{:else}<span class="marker-symbol">◆</span>{/if}<span class="event-name">{title(ev)}<small>{clock(ev.t,true)} · {available(ev.t)?ev.kind==='screenshot'?'Screenshot':'Marker':'Video expired'}</small></span></button>
        {#if renaming===ev.id}<input class="input" aria-label="Marker name" bind:value={markerName} onkeydown={e=>{if(e.key==='Enter')editMarker(ev);if(e.key==='Escape')renaming='';}} /><button class="btn sm" onclick={()=>editMarker(ev)}>Save name</button>{:else}{#if ev.kind==='marker'}<button class="btn ghost sm" onclick={()=>{renaming=ev.id;markerName=ev.label;}}>Rename</button>{/if}<button class="btn sm" disabled={!available(ev.t)} onclick={()=>{eventClick(ev);makeSelection(ev.t);}}>Clip around</button>{/if}
      </div>{:else}<p class="muted small no-moments">No matching moments. Add markers or take screenshots while recording to find gameplay here.</p>{/each}</div>
      {#if filteredEvents.length>40}<div class="pagination"><button class="btn sm" disabled={!page} onclick={()=>page--}>Previous</button><span class="small muted">{page*40+1}–{Math.min((page+1)*40,filteredEvents.length)} of {filteredEvents.length}</span><button class="btn sm" disabled={(page+1)*40>=filteredEvents.length} onclick={()=>page++}>Next</button></div>{/if}
    </div>{/if}
    <div class="footnote"><span>{selection?'Original-quality MP4 · Start aligns to a nearby keyframe':'Space: play / pause · I / O: set clip boundaries · [ / ]: markers'}</span>{#if app.history?.reason}<span title={app.history.reason}>History notice: {app.history.reason}</span>{/if}</div>
  {/if}
</section>

<style>
  .history{display:flex;flex-direction:column;gap:10px;flex:1;min-height:0;min-width:0}.session-nav{display:flex;align-items:flex-end;gap:12px;flex:none}.session-nav label{display:flex;flex-direction:column;gap:5px;min-width:0}.session-nav label>span{display:none}.session-nav label:first-child{width:160px;flex:none}.session-nav .select{width:100%}.library-link{margin-bottom:1px}.stage{flex:1;min-height:200px;background:#080a0e;border:1px solid var(--line);border-radius:12px;position:relative;overflow:hidden;isolation:isolate}.stage video{position:absolute;inset:0;width:100%;height:100%;object-fit:contain;display:block}.stage:fullscreen{border:0;border-radius:0}.stage-caption{position:absolute;inset:0 0 auto;padding:14px 18px;display:flex;justify-content:space-between;gap:12px;font-size:12px;color:#d7dce5;background:linear-gradient(#0009,transparent);pointer-events:none}.stage-caption span:last-child{color:#9ca5b5;font-size:11px}.busy{position:absolute;top:45px;right:14px;padding:8px 12px;background:#1c2027e6;border-radius:6px;color:#d7dce5;font-size:12px}.stage-empty{position:absolute;inset:0;display:flex;flex-direction:column;gap:8px;align-items:center;justify-content:center;color:#a4abb8}.stage-empty>span{font-size:34px;font-weight:200;color:#727a89}.stage-message{position:absolute;inset:auto 15px 15px;padding:10px 14px;background:#1c2027ed;border:1px solid #3a414e;border-radius:7px;color:#e8ebf0;font-size:12px;text-align:center}.transport{display:flex;align-items:center;gap:16px;flex:none;min-height:38px}.play-controls{display:flex;align-items:center;gap:3px}.play{width:44px;padding:0;background:var(--surface-3)}.time{font-size:12px;font-variant-numeric:tabular-nums;white-space:nowrap}.transport-end{display:flex;align-items:center;gap:8px;margin-left:auto}.count{font-size:10px;background:var(--surface-3);padding:1px 5px;border-radius:4px}.active{color:var(--accent)!important;background:var(--accent-soft)!important}.clip-bar{display:flex;align-items:center;justify-content:space-between;flex-wrap:wrap;gap:10px;padding:12px 16px;background:var(--surface);border:1px solid var(--line);border-radius:10px;flex:none}.clip-prompt{display:flex;flex-direction:column;gap:2px}.clip-prompt strong{font-size:13px;font-weight:550}.selection-fields{display:flex;align-items:center;gap:10px}.selection-fields label{display:flex;align-items:center;gap:8px;font-size:12px;color:var(--text-2)}.selection-fields input{width:112px;height:30px;font-size:12px}.selection-fields strong{font-size:12px;color:var(--accent);font-weight:550}.clip-actions{display:flex;align-items:center;gap:6px}.range-warning{width:100%;font-size:12px;color:var(--warn)}.notice{display:flex;align-items:center;justify-content:space-between;gap:8px;font-size:12px;color:var(--warn);flex:none}.notice span{min-width:0}.footnote{display:flex;justify-content:space-between;gap:8px;flex-wrap:wrap;font-size:11px;color:var(--text-3);flex:none}.footnote>span:last-child{max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.moments{flex:none;background:var(--surface);border:1px solid var(--line);border-radius:10px;padding:12px}.moments-header{display:flex;align-items:center;gap:10px}.moments-header strong{font-size:12px;flex:1}.moments-header input{width:160px}.moment-list{max-height:185px;overflow:auto;margin-top:10px}.moment-row{display:flex;align-items:center;gap:6px;border-top:1px solid var(--line);padding:5px 0}.eventrow{display:flex;align-items:center;gap:12px;flex:1;min-width:0;padding:4px;border:0;background:transparent;color:var(--text);text-align:left;cursor:pointer;border-radius:5px}.eventrow:hover{background:var(--surface-2)}.eventrow img{width:64px;height:36px;object-fit:contain;background:#080a0e;border-radius:4px;flex:none}.marker-symbol{width:64px;text-align:center;color:var(--warn)}.event-name{font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.event-name small{display:block;color:var(--text-3);font-size:11px;margin-top:2px}.pagination{display:flex;justify-content:flex-end;align-items:center;gap:12px;margin-top:8px}.no-moments{padding:16px}.empty{padding:40px;text-align:center;display:flex;flex-direction:column;align-items:center;gap:16px}.empty h2{font-size:18px;font-weight:550}
  .compact .stage{min-height:150px}.compact .footnote{display:none}.compact .session-nav label:first-child{width:120px}.compact .stage-caption{padding:10px}.compact .library-link{display:none}
  @media(max-width:850px){.session-nav label:first-child{width:120px}.history{gap:8px}.stage{min-height:170px}.transport{gap:8px}.time{font-size:11px}.transport-end{gap:2px}.play-controls{gap:0}.play-controls .btn.icon{width:28px}.clip-bar{padding:10px 12px}.selection-fields{gap:7px}.selection-fields input{width:82px}.moments-header{flex-wrap:wrap}.moments-header strong{flex-basis:100%}.footnote{display:none}}
  @media(max-height:700px){.stage{min-height:150px}.session-nav label>span{display:none}.history{gap:7px}.clip-bar{padding:8px 12px}}
</style>
