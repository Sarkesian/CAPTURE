<script lang="ts">
  import { onMount } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { app, engine, historyReview, historySave, releaseReview, fmtDuration, fmtBytes } from '../lib/store.svelte';
  import type { HistorySession, TimelineEvent } from '../lib/types';
  let { onclose }: { onclose: () => void } = $props();
  let video = $state<HTMLVideoElement>(null!);
  let selected = $state('');
  let local = $state(0);
  let paused = $state(true);
  let zoom = $state(1);
  let selection = $state<{a:number;b:number} | null>(null);
  let want: {t:number;play:boolean} | null = null;
  let prefetchedFor = '';
  let error = $state('');
  const codecMessage='This player cannot decode the video track. HEVC HDR and Archival 4:4:4 require a compatible player or installed codec; the original footage is unchanged.';
  let renaming = $state('');
  let markerName = $state('');
  let scrub = $state<number | null>(null);
  const rv = $derived(app.review);
  const sessions = $derived(app.history?.sessions ?? []);
  const games=$derived([...new Set([...sessions].reverse().map(s=>s.game))]);
  const session = $derived(sessions.find(s => s.id === selected));
  const first = $derived(session?.intervals[0]?.[0] ?? 0);
  const last = $derived(session?.intervals.at(-1)?.[1] ?? 0);
  const absolute = $derived((rv?.start ?? 0) + local);
  const span = $derived(Math.max(1,(last-first)/zoom));
  const left = $derived(Math.max(first,Math.min(absolute-span/2,last-span)));
  const right = $derived(Math.min(last,left+span));
  const available = (t:number) => session?.intervals.some(([a,b]) => t>=a && t<b) ?? false;
  const pct = (t:number) => Math.max(0,Math.min(100,(t-left)/Math.max(1,right-left)*100));
  const clock = (t:number) => `${Math.floor(t/3600)}:${String(Math.floor(t/60)%60).padStart(2,'0')}:${String(Math.floor(t)%60).padStart(2,'0')}`;
  const events = $derived(session?.events ?? []);
  const globalRanges = $derived.by(() => {
    let offset=0; const ranges: {session:string;a:number;b:number;offset:number}[]=[];
    for(const s of sessions) for(const [a,b] of s.intervals) {ranges.push({session:s.id,a,b,offset});offset+=b-a;}
    return ranges;
  });
  const globalPosition=$derived.by(()=>{const r=globalRanges.find(r=>r.session===selected && absolute>=r.a && absolute<=r.b);return r?r.offset+absolute-r.a:0;});
  onMount(() => {
    engine('history');
    const timer=setInterval(()=>engine('history'),2500);
    const key=(e:KeyboardEvent)=>{if((e.target as HTMLElement)?.closest('input,select,textarea,button'))return;if(e.key===' '){e.preventDefault();toggle();}if(e.key==='ArrowLeft')seek(absolute-5);if(e.key==='ArrowRight')seek(absolute+5);};
    window.addEventListener('keydown',key);
    return ()=>{clearInterval(timer);window.removeEventListener('keydown',key);releaseReview();};
  });
  $effect(()=>{if(!selected && sessions.length){const s=[...sessions].reverse().find(s=>s.intervals.length)??sessions.at(-1)!;choose(s,true);}});
  function choose(s:HistorySession,latest=false){selected=s.id;selection=null;zoom=1;error='';prefetchedFor='';if(app.nextReview){engine('release_review',{path:app.nextReview.path});app.nextReview=null;}if(!s.intervals.length){releaseReview();error='The video has expired. Your events and screenshots are kept.';return;}const at=latest?Math.max(s.intervals.at(-1)![0],s.intervals.at(-1)![1]-25):s.intervals[0][0];want={t:at,play:false};historyReview(s.id,at);}
  function seek(t:number,play=!paused){
    if(t>=last)t=Math.max(first,last-.05);
    if(!available(t)){error='No recording at this position.';return;}error='';
    if(rv?.session===selected && t>=rv.start && t<rv.end-0.05){video.currentTime=t-rv.start;if(video.readyState>=1&&!video.videoWidth)error=codecMessage;return;}
    want={t,play};prefetchedFor='';historyReview(selected,t);
  }
  function loaded(){if(!video || !rv)return;if(!video.videoWidth){video.pause();error=codecMessage;return;}const w=want;want=null;local=Math.max(0,Math.min((w?.t??rv.start)-rv.start,Math.max(0,video.duration-.05)));video.currentTime=local;if(w?.play)video.play().catch(()=>{});}
  function toggle(){if(!video?.videoWidth){error=codecMessage;return;}video.paused?video.play().catch(()=>{}):video.pause();}
  function advance(){
    if(!rv || !session)return;
    const nextAt=session.intervals.find(([,b])=>b>rv.end+.1);
    const nextSession=nextAt?session:sessions.slice(sessions.findIndex(s=>s.id===session.id)+1).find(s=>s.intervals.length);
    if(!nextSession)return;
    const at=nextAt?Math.max(rv.end,nextAt[0]):nextSession.intervals[0][0];
    if(nextSession.id!==selected){selected=nextSession.id;selection=null;zoom=1;}
    if(app.nextReview?.session===selected && app.nextReview.start<=at+.1 && app.nextReview.end>at){const old=rv;app.review=app.nextReview;app.nextReview=null;want={t:at,play:true};setTimeout(()=>engine('release_review',{path:old.path}),300);prefetchedFor='';}
    else seek(at,true);
  }
  function timeupdate(){
    if(!rv || paused || !session || rv.end-absolute>8)return;
    const key=rv.path;
    if(prefetchedFor===key)return;
    const next=session.intervals.find(([,b])=>b>rv.end+.1);
    const nextSession=next?session:sessions.slice(sessions.findIndex(s=>s.id===session.id)+1).find(s=>s.intervals.length);if(!nextSession)return;
    prefetchedFor=key;historyReview(nextSession.id,next?Math.max(rv.end,next[0]):nextSession.intervals[0][0],true);
  }
  function globalSeek(value:number){const range=globalRanges.find(r=>value>=r.offset && value<r.offset+r.b-r.a)??globalRanges.at(-1);if(!range)return;const s=sessions.find(s=>s.id===range.session)!;if(selected!==s.id){selected=s.id;selection=null;zoom=1;}seek(range.a+Math.min(range.b-range.a-.05,value-range.offset),false);}
  function eventClick(ev:TimelineEvent){if(!available(ev.t)){error='This event is kept, but its video has expired.';return;}seek(ev.t,false);if(ev.kind==='marker'){const before=app.settings?.markers.protect_before_secs??30,after=app.settings?.markers.protect_after_secs??15;selection={a:Math.max(first,ev.t-before),b:Math.min(last,ev.t+after)};}}
  async function save(){if(!selection)return;await historySave(selected,selection.a,selection.b);selection=null;}
  async function editMarker(ev:TimelineEvent,remove=false){await engine('history_marker',{session:selected,id:ev.id,label:remove?null:markerName,remove});renaming='';engine('history');}
  function pointerTime(ev:PointerEvent){const box=(ev.currentTarget as HTMLElement).getBoundingClientRect();return Math.max(left,Math.min(right,left+(ev.clientX-box.left)/box.width*(right-left)));}
