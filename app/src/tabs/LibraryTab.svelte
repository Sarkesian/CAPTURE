<script lang="ts">
  import { app, refreshClips, openPath, revealPath, copyPath, openCaptureFolder, fmtBytes } from '../lib/store.svelte';

  let query = $state('');
  const filtered = $derived(app.clips.filter(c => (c.name + ' ' + c.game).toLowerCase().includes(query.trim().toLowerCase())));
  const groups = $derived.by(() => {
    const m = new Map<string, typeof app.clips>();
    for (const c of filtered) m.set(c.game, [...(m.get(c.game) ?? []), c]);
    return [...m.entries()];
  });
  const fmtDate = (ms: number) => new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
</script>

<div class="row" style="margin-bottom:14px">
  <input type="text" class="grow" placeholder="Search clips and games" aria-label="Search clips" bind:value={query} />
  <button onclick={refreshClips}>Refresh</button>
  <button onclick={openCaptureFolder}>Open folder</button>
</div>

{#if app.clips.length === 0}
  <div class="card muted">No clips yet. Turn on instant replay on the Capture tab and press your save hotkey during a game.</div>
{:else if groups.length === 0}
  <div class="card muted">No clips match “{query}”.</div>
{/if}

{#each groups as [game, clips] (game)}
  <section class="card group">
    <h2>{game} <span class="muted">· {clips.length}</span></h2>
    <ul>
      {#each clips as c (c.path)}
        <li class="row">
          <div class="grow">
            <div class="ellipsis">{c.name}</div>
            <div class="muted small">{fmtDate(c.modified_ms)} · {fmtBytes(c.bytes)}</div>
          </div>
          <button onclick={() => openPath(c.path)}>Play</button>
          <button onclick={() => revealPath(c.path)}>Show in folder</button>
          <button onclick={() => copyPath(c.path)}>Copy path</button>
        </li>
      {/each}
    </ul>
  </section>
{/each}

<style>
  .group { margin-bottom: 14px; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 9px 0; border-top: 1px solid var(--line); }
  li:first-child { border-top: none; }
</style>
