<script lang="ts">
  import { onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import Toggle from '../components/Toggle.svelte';
  import Segmented from '../components/Segmented.svelte';
  import Meter from '../components/Meter.svelte';
  import {
    app, setReplay, saveReplay, updateSettings, refreshTargets, openPath, revealPath, copyPath,
    openCaptureFolder, fmtDuration, fmtBytes,
  } from '../lib/store.svelte';
  import type { Quality } from '../lib/types';

  const s = $derived(app.settings!);
  const e = $derived(app.engine);

  const durations = [30, 60, 120, 180, 300, 600, 900, 1200];

  const view = $derived.by(() => {
    if (app.engineDown) return { label: 'Needs attention', tone: 'warn', detail: app.engineDown };
    if (!e) return { label: 'Starting…', tone: 'idle', detail: 'Connecting to the capture engine.' };
    if (e.saves_in_progress > 0) return { label: 'Saving…', tone: 'live', detail: 'Finishing your clip. Buffering continues.' };
    switch (e.status) {
      case 'ready': return { label: 'Ready', tone: 'idle', detail: 'Instant replay is off. Turn it on to keep the last few minutes.' };
      case 'waiting': return { label: s.target.kind === 'auto' ? 'Watching for games' : 'Waiting', tone: 'idle', detail: e.attention ?? 'Waiting for something to record.' };
      case 'starting': return { label: 'Starting…', tone: 'idle', detail: e.attention ?? `Waiting for the first frames from ${e.target_name ?? 'the target'}.` };
      case 'buffering': return { label: 'Buffering', tone: 'live', detail: `${e.target_name} · ${fmtDuration(e.buffered_secs)} ready to save` };
      default: return { label: 'Needs attention', tone: 'warn', detail: e.attention ?? 'Something needs your attention.' };
    }
  });

  const gameTrack = $derived(e?.tracks.find(t => t.name === 'Game Audio' || t.name === 'System Audio'));
  const mbPerMin = $derived(e && e.status === 'buffering' && e.buffered_secs > 8 ? (e.buffer_bytes / Math.max(e.buffered_secs, 1)) * 60 : null);

  // "Pick an app" is a UI-only mode until an app is actually chosen (then it is saved).
  let picking = $state(false);
  const sourceMode = $derived<'auto' | 'app' | 'display'>(
    s.target.kind === 'display' ? 'display' : s.target.kind === 'app' || picking ? 'app' : 'auto');

  // Keep the running-apps list current while this tab is open (cheap: one window enumeration).
  onMount(() => {
    refreshTargets();
    const t = setInterval(() => { if (!document.hidden) refreshTargets(); }, 3000);
    return () => clearInterval(t);
  });

  const addGame = (exe: string) => { if (exe) updateSettings({ games: [...s.games, exe], ignored: s.ignored.filter(i => i.toLowerCase() !== exe.toLowerCase()) }); };
  const ignoreGame = (exe: string) => updateSettings({ ignored: [...s.ignored, exe], games: s.games.filter(g => g.toLowerCase() !== exe.toLowerCase()) });
  const missedCandidates = $derived(apps.filter(w => !s.games.some(g => g.toLowerCase() === w.exe.toLowerCase()) && w.exe.toLowerCase() !== (e?.target_exe ?? '').toLowerCase()));
  const selectedValue = $derived(s.target.kind === 'app' ? `app:${s.target.exe}` : s.target.kind === 'display' ? `display:${s.target.index}` : '');
  const targetRunning = $derived(s.target.kind !== 'app' || app.windows.some(w => s.target.kind === 'app' && w.exe.toLowerCase() === s.target.exe.toLowerCase()));

  function chooseTarget(v: string) {
    if (v.startsWith('app:')) {
      const exe = v.slice(4);
      const w = app.windows.find(w => w.exe === exe);
      updateSettings({ target: { kind: 'app', exe, label: w?.app_name ?? exe } });
    } else if (v.startsWith('display:')) {
      const d = app.displays.find(d => d.index === Number(v.slice(8)));
      if (d) updateSettings({ target: { kind: 'display', index: d.index, label: d.name } });
    }
  }

  // One entry per app (an app can have several windows; the engine picks its largest).
  const apps = $derived.by(() => {
    const seen = new Map<string, (typeof app.windows)[number]>();
    for (const w of app.windows) if (!seen.has(w.exe.toLowerCase())) seen.set(w.exe.toLowerCase(), w);
    return [...seen.values()];
  });

  async function changeFolder() {
    const dir = await open({ directory: true, title: 'Choose where CAPTURE saves clips', defaultPath: s.capture_root });
    if (typeof dir === 'string') updateSettings({ capture_root: dir });
  }
</script>

<section class="status card tone-{view.tone}" aria-live="polite">
  <div class="row">
    <span class="pill">{view.label}</span>
    {#if e?.status === 'buffering' && s.target.kind === 'display'}
      <span class="chip">● Desktop is being recorded</span>
    {/if}
    {#if e?.encoder}
      <span class="muted small grow" style="text-align:right">
        {e.encoder.width}×{e.encoder.height} · {e.encoder.fps} fps · {e.encoder.hardware ? 'GPU' : 'CPU'} encoder
      </span>
    {/if}
  </div>
  <p class="detail">{view.detail}</p>
</section>

<section class="card replay">
  <div class="row">
    <Toggle big checked={s.replay_enabled} label="Instant replay" onchange={setReplay} />
    <div class="grow">
      <div class="title">Instant replay</div>
      <div class="muted small">Keeps the most recent footage in a temporary buffer on your capture drive.</div>
    </div>
    <label class="row small">
      <span class="muted">Save last</span>
      <select value={s.replay_secs} onchange={ev => updateSettings({ replay_secs: Number((ev.target as HTMLSelectElement).value) })}>
        {#each durations as d (d)}<option value={d}>{fmtDuration(d)}</option>{/each}
      </select>
    </label>
  </div>
  <div class="row save-row">
    <button class="primary big" disabled={e?.status !== 'buffering'} onclick={saveReplay}>Save clip</button>
    <span class="muted small">
      or press <kbd>{s.save_hotkey || 'no hotkey set'}</kbd>
      {#if e?.hotkey && !e.hotkey.registered && e.hotkey.message}
        <span class="warn"> — {e.hotkey.message}</span>
      {/if}
    </span>
  </div>
</section>

<div class="grid">
  <section class="card">
    <h2>What to record</h2>
    <Segmented label="Recording source" value={sourceMode}
      options={[{ value: 'auto', label: 'Auto (games)' }, { value: 'app', label: 'Pick an app' }, { value: 'display', label: 'Whole display' }]}
      onchange={v => {
        picking = v === 'app';
        if (v === 'auto') updateSettings({ target: { kind: 'auto' } });
        else if (v === 'display') chooseTarget(`display:${app.displays.find(d => d.primary)?.index ?? 0}`);
      }} />

    {#if sourceMode === 'auto'}
      {#if e?.target_exe && (e.status === 'buffering' || e.status === 'starting')}
        <p>Recording <b>{e.target_name}</b> <span class="muted small">— detected from {e.detected_by ?? 'your choice'}</span></p>
        <button class="small-btn" onclick={() => ignoreGame(e!.target_exe!)}>Not a game — stop recording it</button>
      {:else}
        <p class="muted small">Start a game and recording begins on its own. CAPTURE recognises games installed through Steam, Epic, GOG, itch, Xbox, EA, Ubisoft, Riot and Battle.net, anything in a “Games” folder, and anything running fullscreen.</p>
      {/if}
      <div class="row" style="margin-top:10px">
        <select class="grow" aria-label="Add a game CAPTURE missed" value="" onchange={ev => { addGame((ev.target as HTMLSelectElement).value); (ev.target as HTMLSelectElement).value = ''; }}>
          <option value="">Missed a game? Pick it from running apps…</option>
          {#each missedCandidates as w (w.exe)}<option value={w.exe}>{w.app_name} — {w.exe}</option>{/each}
        </select>
      </div>
    {:else}
      <div class="row" style="margin-top:12px">
        <select class="grow" aria-label="Capture target" value={selectedValue} onchange={ev => chooseTarget((ev.target as HTMLSelectElement).value)}>
          {#if sourceMode === 'app'}
            <option value="" disabled>Choose a running game or app…</option>
            {#if s.target.kind === 'app' && !targetRunning}
              <option value={selectedValue}>{s.target.label} — not running</option>
            {/if}
            {#each apps as w (w.exe)}
              <option value="app:{w.exe}">{w.app_name} — {w.exe}</option>
            {/each}
          {:else}
            {#each app.displays as d (d.index)}
              <option value="display:{d.index}">{d.name} — {d.width}×{d.height}</option>
            {/each}
          {/if}
        </select>
      </div>
      {#if sourceMode === 'app'}
        <p class="muted small">The list updates by itself. CAPTURE remembers this app and starts recording whenever it is open.</p>
      {/if}
    {/if}
  </section>

  <section class="card">
    <h2>Audio</h2>
    <div class="row">
      <Toggle checked={s.game_audio} label="Game sound" onchange={v => updateSettings({ game_audio: v })} />
      <div class="grow">
        <div>{sourceMode === 'display' ? 'System sound' : 'Game sound'}</div>
        <div class="muted small ellipsis">
          {#if !s.game_audio}Off
          {:else if gameTrack?.error}<span class="warn">{gameTrack.error}</span>
          {:else if gameTrack}{gameTrack.mode === 'process-loopback' ? 'Only this app’s sound' : 'Everything you hear'} · {gameTrack.signal_seen ? 'signal detected' : 'no sound yet'}
          {:else}{sourceMode === 'display' ? 'Everything you hear on your default output' : 'Only the selected app’s sound'}{/if}
        </div>
      </div>
    </div>
    {#if s.game_audio && gameTrack}
      <div style="margin-top:10px"><Meter peak={gameTrack.peak} label="Game sound level" /></div>
    {/if}
    <p class="muted small">Saved clips get a <b>Preview Mix</b> track for normal playback plus a separate <b>{sourceMode === 'display' ? 'System Audio' : 'Game Audio'}</b> track for editing. The microphone is not recorded.</p>
  </section>

  <section class="card">
    <h2>Quality</h2>
    <Segmented label="Quality" value={s.quality}
      options={[{ value: 'balanced', label: 'Balanced' }, { value: 'high', label: 'High' }, { value: 'maximum', label: 'Maximum' }] as { value: Quality; label: string }[]}
      onchange={v => updateSettings({ quality: v })} />
    <p class="muted small">
      {#if mbPerMin}Measured right now: about {fmtBytes(mbPerMin)} per minute.{:else}File size depends on the game; it is measured once buffering starts.{/if}
    </p>
  </section>

  <section class="card">
    <h2>Saved to</h2>
    <div class="mono ellipsis" title={s.capture_root}>{s.capture_root}\Clips</div>
    <div class="row" style="margin-top:10px">
      <button onclick={changeFolder}>Change…</button>
      <button onclick={openCaptureFolder}>Open folder</button>
    </div>
    <p class="muted small">
      Temporary buffer: {fmtBytes(e?.buffer_bytes ?? 0)} of {s.buffer_cap_gb} GB · Saved clips are never deleted automatically.
    </p>
  </section>
</div>

{#if app.lastClip}
  {@const c = app.lastClip}
  <section class="card last">
    <h2>Last saved clip</h2>
    <div class="row">
      <div class="grow">
        <div class="ellipsis">{c.path.split('\\').pop()}</div>
        <div class="muted small">{fmtDuration(c.duration_secs)} · {fmtBytes(c.bytes)} · tracks: {c.audio_tracks.join(', ') || 'none'}</div>
      </div>
      <button onclick={() => openPath(c.path)}>Open file</button>
      <button onclick={() => revealPath(c.path)}>Show in folder</button>
      <button onclick={() => copyPath(c.path)}>Copy path</button>
    </div>
  </section>
{/if}

<style>
  .status { margin-bottom: 14px; }
  .pill { font-size: 22px; font-weight: 700; }
  .tone-live .pill { color: var(--accent-2); }
  .tone-warn .pill { color: var(--warn); }
  .tone-live { border-color: #5b2a2a; }
  .tone-warn { border-color: #5c4a1c; }
  .detail { margin: 6px 0 0; color: var(--muted); }
  .chip { font-size: 12px; color: var(--accent-2); border: 1px solid #5b2a2a; border-radius: 999px; padding: 2px 10px; }
  .replay { margin-bottom: 14px; }
  .replay .title { font-weight: 600; font-size: 17px; }
  .save-row { margin-top: 14px; }
  button.big { padding: 10px 26px; font-size: 16px; }
  kbd { font-family: "Cascadia Mono", Consolas, monospace; background: var(--panel-2); border: 1px solid var(--line); border-radius: 5px; padding: 1px 6px; }
  .warn { color: var(--warn); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 14px; }
  .grid p { margin: 10px 0 0; }
  .last { margin-top: 14px; }
  .small-btn { font-size: 13px; padding: 4px 10px; }
</style>
