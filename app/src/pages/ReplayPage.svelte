<script lang="ts">
  // Replay: watch the recent buffer while recording continues, jump between markers, save a range.
  import { app, releaseReview, setReplay, fmtDuration } from '../lib/store.svelte';
  import ReviewPanel from '../overlay/ReviewPanel.svelte';
  import Icon from '../components/Icon.svelte';

  let { onnavigate }: { onnavigate?: (p: 'home' | 'replay' | 'library' | 'settings') => void } = $props();
  const e = $derived(app.engine);
  const s = $derived(app.settings!);
  const live = $derived(e?.status === 'buffering');
  // Keep the snapshot only while this page is open.
  $effect(() => () => { if (app.review) releaseReview(); });
</script>

<div class="page">
  <header class="row">
    <div class="grow">
      <h1>Replay</h1>
      <p class="muted small">
        {#if live}Your last {fmtDuration(e!.buffered_secs)} of {e!.target_name}. Recording continues while you watch.
        {:else}Watch and clip your recent gameplay without stopping the recording.{/if}
      </p>
    </div>
  </header>

  {#if live}
    <ReviewPanel onclose={() => onnavigate?.('home')} />
  {:else}
    <div class="empty">
      <Icon name="replay" size={32} />
      {#if !s.replay_enabled}
        <h2>Recording is off</h2>
        <p class="muted">Turn recording on and your recent gameplay will be available here.</p>
        <button class="btn primary" onclick={() => setReplay(true)}>Turn on recording</button>
      {:else}
        <h2>Nothing to replay yet</h2>
        <p class="muted">{e?.attention ?? 'Start a game. Once CAPTURE is recording, the last few minutes appear here.'}</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .page { display: flex; flex-direction: column; gap: var(--s4); height: calc(100vh - var(--s6) - var(--s10)); min-height: 420px; }
  h1 { font-size: var(--fs-xl); font-weight: 650; margin: 0 0 var(--s1); }
  .empty { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--s3); text-align: center;
           border: 1px dashed var(--line-strong); border-radius: var(--r-lg); color: var(--text-3); padding: var(--s8); }
  .empty h2 { color: var(--text); font-size: var(--fs-lg); margin: 0; }
  .empty p { max-width: 46ch; margin: 0; }
</style>
