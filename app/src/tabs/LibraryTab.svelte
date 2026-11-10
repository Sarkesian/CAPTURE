<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { convertFileSrc, invoke } from '@tauri-apps/api/core';
  import { app, engine, openPath, revealPath, copyPath, openCaptureFolder, deleteClip, requestClipInfo, toast, fmtDuration } from '../lib/store.svelte';
  import type { ClipEntry, HistorySession } from '../lib/types';
  import Segmented from '../components/Segmented.svelte';
  import Icon from '../components/Icon.svelte';
  import VideoPreview from '../components/VideoPreview.svelte';
  import HistoryPanel from '../overlay/HistoryPanel.svelte';

  let { active = $bindable(false) }: { active?: boolean } = $props();
  let host: HTMLElement;
  let items = $state<ClipEntry[]>([]);
  let loaded = $state(false);
  let query = $state('');
  let game = $state('');
  let count = $state(60);
  let menu = $state<string | null>(null);
  let confirmDelete = $state<string | null>(null);
  let renaming = $state<string | null>(null);
  let viewing = $state<{kind: 'recording'; session: HistorySession} | {kind: 'saved'; item: ClipEntry} | null>(null);
  let scroll = 0;
  let origin: HTMLElement | null = null;
  let req = 0;
  const filters = [{value:'all',label:'All'},{value:'recordings',label:'Recordings'},{value:'clips',label:'Clips'},{value:'screenshots',label:'Screenshots'}] as const;
  const sessions = $derived(app.history?.sessions ?? []);
  const recovering = $derived(app.history?.reason?.startsWith('Recovering earlier recordings') ?? false);
  const games = $derived([...new Set([...sessions.map(s=>s.game),...items.map(c=>c.game)])].sort((a,b)=>a.localeCompare(b)));
  const matches = (g: string, name = '') => (!game || g === game) && (g+' '+name).toLowerCase().includes(query.trim().toLowerCase());
  const background = $derived.by(() => {
    const byGame = new Map<string, HistorySession[]>();
    for(const s of [...sessions].reverse().filter(s=>matches(s.game))) {
      const group=byGame.get(s.game)??[];group.push(s);byGame.set(s.game,group);
    }
    return [...byGame.values()].flat();
  });
  const shown = $derived(items.filter(c => matches(c.game,c.name) && (app.libraryFilter==='all' || (app.libraryFilter==='recordings' && c.kind==='session') || (app.libraryFilter==='clips' && c.kind==='clip') || (app.libraryFilter==='screenshots' && c.kind==='screenshot'))));
  const showBackground = $derived(app.libraryFilter==='all' || app.libraryFilter==='recordings');
  const visibleBackground = $derived(background.slice(0,app.libraryFilter==='all'?6:count));
  const date = (ms:number) => new Date(ms).toLocaleString(undefined,{month:'short',day:'numeric',hour:'numeric',minute:'2-digit'});
  const duration = (secs:number) => secs>=3600?`${Math.floor(secs/3600)}h ${Math.floor(secs%3600/60)}m`:fmtDuration(secs);
  const retained = (s:HistorySession) => s.intervals.reduce((n,[a,b])=>n+b-a,0);
  const artKey = (game:string) => game.toLowerCase().replace(/[^a-z0-9]/g,'');
  const coverFallback = $derived.by(() => {
    const covers: Record<string,string> = {};
    for (const item of items) if(item.kind==='screenshot') covers[artKey(item.game)] ??= item.path;
    return covers;
  });
  let gameArt = $state<Record<string,string>>({});
  const cover = (s:HistorySession) => gameArt[artKey(s.game)] || s.events.find(e=>e.kind==='screenshot'&&e.path)?.path || coverFallback[artKey(s.game)];
  const hue = (s:string) => 185+[...s].reduce((n,c)=>(n*31+c.charCodeAt(0))%105,0);
  const mediaLabel = (kind:ClipEntry['kind'])=>kind==='clip'?'Clip':kind==='session'?'Saved recording':'Screenshot';
  const title = (c:ClipEntry) => c.name.replace(/\.(mp4|png)$/i,'');
  const cardTitle = (c:ClipEntry) => c.kind==='screenshot' || /^\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}_/.test(c.name) ? c.game : title(c);

  async function load() {
    const mine=++req;
    try {
      const list=(await Promise.all(['clips','sessions','screenshots'].map(kind=>invoke<ClipEntry[]>('list_clips',{kind})))).flat();
      if(mine===req)items=list.sort((a,b)=>b.modified_ms-a.modified_ms);
    }catch(e){if(mine===req)toast(String(e),'error');}
    if(mine===req)loaded=true;
  }
  function filter(v:typeof app.libraryFilter) { app.libraryFilter=v;count=60; }
  async function openMedia(next:NonNullable<typeof viewing>, trigger:HTMLElement) {
    const scroller=host.closest('main');scroll=scroller?.scrollTop??0;origin=trigger;viewing=next;menu=null;
    await tick();if(scroller)scroller.scrollTop=0;host.querySelector<HTMLButtonElement>('.back')?.focus({preventScroll:true});
  }
  async function back() {
    viewing=null;await tick();const scroller=host.closest('main');if(scroller)scroller.scrollTop=scroll;origin?.focus({preventScroll:true});
  }
  onMount(()=>{
    engine('history');
    invoke<Record<string,string>>('library_game_art').then(a=>gameArt=a).catch(()=>{});
    const timer=setInterval(()=>{if(viewing?.kind!=='recording')engine('history');},2500);
    const close=(e:PointerEvent)=>{if(!(e.target as HTMLElement)?.closest('.menu,.more')){menu=null;confirmDelete=null;}};
    const escape=(e:KeyboardEvent)=>{if(e.key!=='Escape'||e.defaultPrevented||document.fullscreenElement)return;if(menu||renaming){menu=null;confirmDelete=null;renaming=null;}else if(viewing)back();};
    window.addEventListener('pointerdown',close);window.addEventListener('keydown',escape);
    return ()=>{clearInterval(timer);window.removeEventListener('pointerdown',close);window.removeEventListener('keydown',escape);};
  });
  $effect(()=>{app.libraryNavigation;untrack(()=>{viewing=null;menu=null;game='';query='';count=60;});});
  $effect(()=>{app.libraryVersion;untrack(load);});
  $effect(()=>{active=!!viewing;return ()=>{active=false;};});
  async function rename(c:ClipEntry,name:string){renaming=null;if(!name.trim()||name===title(c))return;try{await invoke('rename_clip',{path:c.path,name});await load();}catch(e){toast(String(e),'error');}}
  async function remove(c:ClipEntry){menu=null;confirmDelete=null;await deleteClip(c.path);await load();}
  async function stems(c:ClipEntry){app.stemsBusy[c.path]=true;await engine('export_stems',{path:c.path});}
