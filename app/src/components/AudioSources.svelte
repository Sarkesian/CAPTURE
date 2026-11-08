<script lang="ts">
  // Audio sources in plain language. Used in the main window (Capture tab) and the overlay.
  // Meters show real engine levels: the recorded track while recording, otherwise a level-only
  // monitor that runs only while this panel is open and the microphone is enabled.
  import { onMount } from 'svelte';
  import { app, engine, updateSettings } from '../lib/store.svelte';
  import Toggle from './Toggle.svelte';
  import Meter from './Meter.svelte';

  let { compact = false }: { compact?: boolean } = $props();
  const s = $derived(app.settings!);
  const e = $derived(app.engine);
  const track = (name: string) => e?.tracks.find(t => t.name === name);
  const display = $derived(s.target.kind === 'display');
  const gameT = $derived(track(display ? 'System Audio' : 'Game Audio'));
  const micT = $derived(track('Microphone'));
  const voiceT = $derived(track('Voice Chat'));
  const otherT = $derived(track('Other Desktop'));
  const defaultMic = $derived(app.inputs.find(d => d.is_default));
  const chosenMissing = $derived(!!s.mic_device && app.inputs.length > 0 && !app.inputs.some(d => d.id === s.mic_device));
  const micPeak = $derived(micT ? micT.peak : (e?.mic_monitor?.peak ?? 0));
  const micError = $derived(micT?.error ?? e?.mic_monitor?.error ?? null);
  const voiceApps = $derived.by(() => {
    const known = ['Discord.exe', 'Teams.exe', 'ms-teams.exe', 'TeamSpeak.exe', 'ts3client_win64.exe', 'Mumble.exe', 'Skype.exe', 'Guilded.exe'];
    const list = app.apps.map(a => ({ exe: a.exe, name: a.name }));
    for (const k of known) if (!list.some(a => a.exe.toLowerCase() === k.toLowerCase()) && s.voice_app?.toLowerCase() === k.toLowerCase()) list.push({ exe: k, name: k.replace('.exe', '') });
    if (s.voice_app && !list.some(a => a.exe.toLowerCase() === s.voice_app!.toLowerCase())) list.push({ exe: s.voice_app, name: `${s.voice_app} (not running)` });
    return list;
  });

  // Level monitor only while the panel is open, the mic is on, and nothing is recording it.
  const wantMonitor = $derived(s.mic_enabled && !micT);
  let monitoring = false;
  $effect(() => {
    const want = wantMonitor, dev = s.mic_device;
    if (want) { engine('mic_monitor', { enabled: true, device: dev }); monitoring = true; }
    else if (monitoring) { engine('mic_monitor', { enabled: false }); monitoring = false; }
  });
  onMount(() => {
    engine('list_audio');
    const t = setInterval(() => { if (!document.hidden) engine('list_audio'); }, 5000);
    return () => { clearInterval(t); if (monitoring) engine('mic_monitor', { enabled: false }); };
  });

  const vol = (key: 'game' | 'mic' | 'voice', v: number) => updateSettings({ volumes: { ...s.volumes, [key]: v } });
  const testing = $derived(app.micTest && (app.micTest.phase === 'recording' || app.micTest.phase === 'playing'));
  function test() { app.micTest = { phase: 'recording', peak: 0, message: null }; engine('mic_test', { device: s.mic_device }); }
</script>

