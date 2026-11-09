<script lang="ts">
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { videoPreview } from '../lib/videoPreview';
  let { path, bytes, modified }: { path: string; bytes: number; modified: number } = $props();
  let host: HTMLSpanElement;
  let image = $state('');
  let error = $state('');
  $effect(() => {
    const key = `${path}\0${bytes}\0${modified}`;
    const url = convertFileSrc(path);
    const abort = new AbortController();
    let started = false;
    image = ''; error = '';
    const observer = new IntersectionObserver(entries => {
      if (started || !entries.some(e => e.isIntersecting)) return;
      started = true; observer.disconnect();
      videoPreview(key, url, abort.signal).then(value => { if (!abort.signal.aborted) image = value; }, e => { if (!abort.signal.aborted) error = String(e.message || e); });
    }, { rootMargin: '160px' });
    observer.observe(host);
    return () => { abort.abort(); observer.disconnect(); };
  });
</script>

<span class="preview" bind:this={host}>
  {#if image}
    <img src={image} alt="" />
  {:else if error}
    <span class="state" title={error}>Preview unavailable · Play video</span>
  {:else}
    <span class="state" role="status">Loading preview…</span>
  {/if}
</span>

<style>
  .preview { position: absolute; inset: 0; display: grid; place-items: center; overflow: hidden; border-radius: inherit; }
  img { width: 100%; height: 100%; object-fit: contain; }
  .state { font-size: 11px; color: var(--text-2); padding: 12px; }
</style>
