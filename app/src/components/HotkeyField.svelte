<script lang="ts">
  // Records a key combination. The engine validates and registers it; its result
  // (registered / conflict message) is shown by the parent from engine state.
  let { value, onchange }: { value: string; onchange: (v: string) => void } = $props();
  let recording = $state(false);

  const named: Record<string, string> = {
    PrintScreen: 'PrintScreen', Insert: 'Insert', Delete: 'Delete', Home: 'Home', End: 'End',
    PageUp: 'PageUp', PageDown: 'PageDown', Pause: 'Pause', ScrollLock: 'ScrollLock',
  };

  function onkeydown(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    if (e.key === 'Escape') { recording = false; return; }
    let key: string | null = null;
    if (/^F([1-9]|1\d|2[0-4])$/.test(e.key)) key = e.key;
    else if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3);
    else if (/^Digit\d$/.test(e.code)) key = e.code.slice(5);
    else if (named[e.key]) key = named[e.key];
    if (!key) return; // wait for a non-modifier key
    const parts = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Win', key].filter(Boolean);
    recording = false;
    onchange(parts.join('+'));
  }
</script>

<div class="row">
  <button type="button" class="key" class:rec={recording} onclick={() => (recording = !recording)} {onkeydown} aria-label="Save hotkey: {value || 'none'}. Activate to record a new one.">
    {recording ? 'Press keys… (Esc to cancel)' : value || 'Not set'}
  </button>
  {#if value && !recording}
    <button type="button" class="link" onclick={() => onchange('')}>Clear</button>
  {/if}
</div>

<style>
  .key { font-family: "Cascadia Mono", Consolas, monospace; min-width: 150px; }
  .key.rec { border-color: var(--focus); color: var(--focus); }
</style>