<div class="srcs" class:compact>
  <!-- Game sound -->
  <div class="src">
    <Toggle checked={s.game_audio} label={display ? 'Computer sound' : 'Game sound'} onchange={v => updateSettings({ game_audio: v })} />
    <div class="body">
      <div class="title">{display ? 'Computer sound' : 'Game sound'}</div>
      <div class="muted small">{display ? 'Everything you hear (CAPTURE’s own sounds excluded)' : 'Only the game’s own sound'}
        {#if gameT?.error}<span class="warn"> · {gameT.error}</span>{/if}</div>
    </div>
    {#if s.game_audio}
      <div class="meter"><Meter peak={gameT?.peak ?? 0} label="Game sound level" /></div>
      <input class="vol" type="range" min="0" max="2" step="0.05" value={s.volumes.game} aria-label="Game sound level in the mix" title="Level in Preview Mix" onchange={ev => vol('game', Number((ev.target as HTMLInputElement).value))} />
    {/if}
  </div>

  <!-- Microphone: off until enabled -->
  <div class="src">
    <Toggle checked={s.mic_enabled} label="Your microphone" onchange={v => updateSettings({ mic_enabled: v })} />
    <div class="body">
      <div class="title">Your microphone
        {#if s.mic_enabled && e?.status === 'buffering' && !s.mic_muted}<span class="live">● Mic is being recorded</span>{/if}
        {#if s.mic_enabled && s.mic_muted}<span class="muted small"> · muted</span>{/if}
      </div>
      {#if s.mic_enabled}
        <select aria-label="Microphone" value={s.mic_device ?? ''} onchange={ev => updateSettings({ mic_device: (ev.target as HTMLSelectElement).value || null })}>
          <option value="">Windows default{defaultMic ? ` (now: ${defaultMic.name})` : ''}</option>
          {#each app.inputs as d (d.id)}<option value={d.id}>{d.name}{d.is_virtual ? ' — virtual device' : ''}</option>{/each}
          {#if chosenMissing}<option value={s.mic_device}>Your chosen microphone — disconnected</option>{/if}
        </select>
        {#if micError}<div class="small warn">{micError}</div>
        {:else if chosenMissing}<div class="small warn">Your chosen microphone is disconnected. CAPTURE will not switch to another one; reconnect it or choose another.</div>{/if}
      {:else}
        <div class="muted small">Off — nothing from your microphone is recorded.</div>
      {/if}
    </div>
    {#if s.mic_enabled}
      <div class="meter"><Meter peak={s.mic_muted ? 0 : micPeak} label="Microphone level" /></div>
      <button class:on={s.mic_muted} onclick={() => updateSettings({ mic_muted: !s.mic_muted })}>{s.mic_muted ? 'Unmute' : 'Mute'}</button>
      {#if !compact}<button disabled={!!testing} onclick={test}>{testing ? (app.micTest?.phase === 'playing' ? 'Playing back…' : 'Speak now…') : 'Test'}</button>{/if}
    {/if}
  </div>
  {#if s.mic_enabled && app.micTest && !compact}
    <div class="note small" class:warn={app.micTest.phase === 'failed'}>
      {#if app.micTest.phase === 'recording'}Recording 3 seconds — say something.
      {:else if app.micTest.phase === 'playing'}Playing it back…
      {:else}{app.micTest.message}{/if}
    </div>
  {/if}

  <!-- Voice chat app -->
  <div class="src">
    <Toggle checked={s.voice_enabled} label="Voice chat" onchange={v => updateSettings({ voice_enabled: v })} />
    <div class="body">
      <div class="title">Voice chat</div>
      {#if s.voice_enabled}
        <select aria-label="Voice chat app" value={s.voice_app ?? ''} onchange={ev => updateSettings({ voice_app: (ev.target as HTMLSelectElement).value || null })}>
          <option value="">Choose the app (Discord, Teams…)</option>
          {#each voiceApps as a (a.exe)}<option value={a.exe}>{a.name} — {a.exe}</option>{/each}
        </select>
        {#if voiceT?.error}<div class="small muted">{voiceT.error}</div>{/if}
        {#if !compact}<div class="small muted">Recorded from that app only. Voices inside the game’s own chat are part of the game sound and can’t be separated.</div>{/if}
      {:else}
        <div class="muted small">Off</div>
      {/if}
    </div>
    {#if s.voice_enabled}
      <div class="meter"><Meter peak={voiceT?.peak ?? 0} label="Voice chat level" /></div>
      <input class="vol" type="range" min="0" max="2" step="0.05" value={s.volumes.voice} aria-label="Voice chat level in the mix" title="Level in Preview Mix" onchange={ev => vol('voice', Number((ev.target as HTMLInputElement).value))} />
    {/if}
  </div>

  {#if !compact && !display}
    <div class="src">
      <Toggle checked={s.other_enabled} label="Other computer audio" onchange={v => updateSettings({ other_enabled: v })} />
      <div class="body">
        <div class="title">Other computer audio</div>
        <div class="muted small">Everything you hear, as an extra track for editing. It also contains the game and voice chat, so it is not added to Preview Mix.</div>
      </div>
      {#if s.other_enabled}<div class="meter"><Meter peak={otherT?.peak ?? 0} label="Other audio level" /></div>{/if}
    </div>
  {/if}
</div>

<style>
  .srcs { display: flex; flex-direction: column; gap: 4px; }
  .src { display: flex; align-items: center; gap: 12px; padding: 8px 0; border-top: 1px solid var(--line); flex-wrap: wrap; }
  .src:first-child { border-top: none; }
  .body { flex: 1; min-width: 200px; display: flex; flex-direction: column; gap: 4px; }
  .title { font-weight: 500; }
  .body select { max-width: 420px; }
  .meter { width: 140px; }
  .vol { width: 90px; }
  .live { color: var(--accent-2); font-size: 12px; margin-left: 8px; }
  .warn { color: var(--warn); }
  .note { padding: 0 0 8px 58px; color: var(--muted); }
  .note.warn { color: var(--warn); }
  button.on { border-color: var(--warn); color: var(--warn); }
  .compact .src { padding: 6px 0; }
</style>
