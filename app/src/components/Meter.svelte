<script lang="ts">
  // Shows a real peak value reported by the engine (0..1 linear). No synthetic animation.
  let { peak, label }: { peak: number; label: string } = $props();
  const db = $derived(peak > 0 ? 20 * Math.log10(peak) : -Infinity);
  const pct = $derived(Math.max(0, Math.min(100, ((db + 60) / 60) * 100)));
</script>

<div class="meter" role="meter" aria-label={label} aria-valuemin={-60} aria-valuemax={0} aria-valuenow={Number.isFinite(db) ? Math.round(db) : -60}>
  <div class="fill" class:hot={db > -3} style:width="{pct}%"></div>
</div>

<style>
  .meter { height: 8px; border-radius: 4px; background: #2a2f39; overflow: hidden; min-width: 120px; }
  .fill { height: 100%; background: var(--ok); transition: width 0.12s linear; }
  .fill.hot { background: var(--warn); }
</style>
