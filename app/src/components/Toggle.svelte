<script lang="ts">
  let { checked, label, onchange, disabled = false, big = false }: {
    checked: boolean; label: string; onchange: (v: boolean) => void; disabled?: boolean; big?: boolean;
  } = $props();
</script>

<button type="button" role="switch" aria-checked={checked} aria-label={label} class="switch" class:on={checked} class:big {disabled}
  onclick={() => onchange(!checked)}>
  <span class="knob"></span>
</button>

<style>
  .switch {
    --w: 40px; --h: 22px; --k: 16px;
    position: relative; flex: none; width: var(--w); height: var(--h); padding: 0; border-radius: 999px;
    border: 1px solid var(--line-strong); background: var(--surface-3); cursor: pointer;
    transition: background var(--t-fast) var(--ease), border-color var(--t-fast) var(--ease);
  }
  .switch.big { --w: 48px; --h: 26px; --k: 20px; }
  .switch:hover:not(:disabled) { border-color: var(--text-3); }
  .switch.on { background: var(--accent); border-color: var(--accent); }
  .switch:disabled { opacity: 0.45; cursor: default; }
  .knob {
    position: absolute; top: 50%; left: 2px; width: var(--k); height: var(--k); margin-top: calc(var(--k) / -2);
    border-radius: 50%; background: #fff; box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
    transition: transform var(--t-med) var(--ease);
  }
  .on .knob { transform: translateX(calc(var(--w) - var(--k) - 6px)); }
  .switch:active:not(:disabled) .knob { width: calc(var(--k) + 3px); }
</style>
