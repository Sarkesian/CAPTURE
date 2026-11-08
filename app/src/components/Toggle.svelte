<script lang="ts">
  let { checked, label, onchange, disabled = false, big = false }: {
    checked: boolean; label: string; onchange: (v: boolean) => void; disabled?: boolean; big?: boolean;
  } = $props();
</script>

<button
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  class="switch"
  class:on={checked}
  class:big
  {disabled}
  onclick={() => onchange(!checked)}
>
  <span class="knob"></span>
</button>

<style>
  .switch { width: 46px; height: 26px; padding: 0; border-radius: 999px; position: relative; background: #3a404c; border: none; flex: none; }
  .switch.big { width: 60px; height: 34px; }
  .switch.on { background: var(--accent); }
  .knob { position: absolute; top: 3px; left: 3px; width: 20px; height: 20px; border-radius: 50%; background: white; transition: transform 0.15s; }
  .big .knob { width: 28px; height: 28px; }
  .on .knob { transform: translateX(20px); }
  .big.on .knob { transform: translateX(26px); }
</style>
