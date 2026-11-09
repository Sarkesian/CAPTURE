<script lang="ts">
  // Library: Clips | Sessions | Screenshots, straight from the capture folder (no private index).
  import { onMount, untrack } from 'svelte';
  import { convertFileSrc, invoke } from '@tauri-apps/api/core';
  import {
    app, engine, openPath, revealPath, copyPath, openCaptureFolder, deleteClip, requestClipInfo, toast, fmtBytes, fmtDuration,
  } from '../lib/store.svelte';
  import type { ClipEntry } from '../lib/types';
  import Segmented from '../components/Segmented.svelte';
  import Icon from '../components/Icon.svelte';
  import VideoPreview from '../components/VideoPreview.svelte';

  type Kind = 'clips' | 'sessions' | 'screenshots';
  type Sort = 'new' | 'old' | 'big' | 'name';
  let kind = $state<Kind>('clips');
  let items = $state<ClipEntry[]>([]);
  let loaded = $state(false);
  let query = $state('');
  let game = $state('');
  let sort = $state<Sort>('new');
  let menu = $state<string | null>(null);
  let confirmDelete = $state<string | null>(null);
  let renaming = $state<string | null>(null);

  // Only the newest request may fill the list: a slow answer for the previous tab must not
  // replace the current one.
  let req = 0;
  async function load() {
    const k = kind, mine = ++req;
    try {
      const r = await invoke<ClipEntry[]>('list_clips', { kind: k });
      if (mine === req) items = r;
    } catch (e) { if (mine === req) toast(String(e), 'error'); }
    if (mine === req) loaded = true;
  }
  onMount(() => {
    const close = (ev: Event) => { if (!(ev.target as HTMLElement).closest?.('.menu, .more')) { menu = null; confirmDelete = null; } };
    const esc = (ev: KeyboardEvent) => { if (ev.key === 'Escape') { menu = null; confirmDelete = null; renaming = null; } };
    window.addEventListener('pointerdown', close);
    window.addEventListener('keydown', esc);
    return () => { window.removeEventListener('pointerdown', close); window.removeEventListener('keydown', esc); };
  });
  $effect(() => { kind; loaded = false; untrack(load); });
  // Refresh when files change (saves, editor audio files, screenshots).
  $effect(() => { app.libraryVersion; untrack(load); });

  const games = $derived([...new Set(items.map(c => c.game))].sort((a, b) => a.localeCompare(b)));
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = items.filter(c => (!game || c.game === game) && (c.name + ' ' + c.game).toLowerCase().includes(q));
    const cmp: Record<Sort, (a: ClipEntry, b: ClipEntry) => number> = {
      new: (a, b) => b.modified_ms - a.modified_ms,
      old: (a, b) => a.modified_ms - b.modified_ms,
      big: (a, b) => b.bytes - a.bytes,
      name: (a, b) => a.name.localeCompare(b.name),
    };
    return list.sort(cmp[sort]);
  });
  const total = $derived(shown.reduce((n, c) => n + c.bytes, 0));
  $effect(() => { for (const c of shown.slice(0, 80)) if (c.kind !== 'screenshot') requestClipInfo(c.path); });
  const fmtDate = (ms: number) => new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });

  async function rename(c: ClipEntry, name: string) {
    renaming = null;
    if (!name.trim() || name === c.name.replace(/\.(mp4|png)$/i, '')) return;
    try { await invoke('rename_clip', { path: c.path, name }); toast('Renamed', 'ok', 2000); load(); } catch (e) { toast(String(e), 'error'); }
  }
  async function stems(c: ClipEntry) {
    app.stemsBusy[c.path] = true;
    await engine('export_stems', { path: c.path });
  }
  async function remove(c: ClipEntry) {
    menu = null; confirmDelete = null;
    await deleteClip(c.path);
    load();
  }
  const noun = $derived(kind === 'clips' ? 'clip' : kind === 'sessions' ? 'session' : 'screenshot');
</script>

