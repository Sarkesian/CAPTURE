<script lang="ts" generics="T extends string | number">
  let { value, options, label, onchange, size = 'md' }: {
    value: T; options: { value: T; label: string }[]; label: string; onchange: (v: T) => void; size?: 'sm' | 'md';
  } = $props();

  function key(ev: KeyboardEvent, i: number) {
    if (ev.key !== 'ArrowRight' && ev.key !== 'ArrowLeft') return;
    ev.preventDefault();
    const n = (i + (ev.key === 'ArrowRight' ? 1 : options.length - 1)) % options.length;
    onchange(options[n].value);
    ((ev.currentTarget as HTMLElement).parentElement?.children[n] as HTMLElement | undefined)?.focus();
  }
</script>

<div class="seg {size}" role="radiogroup" aria-label={label}>
  {#each options as o, i (o.value)}
    <button type="button" role="radio" aria-checked={value === o.value} tabindex={value === o.value ? 0 : -1}
      class:active={value === o.value} onclick={() => onchange(o.value)} onkeydown={ev => key(ev, i)}>{o.label}</button>
  {/each}
</div>

<style>
  .seg { display: inline-flex; flex: none; padding: 2px; gap: 2px; border-radius: var(--r-md); background: var(--surface-2); border: 1px solid var(--line); }
  .seg button {
    height: calc(var(--h-md) - 6px); padding: 0 var(--s3); border: none; border-radius: calc(var(--r-md) - 2px);
    background: transparent; color: var(--text-2); font-size: var(--fs-sm); font-weight: 500; white-space: nowrap; cursor: pointer;
    transition: background var(--t-fast) var(--ease), color var(--t-fast) var(--ease);
  }
  .seg.sm button { height: calc(var(--h-sm) - 6px); padding: 0 var(--s2); font-size: var(--fs-xs); }
  .seg button:hover { color: var(--text); }
  .seg button.active { background: var(--surface); color: var(--text); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25), 0 0 0 1px var(--line-strong); }
</style>
