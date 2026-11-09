<script lang="ts">
  // The most recent clips (play tile + duration). Used on Home.
  import { app, requestClipInfo, openPath, revealPath, fmtDuration } from '../lib/store.svelte';
  import Icon from './Icon.svelte';
  import VideoPreview from './VideoPreview.svelte';
  import type { ClipEntry } from '../lib/types';

  let { count = 4, onplay }: { count?: number; onplay?: (c: ClipEntry) => void } = $props();
  const clips = $derived(app.clips.slice(0, count));
  $effect(() => { for (const c of clips) requestClipInfo(c.path); });
  const ago = (ms: number) => {
    const m = Math.round((Date.now() - ms) / 60000);
    if (m < 1) return 'just now';
    if (m < 60) return `${m} min ago`;
    const h = Math.round(m / 60);
    if (h < 24) return `${h} h ago`;
    return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  };
</script>

{#if clips.length === 0}
  <p class="empty muted small">No clips yet. Saved replays appear here.</p>
{:else}
  <div class="strip" style:--n={count}>
    {#each clips as c (c.path)}
      {@const info = app.clipInfo[c.path]}
      <article class="clip">
        <button class="thumb" title="Play {c.name}" onclick={() => (onplay ? onplay(c) : openPath(c.path))}>
          <VideoPreview path={c.path} bytes={c.bytes} modified={c.modified_ms} />
          <span class="play"><Icon name="play" size={16} /></span>
          {#if info?.duration}<span class="dur">{fmtDuration(info.duration)}</span>{/if}
        </button>
        <div class="meta row">
          <div class="grow">
            <div class="ellipsis small" title={c.game}>{c.game}</div>
            <div class="faint xs">{ago(c.modified_ms)}</div>
          </div>
          <button class="btn ghost sm icon" title="Show in folder" aria-label="Show {c.name} in folder" onclick={() => revealPath(c.path)}><Icon name="folder" size={16} /></button>
        </div>
      </article>
    {/each}
  </div>
{/if}

<style>
  .strip { display: grid; grid-template-columns: repeat(var(--n), minmax(0, 1fr)); gap: var(--s4); }
  .clip { min-width: 0; display: flex; flex-direction: column; gap: var(--s2); }
  .thumb { position: relative; aspect-ratio: 16 / 9; padding: 0; border: none; border-radius: var(--r-md); overflow: hidden; background: var(--surface-2); cursor: pointer; }
  .play { position: absolute; inset: 0; display: grid; place-items: center; color: var(--text-2); transition: color var(--t-fast) var(--ease); }
  .thumb:hover .play, .thumb:focus-visible .play { color: var(--text); }
  .dur { position: absolute; right: 6px; bottom: 6px; padding: 0 6px; border-radius: 4px; background: rgba(0, 0, 0, 0.7); color: #fff; font-size: var(--fs-xs); }
  .meta { gap: var(--s1); }
  .empty { padding: var(--s3) 0; }
  @media (max-width: 760px) { .strip { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
</style>