</script>

<section class="history">
  {#if !sessions.length}
    <div class="empty"><p>{app.history?.reason ?? 'Your gameplay history will appear here.'}</p><button class="btn" onclick={onclose}>Back</button></div>
  {:else}
    <div class="row">
      <select class="select grow" aria-label="Game and recording" value={selected} onchange={e=>{const s=sessions.find(s=>s.id===e.currentTarget.value);if(s)choose(s);}}>
        {#each games as game}<optgroup label={game}>{#each [...sessions].reverse().filter(s=>s.game===game) as s (s.id)}<option value={s.id}>{s.game} · {new Date(s.created_ms).toLocaleString()}</option>{/each}</optgroup>{/each}
      </select>
      <span class="small muted">{fmtDuration(app.history?.retained_secs??0)} kept · {fmtBytes(app.history?.bytes??0)}</span>
    </div>
    <label class="small muted">All gameplay <input class="global" aria-label="Global history" type="range" min="0" max={Math.max(1,app.history?.retained_secs??1)} step="1" value={globalPosition} onpointerdown={()=>video?.pause()} onchange={e=>globalSeek(Number(e.currentTarget.value))} /></label>
    {#if app.history?.reason}<p class="small muted">{app.history.reason}. Original recording quality is unchanged.</p>{/if}
    {#each app.history?.failed_saves ?? [] as job (job.id)}<div class="row small"><span class="muted grow" title={job.error}>A clip could not be saved. Its available source footage is protected.</span><button class="btn sm" onclick={()=>historySave(job.session,job.start,job.end)}>Retry save</button></div>{/each}
    <div class="stage">
      {#if rv}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={video} src={convertFileSrc(rv.path)} bind:currentTime={local} bind:paused onloadedmetadata={loaded} onended={advance} ontimeupdate={timeupdate} onclick={toggle} onerror={()=>error=codecMessage}></video>
      {/if}
      {#if app.reviewLoading}<span class="busy">Opening gameplay…</span>{/if}
    </div>
    {#if app.reviewError || error}<p role="status" class="small error">{error || app.reviewError} <button class="link" onclick={()=>seek(absolute,false)}>Retry</button></p>{/if}
    <div class="track" role="slider" tabindex="0" aria-label="Recording timeline" aria-valuemin={left} aria-valuemax={right} aria-valuenow={absolute} onkeydown={e=>{if(e.key==='ArrowLeft')seek(absolute-5);if(e.key==='ArrowRight')seek(absolute+5);}} onpointerdown={e=>{video?.pause();scrub=pointerTime(e);e.currentTarget.setPointerCapture(e.pointerId);}} onpointermove={e=>{if(scrub!=null)scrub=pointerTime(e);}} onpointerup={e=>{if(scrub!=null){seek(pointerTime(e),false);scrub=null;}}} onpointercancel={()=>scrub=null}>
      {#each session?.intervals ?? [] as [a,b]}{#if b>=left && a<=right}<div class="available" style:left="{pct(a)}%" style:width="{pct(b)-pct(a)}%"></div>{/if}{/each}
      {#if selection}<div class="selection" style:left="{pct(selection.a)}%" style:width="{pct(selection.b)-pct(selection.a)}%"></div>{/if}
      {#each events.filter(e=>e.t>=left && e.t<=right) as ev (ev.id)}
        <button class="event" class:shot={ev.kind==='screenshot'} style:left="{pct(ev.t)}%" title="{ev.label || 'Marker'} · {clock(ev.t)}" onpointerdown={e=>e.stopPropagation()} onclick={e=>{e.stopPropagation();eventClick(ev);}}>
          {#if ev.kind==='screenshot' && ev.path}<img class="hover" src={convertFileSrc(ev.path)} alt="Screenshot preview" />{/if}
        </button>
      {/each}
      <div class="head" style:left="{pct(scrub??absolute)}%"></div>
    </div>
    <div class="row controls">
      <button class="btn sm" onclick={()=>seek(absolute-5)}>−5s</button><button class="btn" onclick={toggle}>{paused?'Play':'Pause'}</button><button class="btn sm" onclick={()=>seek(absolute+5)}>+5s</button>
      <button class="btn sm" onclick={()=>{const ev=[...events].filter(e=>e.t<absolute-.5).sort((a,b)=>b.t-a.t)[0];if(ev)eventClick(ev);}}>Previous event</button>
      <button class="btn sm" onclick={()=>{const ev=events.filter(e=>e.t>absolute+.5).sort((a,b)=>a.t-b.t)[0];if(ev)eventClick(ev);}}>Next event</button>
      <span class="mono small">{clock(absolute)} / {clock(last)}</span><span class="grow"></span>
      <label class="small">Zoom <select class="select" bind:value={zoom} aria-label="Timeline zoom">{#each [1,2,4,8,16,32,64] as z}<option value={z}>{z}×</option>{/each}</select></label>
      {#if !selection}<button class="btn primary" onclick={()=>selection={a:Math.max(first,absolute-15),b:Math.min(last,absolute+5)}}>Create clip</button>{/if}
    </div>
    {#if selection}
      <div class="range row">
        <label class="grow small">Start · {clock(selection.a)}<input aria-label="Clip start" type="range" min={Math.min(left,selection.a)} max={Math.max(left,Math.min(right,selection.b-1))} step="0.1" bind:value={selection.a} /></label>
        <label class="grow small">End · {clock(selection.b)}<input aria-label="Clip end" type="range" min={Math.min(right,Math.max(left,selection.a+1))} max={Math.max(right,selection.b)} step="0.1" bind:value={selection.b} /></label>
        <button class="btn ghost" onclick={()=>selection=null}>Cancel</button><button class="btn primary" onclick={save}>Save clip</button>
      </div>
      <p class="faint xs">Original-quality MP4. The beginning may include up to a keyframe of earlier footage; no video re-encoding.</p>
    {/if}
    {#if events.length}<div class="events">{#each [...events].sort((a,b)=>a.t-b.t) as ev (ev.id)}<div class="row">
      <button class="eventrow grow" onclick={()=>eventClick(ev)}>{#if ev.kind==='screenshot' && ev.path}<img src={convertFileSrc(ev.path)} alt="Screenshot" loading="lazy" />{/if}<span>{clock(ev.t)} · {ev.label || 'Marker'}</span><span class="muted small">{available(ev.t)?'':'Video expired'}</span></button>
      {#if ev.kind==='marker'}
        {#if renaming===ev.id}<input class="input" aria-label="Marker name" bind:value={markerName} onkeydown={e=>{if(e.key==='Enter')editMarker(ev);if(e.key==='Escape')renaming='';}} /><button class="btn sm" onclick={()=>editMarker(ev)}>Save name</button>
        {:else}<button class="btn ghost sm" onclick={()=>{renaming=ev.id;markerName=ev.label;}}>Rename</button>{/if}
        <button class="btn ghost sm" onclick={()=>editMarker(ev,true)}>Remove</button>
      {/if}
    </div>{/each}</div>{/if}
  {/if}
</section>

<style>
  .history{display:flex;flex-direction:column;gap:var(--s3);flex:1;min-height:0}.row{gap:var(--s2);flex-wrap:wrap}.stage{flex:1;min-height:160px;background:#000;border-radius:var(--r-lg);position:relative;overflow:hidden}video{width:100%;height:100%;object-fit:contain;display:block}.busy{position:absolute;top:12px;right:12px;padding:8px;background:#222c;border-radius:8px}.global{width:100%;display:block}.track{position:relative;height:28px;background:repeating-linear-gradient(135deg,var(--surface-2) 0 6px,var(--surface) 6px 12px);border-radius:var(--r-sm);cursor:pointer}.available{position:absolute;height:100%;background:var(--surface-3)}.selection{position:absolute;height:100%;background:var(--accent-soft);border:1px solid var(--accent)}.head{position:absolute;top:-3px;bottom:-3px;width:2px;background:var(--text);pointer-events:none}.event{position:absolute;top:4px;width:8px;height:20px;background:var(--warn);border:0;padding:0;transform:translateX(-4px)}.event.shot{background:var(--accent);height:10px;top:15px}.hover{display:none;position:absolute;width:160px;bottom:22px;left:-80px;z-index:4}.event:hover .hover,.event:focus .hover{display:block}.range input{display:block;width:100%}.events{max-height:100px;overflow:auto}.eventrow{display:flex;align-items:center;gap:12px;padding:5px;width:100%;background:transparent;border:0;color:var(--text);text-align:left}.eventrow img{width:60px;height:34px;object-fit:contain}.error{color:var(--warn)}.empty{padding:40px;text-align:center}p{margin:0}
</style>
