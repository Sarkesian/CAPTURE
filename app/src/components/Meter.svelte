<script lang="ts">
  // A real level meter (see lib/meters.ts). `source` is the engine track name, or 'mic_monitor'.
  // `state` other than 'live' shows a flat, labelled meter; it never animates fake activity.
  import { meter } from '../lib/meters';

  let { source, label, state = 'live', size = 'md' }: {
    source: string; label: string; state?: 'live' | 'off' | 'muted' | 'error' | 'idle'; size?: 'sm' | 'md';
  } = $props();
</script>

{#if state === 'live'}
  <div class="meter {size}" use:meter={source} role="img" aria-label={label}>
    <div class="peak"></div>
    <div class="rms"></div>
    <div class="hold"></div>
    <span class="clip" title="Clipping: this source hit full scale"></span>
  </div>
{:else}
  <div class="meter {size} flat" class:err={state === 'error'} role="img" aria-label="{label}: {state}"></div>
{/if}

<style>
  .meter {
    --rms: 0; --peak: 0; --hold: 0;
    position: relative; height: 6px; min-width: 80px; border-radius: 3px; overflow: hidden;
    background: var(--meter-bg);
  }
  .meter.sm { height: 4px; min-width: 60px; }
  .rms, .peak { position: absolute; inset: 0; transform-origin: left center; will-change: transform; }
  .peak { background: var(--meter-peak); transform: scaleX(var(--peak)); }
  .rms { background: var(--meter-rms); transform: scaleX(var(--rms)); }
  .hold { position: absolute; top: 0; bottom: 0; width: 2px; left: calc(var(--hold) * 100% - 2px); background: var(--meter-rms); opacity: 0.9; }
  .meter:global([data-hot='1']) .hold { background: var(--meter-hot); }
  .clip { position: absolute; right: 0; top: 0; bottom: 0; width: 4px; background: var(--meter-clip); opacity: 0; }
  .meter:global([data-clip='1']) .clip { opacity: 1; }
  .flat { opacity: 0.55; }
  .flat.err { background: var(--warn-soft); opacity: 1; }
</style>