</script>

<div class="library" bind:this={host}>
  {#if viewing}
    <div class="viewer">
      <header class="viewer-header">
        <button class="btn ghost back" onclick={back}><Icon name="back" size={17}/>Library</button>
        <div class="viewer-title"><h1>{viewing.kind==='recording'?viewing.session.game:viewing.item.game}</h1><p>{viewing.kind==='recording'?'Background recording · '+date(viewing.session.created_ms):mediaLabel(viewing.item.kind)+' · '+date(viewing.item.modified_ms)}</p></div>
        {#if viewing.kind==='saved'}<button class="btn ghost icon" title="Show in folder" aria-label="Show in folder" onclick={()=>viewing?.kind==='saved'&&revealPath(viewing.item.path)}><Icon name="folder" size={18}/></button>{/if}
      </header>
      {#if viewing.kind==='recording'}
        <HistoryPanel sessionId={viewing.session.id} onclose={back}/>
      {:else}
        <div class="saved-stage">
          {#if viewing.item.kind==='screenshot'}<img src={convertFileSrc(viewing.item.path)} alt={title(viewing.item)}/>
          {:else}
            <!-- svelte-ignore a11y_media_has_caption -->
            <video src={convertFileSrc(viewing.item.path)} controls preload="metadata" autoplay aria-label={title(viewing.item)}></video>
          {/if}
        </div>
      {/if}
    </div>
  {:else}
    <header class="heading"><div><h1>Library</h1><p>Recordings, clips and screenshots</p></div><button class="btn ghost" onclick={openCaptureFolder}><Icon name="folder" size={17}/>Open folder</button></header>
    <div class="toolbar">
      <Segmented label="Media type" value={app.libraryFilter} onchange={filter} options={[...filters]} />
      <div class="filters"><select class="select" aria-label="Filter by game" bind:value={game} onchange={()=>count=60}><option value="">All games</option>{#each games as g (g)}<option value={g}>{g}</option>{/each}</select>
      <label class="search"><Icon name="search" size={16}/><input class="input" type="search" placeholder="Search library" aria-label="Search library" bind:value={query} oninput={()=>count=60}/></label></div>
    </div>
    {#if showBackground && visibleBackground.length}
      <section aria-label="Background recordings">
        <div class="section-head"><h2>Background recordings</h2><span class="temp">Temporary</span>{#if app.libraryFilter==='all' && background.length>6}<button class="link" onclick={()=>filter('recordings')}>View all</button>{/if}</div>
        <div class="grid">
          {#each visibleBackground as s (s.id)}
            {@const image = cover(s)}
            <button class="recording-card" disabled={recovering && !s.intervals.length} aria-label={'Open background recording: '+s.game+' · '+date(s.created_ms)} onclick={e=>openMedia({kind:'recording',session:s},e.currentTarget)}>
              <span class="cover" style:--cover-hue={hue(s.game)}>
                <span class="fallback"><Icon name="game" size={36}/><b>{s.game}</b></span>
                {#if image}<img src={convertFileSrc(image)} alt="" loading="lazy" onerror={e=>e.currentTarget.style.display='none'}/>{/if}
                <span class="tag"><Icon name="replay" size={14}/>Background</span>
                <span class="duration">{s.intervals.length?duration(retained(s)):recovering?'Loading…':'Expired'}</span>
              </span>
              <span class="card-text"><strong>{s.game}</strong><small>{date(s.created_ms)}{!s.ended?' · Recording now':''}</small></span>
            </button>
          {/each}
        </div>
        {#if app.libraryFilter==='recordings' && background.length>count}<button class="btn more-items" onclick={()=>count+=60}>Show more recordings</button>{/if}
      </section>
    {/if}
    {#if shown.length}
      <section aria-label="Saved media">
        <div class="section-head"><h2>{app.libraryFilter==='clips'?'Saved clips':app.libraryFilter==='screenshots'?'Screenshots':app.libraryFilter==='recordings'?'Saved recordings':'Saved media'}</h2><span class="permanent">Permanent</span></div>
        <div class="grid">
          {#each shown.slice(0,count) as c (c.path)}
            <article class="saved-card" class:open={menu===c.path}>
              <div class="cover">
                <button class="tile" aria-label={(c.kind==='screenshot'?'Open ':'Play ')+c.name} onfocus={()=>c.kind!=='screenshot'&&requestClipInfo(c.path)} onpointerenter={()=>c.kind!=='screenshot'&&requestClipInfo(c.path)} onclick={e=>openMedia({kind:'saved',item:c},e.currentTarget)}>
                  {#if c.kind==='screenshot'}<img src={convertFileSrc(c.path)} alt="" loading="lazy"/>{:else}<VideoPreview path={c.path} bytes={c.bytes} modified={c.modified_ms}/>{/if}
                  <span class="tile-play"><Icon name={c.kind==='screenshot'?'search':'play'} size={22}/></span>
                </button>
                <span class="tag"><Icon name={c.kind==='screenshot'?'camera':c.kind==='session'?'record':'scissors'} size={14}/>{mediaLabel(c.kind)}</span>
                {#if app.clipInfo[c.path]?.duration}<span class="duration">{duration(app.clipInfo[c.path].duration!)}</span>{/if}
                <button class="more" aria-label={'More actions for '+c.name} aria-expanded={menu===c.path} onclick={()=>{menu=menu===c.path?null:c.path;confirmDelete=null;}}><Icon name="more" size={17}/></button>
                {#if menu===c.path}<div class="menu">
                  <button onclick={()=>{revealPath(c.path);menu=null;}}>Show in folder</button>
                  <button onclick={()=>{openPath(c.path);menu=null;}}>Open externally</button>
                  <button onclick={()=>{copyPath(c.path);menu=null;}}>Copy path</button>
                  {#if c.kind!=='screenshot'}<button disabled={app.stemsBusy[c.path]} onclick={()=>{if(c.has_stems)revealPath(c.path.replace(/\.mp4$/i,'_Audio\\manifest.json'));else stems(c);menu=null;}}>{c.has_stems?'Show editor audio files':'Export editor audio files'}</button>{/if}
                  <button onclick={()=>{renaming=c.path;menu=null;}}>Rename</button>
                  {#if confirmDelete===c.path}<button class="danger" onclick={()=>remove(c)}>Move to Recycle Bin</button>{:else}<button class="danger" onclick={()=>confirmDelete=c.path}>Delete…</button>{/if}
                </div>{/if}
              </div>
              <div class="card-text">
                {#if renaming===c.path}
                  <!-- svelte-ignore a11y_autofocus -->
                  <input class="input" aria-label="New name" autofocus value={title(c)} onblur={e=>{if(renaming===c.path)rename(c,e.currentTarget.value);}} onkeydown={e=>{if(e.key==='Enter')e.currentTarget.blur();if(e.key==='Escape'){e.stopPropagation();renaming=null;}}}/>
                {:else}<strong title={title(c)}>{cardTitle(c)}</strong>{/if}
                <small>{cardTitle(c)===c.game?'':c.game+' · '}{date(c.modified_ms)}</small>
              </div>
            </article>
          {/each}
        </div>
        {#if shown.length>count}<button class="btn more-items" onclick={()=>count+=60}>Show more</button>{/if}
      </section>
    {/if}
    {#if !shown.length && (!showBackground || !background.length)}
      <div class="empty"><Icon name="library" size={34}/><h2>{!loaded || (showBackground&&!app.history)?'Loading your Library…':game||query?'Nothing matches':'Nothing here yet'}</h2><p>{game||query?'Try another game or search.':'Your recordings and screenshots will appear here after you play.'}</p>{#if game||query}<button class="btn" onclick={()=>{game='';query='';}}>Clear filters</button>{/if}</div>
    {/if}
  {/if}
</div>

<style>
  .library{display:flex;flex-direction:column;gap:22px;min-width:0}.heading,.toolbar,.section-head,.viewer-header{display:flex;align-items:center;gap:12px}.heading{justify-content:space-between}.heading h1{font-size:30px;letter-spacing:-.025em;line-height:1.2}.heading p,.viewer-title p{color:var(--text-2);margin-top:3px;font-size:13px}.toolbar{justify-content:space-between;flex-wrap:wrap;padding-bottom:14px;border-bottom:1px solid var(--line)}.filters{display:flex;gap:8px;min-width:0}.filters select{max-width:190px}.search{display:flex;align-items:center;position:relative;color:var(--text-2)}.search :global(svg){position:absolute;left:10px;pointer-events:none}.search input{padding-left:34px;width:175px}.section-head{margin-bottom:10px}.section-head h2{font-size:16px;font-weight:650}.section-head .link{margin-left:auto}.temp,.permanent{font-size:11px;font-weight:600;border-radius:4px;padding:2px 7px}.temp{color:var(--accent);background:var(--accent-soft)}.permanent{color:var(--text-2);background:var(--surface-2)}
  .grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(188px,1fr));gap:12px}.recording-card,.saved-card{min-width:0;border:1px solid var(--line);border-radius:9px;background:var(--surface);color:var(--text);text-align:left;position:relative}.recording-card{padding:0;cursor:pointer}.recording-card:hover,.recording-card:focus-visible,.saved-card:hover{border-color:var(--accent)}.cover{position:relative;aspect-ratio:16/9;display:block;background:var(--surface-2);border-radius:8px}.cover>img,.tile>img{width:100%;height:100%;object-fit:cover;position:absolute;inset:0;border-radius:inherit}.fallback{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;gap:12px;padding:12px;color:#e3edf9;border-radius:inherit;background:radial-gradient(ellipse at 85% 10%,hsl(var(--cover-hue) 55% 35%),transparent 67%),linear-gradient(125deg,hsl(var(--cover-hue) 40% 23%),#111927 75%)}.fallback b{font-size:clamp(13px,1.3vw,20px);overflow:hidden;text-overflow:ellipsis}.tag,.duration{position:absolute;top:7px;display:flex;align-items:center;gap:5px;padding:3px 6px;border-radius:4px;background:#10151de8;color:#f0f5fc;font-size:10px;pointer-events:none;white-space:nowrap}.tag{left:7px}.recording-card .tag{color:#a9d0ff}.duration{right:7px;font-variant-numeric:tabular-nums}.card-text{position:absolute;inset:auto 0 0;display:flex;flex-direction:column;gap:2px;padding:21px 10px 9px;min-width:0;color:#f0f5fc;background:linear-gradient(transparent,#0c111ae8 45%,#0c111a);border-radius:0 0 8px 8px;opacity:0;pointer-events:none;transition:opacity 110ms ease}.recording-card:is(:hover,:focus-visible) .card-text,.saved-card:is(:hover,:focus-within,.open) .card-text{opacity:1}.saved-card .tag,.saved-card .duration{opacity:0}.saved-card:is(:hover,:focus-within,.open) :is(.tag,.duration){opacity:1}.saved-card .duration{right:40px}.card-text strong{font-size:13px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.card-text small{font-size:11px;color:#c4ceda;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.card-text input{height:26px;width:100%;pointer-events:auto}
  .tile{position:absolute;inset:0;border:0;padding:0;border-radius:inherit;background:transparent;cursor:pointer;overflow:hidden}.tile-play{position:absolute;inset:0;display:grid;place-items:center;background:#0005;color:white;opacity:0}.tile:hover .tile-play,.tile:focus-visible .tile-play{opacity:1}.more{position:absolute;right:6px;top:6px;display:grid;place-items:center;width:28px;height:28px;border:0;border-radius:5px;background:#10151de8;color:white;cursor:pointer;opacity:0}.saved-card:hover .more,.saved-card:focus-within .more,.saved-card.open .more{opacity:1}.menu{position:absolute;z-index:10;right:6px;top:39px;min-width:180px;display:flex;flex-direction:column;padding:4px;background:var(--surface-3);border:1px solid var(--line-strong);border-radius:7px;box-shadow:var(--shadow-pop)}.menu button{border:0;padding:7px 9px;border-radius:4px;background:none;color:var(--text);text-align:left;cursor:pointer}.menu button:hover{background:var(--surface-2)}.menu .danger{color:var(--danger)}.more-items{margin-top:12px}
  .empty{display:grid;justify-items:center;gap:12px;padding:54px 16px;color:var(--text-2);text-align:center}.empty h2{font-size:18px;color:var(--text)}.viewer{display:flex;flex-direction:column;gap:0;min-height:450px;height:100vh}.viewer-header{display:grid;grid-template-columns:1fr minmax(0,2fr) 1fr;min-height:48px;padding:0 12px;border-bottom:1px solid var(--line);background:var(--chrome)}.viewer-header>button:first-child{justify-self:start}.viewer-header>button:last-child:not(:first-child){justify-self:end}.viewer-title{min-width:0;text-align:center}.viewer-title h1{font-size:15px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.viewer-title p{margin:0;font-size:12px}.saved-stage{flex:1;min-height:0;display:grid;place-items:center;background:#080a0e;overflow:hidden}.saved-stage video,.saved-stage img{width:100%;height:100%;min-height:0;object-fit:contain}
  @media(max-width:850px){.library{gap:17px}.heading h1{font-size:26px}.grid{grid-template-columns:repeat(auto-fill,minmax(164px,1fr));gap:9px}.search input{width:145px}}
  @media(prefers-reduced-motion:reduce){.card-text{transition:none}}
</style>