<div class="page">
  <header class="row wrap top">
    <h1 class="grow">Library</h1>
    <Segmented label="Library section" value={kind} onchange={v => { kind = v; game = ''; }}
      options={[{ value: 'clips', label: 'Clips' }, { value: 'sessions', label: 'Sessions' }, { value: 'screenshots', label: 'Screenshots' }]} />
  </header>

  <div class="row wrap tools">
    <label class="search grow">
      <Icon name="search" size={16} />
      <input class="input" type="search" placeholder="Search by name or game" aria-label="Search" bind:value={query} />
    </label>
    <select class="select" aria-label="Filter by game" bind:value={game}>
      <option value="">All games</option>
      {#each games as g (g)}<option value={g}>{g}</option>{/each}
    </select>
    <select class="select" aria-label="Sort" bind:value={sort}>
      <option value="new">Newest first</option>
      <option value="old">Oldest first</option>
      <option value="big">Largest first</option>
      <option value="name">Name</option>
    </select>
    <button class="btn icon ghost" title="Refresh" aria-label="Refresh" onclick={load}><Icon name="refresh" size={16} /></button>
    <button class="btn" onclick={openCaptureFolder}><Icon name="folder" size={16} />Open folder</button>
  </div>

  {#if !loaded}
    <div class="grid">{#each Array(6) as _, i (i)}<div class="item skel"><div class="thumb"></div><div class="line"></div></div>{/each}</div>
  {:else if items.length === 0}
    <div class="empty">
      <Icon name={kind === 'screenshots' ? 'camera' : kind === 'sessions' ? 'record' : 'library'} size={32} />
      <h2>No {noun}s yet</h2>
      <p class="muted">
        {#if kind === 'clips'}Turn on recording and press <kbd>{app.settings?.save_hotkey || 'your Save Replay key'}</kbd> during a game.
        {:else if kind === 'sessions'}Use <b>Record</b> on Home or in the overlay for a full-length recording.
        {:else}Press <kbd>{app.settings?.hotkeys.screenshot || 'your screenshot key'}</kbd> during a game.{/if}
      </p>
    </div>
  {:else if shown.length === 0}
    <div class="empty">
      <Icon name="search" size={28} />
      <h2>Nothing matches</h2>
      <button class="btn" onclick={() => { query = ''; game = ''; }}>Clear filters</button>
    </div>
  {:else}
    <p class="faint xs count">{shown.length} {noun}{shown.length === 1 ? '' : 's'} · {fmtBytes(total)}</p>
    <div class="grid">
      {#each shown as c (c.path)}
        {@const info = app.clipInfo[c.path]}
        <article class="item" class:open={menu === c.path}>
          <div class="thumb">
            <button class="tbtn" aria-label="{c.kind === 'screenshot' ? 'Open' : 'Play'} {c.name}" onclick={() => openPath(c.path)}>
              {#if c.kind === 'screenshot'}<img src={convertFileSrc(c.path)} alt="" loading="lazy" />
              {:else}<VideoPreview path={c.path} bytes={c.bytes} modified={c.modified_ms} />{/if}
              <span class="play" class:video={c.kind !== 'screenshot'}><Icon name={c.kind === 'screenshot' ? 'search' : 'play'} size={20} /></span>
            </button>
            {#if info?.duration}<span class="dur">{fmtDuration(info.duration)}</span>{/if}
            <div class="hover">
              <button class="chip" title="Show in folder" aria-label="Show {c.name} in folder" onclick={() => revealPath(c.path)}><Icon name="folder" size={15} /></button>
              <button class="chip more" title="More actions" aria-label="More actions for {c.name}" aria-haspopup="menu" aria-expanded={menu === c.path}
                onclick={() => { menu = menu === c.path ? null : c.path; confirmDelete = null; }}><Icon name="more" size={15} /></button>
            </div>
            {#if menu === c.path}
              <div class="menu" role="menu">
                <button role="menuitem" onclick={() => { openPath(c.path); menu = null; }}><Icon name={c.kind === 'screenshot' ? 'search' : 'play'} size={15} />{c.kind === 'screenshot' ? 'Open' : 'Play'}</button>
                <button role="menuitem" onclick={() => { copyPath(c.path); menu = null; }}><Icon name="copy" size={15} />Copy path</button>
                {#if c.kind !== 'screenshot'}
                  <button role="menuitem" disabled={app.stemsBusy[c.path]} title="One WAV per audio track, in sync, for editors that can't use MP4 audio tracks"
                    onclick={() => { if (c.has_stems) revealPath(c.path.replace(/\.mp4$/i, '_Audio\\manifest.json')); else stems(c); menu = null; }}>
                    <Icon name="audio" size={15} />{app.stemsBusy[c.path] ? 'Exporting audio files…' : c.has_stems ? 'Show editor audio files' : 'Export editor audio files'}</button>
                {/if}
                <button role="menuitem" onclick={() => { renaming = c.path; menu = null; }}><Icon name="pencil" size={15} />Rename</button>
                <div class="div"></div>
                {#if confirmDelete === c.path}
                  <button role="menuitem" class="danger" onclick={() => remove(c)}><Icon name="trash" size={15} />Move to Recycle Bin</button>
                {:else}
                  <button role="menuitem" class="danger" onclick={() => (confirmDelete = c.path)}><Icon name="trash" size={15} />Delete…</button>
                {/if}
              </div>
            {/if}
          </div>
          {#if renaming === c.path}
            <!-- svelte-ignore a11y_autofocus -->
            <input class="input rn" type="text" value={c.name.replace(/\.(mp4|png)$/i, '')} aria-label="New name" autofocus
              onblur={ev => rename(c, (ev.target as HTMLInputElement).value)}
              onkeydown={ev => { if (ev.key === 'Enter') (ev.target as HTMLInputElement).blur(); if (ev.key === 'Escape') { ev.stopPropagation(); renaming = null; } }} />
          {:else}
            <div class="name ellipsis" title={c.name}>{c.name.replace(/\.(mp4|png)$/i, '')}</div>
          {/if}
          <div class="meta faint xs row">
            <span class="ellipsis">{c.game}</span><span>·</span><span>{fmtDate(c.modified_ms)}</span><span>·</span><span>{fmtBytes(c.bytes)}</span>
            {#if c.has_markers}<span class="badge" title="Has markers"><Icon name="marker" size={12} /></span>{/if}
            {#if c.has_stems}<span class="badge" title="Has editor audio files"><Icon name="audio" size={12} /></span>{/if}
          </div>
        </article>
      {/each}
    </div>
  {/if}
</div>

<style>
  .page { display: flex; flex-direction: column; gap: var(--s4); }
  h1 { font-size: var(--fs-xl); font-weight: 650; margin: 0; }
  .top { gap: var(--s3); }
  .tools { gap: var(--s2); }
  .search { position: relative; display: flex; align-items: center; min-width: 200px; color: var(--text-3); }
  .search :global(svg) { position: absolute; left: 10px; pointer-events: none; }
  .search .input { width: 100%; padding-left: 34px; }
  .count { margin: 0; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(232px, 1fr)); gap: var(--s5) var(--s4); }
  .item { display: flex; flex-direction: column; gap: var(--s1); min-width: 0; }
  .thumb { position: relative; aspect-ratio: 16 / 9; border-radius: var(--r-md); background: var(--surface-2); }
  .tbtn { position: absolute; inset: 0; padding: 0; border: none; background: none; border-radius: var(--r-md); overflow: hidden; cursor: pointer; }
  .tbtn img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .play { position: absolute; inset: 0; display: grid; place-items: center; color: #fff; background: rgba(0, 0, 0, 0.35); opacity: 0; transition: opacity var(--t-fast) var(--ease); }
  .dur { position: absolute; right: 6px; bottom: 6px; padding: 0 6px; border-radius: 4px; background: rgba(0, 0, 0, 0.72); color: #fff; font-size: var(--fs-xs); pointer-events: none; }
  .hover { position: absolute; top: 6px; right: 6px; display: flex; gap: 4px; opacity: 0; transition: opacity var(--t-fast) var(--ease); }
  .item:hover .hover, .item:focus-within .hover, .item.open .hover { opacity: 1; }
  .item:hover .play, .tbtn:focus-visible .play { opacity: 1; }
  .play.video { background: rgba(0, 0, 0, 0.18); }
  .chip { width: 28px; height: 28px; display: grid; place-items: center; border: none; border-radius: var(--r-sm); background: rgba(0, 0, 0, 0.65); color: #fff; cursor: pointer; }
  .chip:hover { background: rgba(0, 0, 0, 0.85); }
  .menu { position: absolute; top: 38px; right: 6px; z-index: 5; min-width: 220px; padding: var(--s1); border-radius: var(--r-md);
          background: var(--surface-3); border: 1px solid var(--line-strong); box-shadow: var(--shadow-pop); display: flex; flex-direction: column; }
  .menu button { display: flex; align-items: center; gap: var(--s2); height: 32px; padding: 0 var(--s3); border: none; border-radius: var(--r-sm);
                 background: none; color: var(--text); font-size: var(--fs-sm); text-align: left; cursor: pointer; }
  .menu button:hover:not(:disabled), .menu button:focus-visible { background: var(--surface-2); }
  .menu button:disabled { color: var(--text-3); cursor: default; }
  .menu .danger { color: var(--danger); }
  .menu .div { height: 1px; background: var(--line); margin: var(--s1) 0; }
  .name { font-weight: 550; margin-top: var(--s1); }
  .rn { height: var(--h-sm); margin-top: var(--s1); }
  .meta { gap: var(--s1); min-width: 0; }
  .meta > :first-child { min-width: 0; flex: 0 1 auto; }
  .badge { display: inline-grid; place-items: center; color: var(--text-2); }
  .empty { display: flex; flex-direction: column; align-items: center; gap: var(--s3); padding: var(--s10) var(--s6); text-align: center;
           border: 1px dashed var(--line-strong); border-radius: var(--r-lg); color: var(--text-3); }
  .empty h2 { margin: 0; font-size: var(--fs-lg); color: var(--text); }
  .empty p { margin: 0; max-width: 48ch; }
  .skel .thumb { animation: shimmer 1.2s ease-in-out infinite; }
  .skel .line { height: 12px; width: 60%; border-radius: 4px; background: var(--surface-2); margin-top: var(--s2); }
  @keyframes shimmer { 50% { opacity: 0.55; } }
  @media (prefers-reduced-motion: reduce) { .skel .thumb { animation: none; } }
</style>
