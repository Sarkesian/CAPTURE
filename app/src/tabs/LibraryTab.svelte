<script lang="ts">
  // Library: Clips | Sessions | Screenshots, straight from the capture folder (no private index).
  import { onMount } from 'svelte';
  import { convertFileSrc, invoke } from '@tauri-apps/api/core';
  import {
    app, engine, openPath, revealPath, copyPath, openCaptureFolder, deleteClip, requestClipInfo, toast, fmtBytes, fmtDuration,
  } from '../lib/store.svelte';
  import type { ClipEntry } from '../lib/types';

  type Kind = 'clips' | 'sessions' | 'screenshots';
  let kind = $state<Kind>('clips');
  let items = $state<ClipEntry[]>([]);
  let query = $state('');
  let confirmDelete = $state<string | null>(null);
  let renaming = $state<string | null>(null);

  async function load() {
    try { items = await invoke<ClipEntry[]>('list_clips', { kind }); } catch (e) { toast(String(e), 'error'); }
  }
  onMount(load);
  $effect(() => { kind; load(); });
  // Refresh when files change (saves, editor audio files, screenshots).
  $effect(() => { app.libraryVersion; load(); });

  const filtered = $derived(items.filter(c => (c.name + ' ' + c.game).toLowerCase().includes(query.trim().toLowerCase())));
  const groups = $derived.by(() => {
    const m = new Map<string, ClipEntry[]>();
    for (const c of filtered) m.set(c.game, [...(m.get(c.game) ?? []), c]);
    return [...m.entries()];
  });
  $effect(() => { for (const c of filtered.slice(0, 60)) if (c.kind !== 'screenshot') requestClipInfo(c.path); });
  const fmtDate = (ms: number) => new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });

  async function rename(c: ClipEntry, name: string) {
    renaming = null;
    try { await invoke('rename_clip', { path: c.path, name }); toast('Renamed', 'ok', 2000); load(); } catch (e) { toast(String(e), 'error'); }
  }
  async function stems(c: ClipEntry) {
    app.stemsBusy[c.path] = true;
    await engine('export_stems', { path: c.path });
  }
</script>

<div class="row" style="margin-bottom:14px">
  <div class="seg" role="tablist" aria-label="Library section">
    {#each [['clips', 'Clips'], ['sessions', 'Sessions'], ['screenshots', 'Screenshots']] as [k, label] (k)}
      <button role="tab" aria-selected={kind === k} class:active={kind === k} onclick={() => (kind = k as Kind)}>{label}</button>
    {/each}
  </div>
  <input type="text" class="grow" placeholder="Search by name or game" aria-label="Search" bind:value={query} />
  <button onclick={load}>Refresh</button>
  <button onclick={openCaptureFolder}>Open folder</button>
</div>

{#if items.length === 0}
  <div class="card muted">
    {#if kind === 'clips'}No clips yet. Turn on recording and press your Save Replay hotkey during a game.
    {:else if kind === 'sessions'}No sessions yet. Use Start Recording in the overlay for a full-length recording.
    {:else}No screenshots yet. Press your screenshot hotkey ({app.settings?.hotkeys.screenshot || 'set one in Settings'}) during a game.{/if}
  </div>
{:else if groups.length === 0}
  <div class="card muted">Nothing matches “{query}”.</div>
{/if}

{#each groups as [game, list] (game)}
  <section class="card group">
    <h2>{game} <span class="muted">· {list.length}</span></h2>
    <div class="grid">
      {#each list as c (c.path)}
        {@const info = app.clipInfo[c.path]}
        <article class="item">
          <button class="thumb" aria-label="Open {c.name}" onclick={() => openPath(c.path)}>
            {#if c.kind === 'screenshot'}<img src={convertFileSrc(c.path)} alt="" loading="lazy" />
            {:else if info?.thumb}<img src={convertFileSrc(info.thumb)} alt="" loading="lazy" />
            {:else}<span class="muted small">…</span>{/if}
            {#if info?.duration}<span class="dur">{fmtDuration(info.duration)}</span>{/if}
          </button>
          {#if renaming === c.path}
            <input type="text" value={c.name.replace(/\.(mp4|png)$/i, '')} aria-label="New name"
              onkeydown={ev => { if (ev.key === 'Enter') rename(c, (ev.target as HTMLInputElement).value); if (ev.key === 'Escape') renaming = null; }} />
          {:else}
            <div class="name ellipsis" title={c.name}>{c.name}</div>
          {/if}
          <div class="muted small">{fmtDate(c.modified_ms)} · {fmtBytes(c.bytes)}
            {#if c.has_markers} · markers{/if}{#if c.has_stems} · editor audio files{/if}</div>
          <div class="acts">
            <button onclick={() => openPath(c.path)}>{c.kind === 'screenshot' ? 'Open' : 'Play'}</button>
            <button onclick={() => revealPath(c.path)}>Show in folder</button>
            <button onclick={() => copyPath(c.path)}>Copy path</button>
            {#if c.kind !== 'screenshot'}
              <button disabled={app.stemsBusy[c.path]} title="Separate WAV per audio track, in sync, for editors that can't use MP4 audio tracks"
                onclick={() => (c.has_stems ? revealPath(c.path.replace(/\.mp4$/i, '_Audio\\manifest.json')) : stems(c))}>
                {app.stemsBusy[c.path] ? 'Exporting…' : c.has_stems ? 'Audio files' : 'Editor audio files'}</button>
            {/if}
            <button onclick={() => (renaming = c.path)}>Rename</button>
            {#if confirmDelete === c.path}
              <button class="danger" onclick={() => { deleteClip(c.path).then(load); confirmDelete = null; }}>Move to Recycle Bin?</button>
            {:else}
              <button onclick={() => (confirmDelete = c.path)}>Delete</button>
            {/if}
          </div>
        </article>
      {/each}
    </div>
  </section>
{/each}

<style>
  .seg { display: inline-flex; background: var(--panel-2); border: 1px solid var(--line); border-radius: 9px; padding: 3px; gap: 2px; }
  .seg button { border: none; background: none; padding: 5px 14px; border-radius: 7px; color: var(--muted); }
  .seg button.active { background: #343a46; color: var(--text); }
  .group { margin-bottom: 14px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 14px; }
  .item { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
  .thumb { position: relative; padding: 0; height: 146px; overflow: hidden; display: grid; place-items: center; background: #000; }
  .thumb img { width: 100%; height: 100%; object-fit: cover; }
  .dur { position: absolute; right: 6px; bottom: 6px; background: #000c; font-size: 12px; padding: 1px 6px; border-radius: 4px; }
  .name { font-size: 14px; }
  .acts { display: flex; flex-wrap: wrap; gap: 5px; }
  .acts button { padding: 3px 8px; font-size: 12.5px; }
  .danger { border-color: var(--accent); color: var(--accent-2); }
</style>
