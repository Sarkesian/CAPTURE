<script lang="ts" generics="T extends string | number">
  let { value, options, label, onchange }: {
    value: T; options: { value: T; label: string }[]; label: string; onchange: (v: T) => void;
  } = $props();
</script>

<div class="seg" role="radiogroup" aria-label={label}>
  {#each options as o (o.value)}
    <button type="button" role="radio" aria-checked={value === o.value} class:active={value === o.value} onclick={() => onchange(o.value)}>
      {o.label}
    </button>
  {/each}
</div>

<style>
  .seg { display: inline-flex; background: var(--panel-2); border: 1px solid var(--line); border-radius: 9px; padding: 3px; gap: 2px; }
  .seg button { border: none; background: none; padding: 5px 14px; border-radius: 7px; color: var(--muted); }
  .seg button.active { background: #343a46; color: var(--text); }
</style>
