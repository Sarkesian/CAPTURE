<script lang="ts">
  // What to record: Auto (games) / a chosen app / a whole display.
  import { onMount } from 'svelte';
  import { app, updateSettings, refreshTargets } from '../lib/store.svelte';
  import Segmented from './Segmented.svelte';
  import Icon from './Icon.svelte';

  const s = $derived(app.settings!);
  const e = $derived(app.engine);
  let picking = $state(false);
  const mode = $derived<'auto' | 'app' | 'display'>(s.target.kind === 'display' ? 'display' : s.target.kind === 'app' || picking ? 'app' : 'auto');
  const apps = $derived.by(() => {
    const seen = new Map<string, (typeof app.windows)[number]>();
    for (const w of app.windows) if (!seen.has(w.exe.toLowerCase())) seen.set(w.exe.toLowerCase(), w);
    return [...seen.values()];
  });
  const appValue = $derived(s.target.kind === 'app' ? s.target.exe : '');
  const running = $derived(s.target.kind !== 'app' || apps.some(w => s.target.kind === 'app' && w.exe.toLowerCase() === s.target.exe.toLowerCase()));
  const missed = $derived(apps.filter(w => !s.games.some(g => g.toLowerCase() === w.exe.toLowerCase()) && w.exe.toLowerCase() !== (e?.target_exe ?? '').toLowerCase()));
  const capturing = $derived(e?.target_exe && (e.status === 'buffering' || e.status === 'starting'));

  onMount(() => {
    refreshTargets();
    const t = setInterval(() => { if (!document.hidden) refreshTargets(); }, 3000);
    return () => clearInterval(t);
  });

  function setMode(v: 'auto' | 'app' | 'display') {
    picking = v === 'app';
    if (v === 'auto') updateSettings({ target: { kind: 'auto' } });
    if (v === 'display') {
      const d = app.displays.find(d => d.primary) ?? app.displays[0];
      if (d) updateSettings({ target: { kind: 'display', index: d.index, label: d.name } });
    }
  }
  function pickApp(exe: string) {
    const w = apps.find(w => w.exe === exe);
    if (exe) updateSettings({ target: { kind: 'app', exe, label: w?.app_name ?? exe } });
  }
  const addGame = (exe: string) => exe && updateSettings({ games: [...s.games, exe], ignored: s.ignored.filter(i => i.toLowerCase() !== exe.toLowerCase()) });
  const ignore = (exe: string) => updateSettings({ ignored: [...s.ignored, exe], games: s.games.filter(g => g.toLowerCase() !== exe.toLowerCase()) });
</script>

<div class="stack">
  <Segmented label="What to record" value={mode} onchange={setMode}
    options={[{ value: 'auto', label: 'Auto (games)' }, { value: 'app', label: 'An app' }, { value: 'display', label: 'Whole display' }]} />

  {#if mode === 'auto'}
    {#if capturing}
      <div class="current">
        <Icon name="game" size={18} />
        <div class="grow">
          <div class="ellipsis"><b>{e!.target_name}</b></div>
          <div class="faint xs">Found in {e!.detected_by ?? 'your games list'}</div>
        </div>
        <button class="btn ghost sm" onclick={() => ignore(e!.target_exe!)} title="Stop recording this app and never auto-record it again">Not a game</button>
      </div>
    {:else}
      <p class="muted small">Start a game and recording begins on its own: Steam, Epic, GOG, itch, Xbox, EA, Ubisoft, Riot and Battle.net games, anything in a “Games” folder, and anything fullscreen.</p>
    {/if}
    <select class="select" aria-label="Add a game CAPTURE missed" value="" onchange={ev => { addGame((ev.target as HTMLSelectElement).value); (ev.target as HTMLSelectElement).value = ''; }}>
      <option value="">Missed a game? Choose it from running apps…</option>
      {#each missed as w (w.exe)}<option value={w.exe}>{w.app_name} ({w.exe})</option>{/each}
    </select>
  {:else if mode === 'app'}
    <select class="select" aria-label="App to record" value={appValue} onchange={ev => pickApp((ev.target as HTMLSelectElement).value)}>
      <option value="" disabled>Choose a running app…</option>
      {#if s.target.kind === 'app' && !running}<option value={appValue}>{s.target.label}: not running</option>{/if}
      {#each apps as w (w.exe)}<option value={w.exe}>{w.app_name} ({w.exe})</option>{/each}
    </select>
    <p class="faint small">Remembered: recording starts whenever this app is open.</p>
  {:else}
    <select class="select" aria-label="Display to record" value={s.target.kind === 'display' ? String(s.target.index) : ''}
      onchange={ev => { const d = app.displays.find(d => d.index === Number((ev.target as HTMLSelectElement).value)); if (d) updateSettings({ target: { kind: 'display', index: d.index, label: d.name } }); }}>
      {#each app.displays as d (d.index)}<option value={String(d.index)}>{d.name} · {d.width}×{d.height}</option>{/each}
    </select>
    <p class="faint small">Everything on this screen is recorded, including notifications and other windows.</p>
  {/if}
</div>

<style>
  .current { display: flex; align-items: center; gap: var(--s3); padding: var(--s3); border-radius: var(--r-md); background: var(--surface-2); color: var(--text-2); }
  .current b { color: var(--text); font-weight: 600; }
  .select { width: 100%; }
</style>
