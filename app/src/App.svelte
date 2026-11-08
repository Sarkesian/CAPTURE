<script lang="ts">
  import { onMount } from 'svelte';
  import { app, init, refreshClips } from './lib/store.svelte';
  import CaptureTab from './tabs/CaptureTab.svelte';
  import LibraryTab from './tabs/LibraryTab.svelte';
  import SettingsTab from './tabs/SettingsTab.svelte';

  type Tab = 'capture' | 'library' | 'settings';
  const tab = $derived(app.tab);
  let failed = $state<string | null>(null);
  const tabs: { id: Tab; label: string }[] = [
    { id: 'capture', label: 'Capture' },
    { id: 'library', label: 'Library' },
    { id: 'settings', label: 'Settings' },
  ];

  onMount(() => { init().catch(e => (failed = String(e))); });

  function select(t: Tab) {
    app.tab = t;
    if (t === 'library') refreshClips();
  }

  function keynav(e: KeyboardEvent, i: number) {
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      const n = (i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length;
      select(tabs[n].id);
      document.getElementById(`tab-${tabs[n].id}`)?.focus();
    }
  }
</script>

<div class="shell">
  <header>
    <div class="brand"><span class="dot" aria-hidden="true"></span>CAPTURE</div>
    <div role="tablist" aria-label="Sections" class="tabs">
      {#each tabs as t, i (t.id)}
        <button id="tab-{t.id}" role="tab" aria-selected={tab === t.id} aria-controls="panel" tabindex={tab === t.id ? 0 : -1}
          class:active={tab === t.id} onclick={() => select(t.id)} onkeydown={e => keynav(e, i)}>{t.label}</button>
      {/each}
    </div>
  </header>

  <main>
    <div id="panel" role="tabpanel" tabindex="-1" aria-labelledby="tab-{tab}">
    {#if failed}
      <div class="card">CAPTURE could not load its settings: {failed}</div>
    {:else if !app.settings}
      <p class="muted">Loading…</p>
    {:else if tab === 'capture'}
      <CaptureTab />
    {:else if tab === 'library'}
      <LibraryTab />
    {:else}
      <SettingsTab />
    {/if}
    </div>
  </main>

  <div class="toasts" aria-live="polite">
    {#each app.toasts as t (t.id)}
      <div class="toast {t.kind}">{t.text}</div>
    {/each}
  </div>
</div>

<style>
  .shell { min-height: 100vh; display: flex; flex-direction: column; }
  header { display: flex; align-items: center; gap: 28px; padding: 14px 24px; border-bottom: 1px solid var(--line); background: #14161b; }
  .brand { font-weight: 700; letter-spacing: 0.12em; display: flex; align-items: center; gap: 9px; }
  .dot { width: 12px; height: 12px; border-radius: 50%; background: var(--accent); }
  .tabs { display: flex; gap: 4px; }
  .tabs button { background: none; border: none; color: var(--muted); padding: 8px 16px; border-radius: 8px; font-weight: 500; }
  .tabs button.active { color: var(--text); background: var(--panel-2); }
  main { padding: 22px 24px 40px; max-width: 1040px; width: 100%; margin: 0 auto; }
  .toasts { position: fixed; right: 18px; bottom: 18px; display: flex; flex-direction: column; gap: 8px; max-width: 460px; }
  .toast { background: #262b35; border: 1px solid var(--line); border-left: 4px solid var(--focus); padding: 10px 14px; border-radius: 8px; }
  .toast.ok { border-left-color: var(--ok); }
  .toast.error { border-left-color: var(--accent); }
</style>
