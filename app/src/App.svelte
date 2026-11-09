<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { app, init, refreshClips, fmtDuration } from './lib/store.svelte';
  import Icon from './components/Icon.svelte';
  import HomePage from './pages/HomePage.svelte';
  import ReplayPage from './pages/ReplayPage.svelte';
  import LibraryTab from './tabs/LibraryTab.svelte';
  import SettingsTab from './tabs/SettingsTab.svelte';

  type Page = 'home' | 'replay' | 'library' | 'settings';
  const pages: { id: Page; label: string; icon: string }[] = [
    { id: 'home', label: 'Home', icon: 'home' },
    { id: 'replay', label: 'Replay', icon: 'replay' },
    { id: 'library', label: 'Library', icon: 'library' },
    { id: 'settings', label: 'Settings', icon: 'settings' },
  ];
  let failed = $state<string | null>(null);
  let main = $state<HTMLElement | null>(null);
  const scroll: Record<string, number> = {};
  const page = $derived<Page>((app.tab as string) === 'capture' ? 'home' : (app.tab as Page));

  onMount(() => { init().catch(e => (failed = String(e))); });

  async function go(p: Page) {
    if (main) scroll[page] = main.scrollTop;
    app.tab = p as typeof app.tab;
    if (p === 'library') refreshClips();
    await tick();
    if (main) main.scrollTop = scroll[p] ?? 0;
  }
  function navKey(ev: KeyboardEvent, i: number) {
    if (ev.key !== 'ArrowDown' && ev.key !== 'ArrowUp') return;
    ev.preventDefault();
    const n = (i + (ev.key === 'ArrowDown' ? 1 : pages.length - 1)) % pages.length;
    go(pages[n].id);
    document.getElementById(`nav-${pages[n].id}`)?.focus();
  }

  const e = $derived(app.engine);
  const live = $derived(e?.status === 'buffering');
  const status = $derived.by(() => {
    if (app.engineDown) return { label: 'Needs attention', tone: 'warn' };
    if (!e) return { label: 'Starting…', tone: 'idle' };
    if (e.session_secs != null) return { label: `Recording ${fmtDuration(e.session_secs)}`, tone: 'rec' };
    switch (e.status) {
      case 'buffering': return { label: 'Replay on', tone: 'rec' };
      case 'waiting': return { label: 'Watching for games', tone: 'idle' };
      case 'starting': return { label: 'Starting…', tone: 'idle' };
      case 'ready': return { label: 'Off', tone: 'off' };
      default: return { label: 'Needs attention', tone: 'warn' };
    }
  });
</script>

<div class="shell">
  <nav aria-label="Main">
    <div class="brand"><span class="mark" aria-hidden="true"></span><span class="label">CAPTURE</span></div>
    <ul>
      {#each pages as p, i (p.id)}
        <li>
          <button id="nav-{p.id}" data-nav class:active={page === p.id} aria-current={page === p.id ? 'page' : undefined}
            title={p.label} onclick={() => go(p.id)} onkeydown={ev => navKey(ev, i)}>
            <Icon name={p.icon} size={18} /><span class="label">{p.label}</span>
          </button>
        </li>
      {/each}
    </ul>
    <button class="mini tone-{status.tone}" onclick={() => go('home')} title="{status.label}{live && e?.target_name ? ` · ${e.target_name}` : ''}">
      <span class="dot" aria-hidden="true"></span>
      <span class="label">
        <span class="s">{status.label}</span>
        {#if live && e?.target_name}<span class="g ellipsis">{e.target_name}</span>{/if}
      </span>
    </button>
  </nav>

  <main bind:this={main}>
    <div class="content">
      {#if failed}
        <div class="banner warn">CAPTURE could not load its settings: {failed}</div>
      {:else if !app.settings}
        <p class="muted">Loading…</p>
      {:else if page === 'home'}
        <HomePage onnavigate={go} />
      {:else if page === 'replay'}
        <ReplayPage onnavigate={go} />
      {:else if page === 'library'}
        <LibraryTab />
      {:else}
        <SettingsTab />
      {/if}
    </div>
  </main>

  <div class="toasts" aria-live="polite">
    {#each app.toasts as t (t.id)}<div class="toast {t.kind}">{t.text}</div>{/each}
  </div>
</div>

<style>
  .shell { height: 100vh; display: flex; background: var(--chrome); }
  nav { width: var(--nav-w); flex: none; display: flex; flex-direction: column; padding: var(--s3) var(--s2); gap: var(--s1); }
  .brand { display: flex; align-items: center; gap: var(--s2); height: 36px; padding: 0 var(--s3); margin-bottom: var(--s3);
           font-weight: 700; letter-spacing: 0.14em; font-size: var(--fs-sm); color: var(--text); }
  .mark { width: 10px; height: 10px; border-radius: 50%; background: var(--rec); flex: none; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  nav li button {
    width: 100%; display: flex; align-items: center; gap: var(--s3); height: 38px; padding: 0 var(--s3);
    border: none; border-radius: var(--r-md); background: transparent; color: var(--text-2);
    font-size: var(--fs-md); font-weight: 500; cursor: pointer; text-align: left; position: relative;
    transition: background var(--t-fast) var(--ease), color var(--t-fast) var(--ease);
  }
  nav li button:hover { background: var(--surface-2); color: var(--text); }
  nav li button.active { background: var(--surface); color: var(--text); }
  nav li button.active::before { content: ''; position: absolute; left: 0; top: 10px; bottom: 10px; width: 3px; border-radius: 2px; background: var(--accent); }
  .mini {
    margin-top: auto; display: flex; align-items: center; gap: var(--s2); min-height: 44px; padding: var(--s2) var(--s3);
    border: none; border-radius: var(--r-md); background: transparent; color: var(--text-2); cursor: pointer; text-align: left;
  }
  .mini:hover { background: var(--surface-2); }
  .mini .label { display: flex; flex-direction: column; min-width: 0; line-height: 1.3; }
  .mini .s { font-size: var(--fs-sm); font-weight: 600; color: var(--text); }
  .mini .g { font-size: var(--fs-xs); color: var(--text-3); }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; background: var(--text-3); }
  .tone-rec .dot { background: var(--rec); box-shadow: 0 0 0 3px var(--rec-soft); }
  .tone-warn .dot { background: var(--warn); }
  .tone-off .dot { background: transparent; border: 1.5px solid var(--text-3); }

  main { flex: 1; min-width: 0; overflow-y: auto; background: var(--bg); border-top-left-radius: var(--r-lg); }
  .content { max-width: var(--content-max); margin: 0 auto; padding: var(--s6) var(--s8) var(--s10); }

  @media (max-width: 900px) {
    nav { width: var(--nav-w-compact); align-items: center; }
    .label { display: none !important; }
    .brand { padding: 0; justify-content: center; }
    nav li button { justify-content: center; padding: 0; width: 44px; }
    .mini { justify-content: center; padding: var(--s2); }
    .content { padding: var(--s5) var(--s5) var(--s8); }
  }
</style>
