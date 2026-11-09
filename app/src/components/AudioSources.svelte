<script lang="ts">
  // Audio sources in plain language, each with a real level meter and a clear state.
  // Meters read real engine levels: the recorded track while recording; for the microphone,
  // a level-only monitor that runs only while this panel is visible and the mic is enabled.
  import { onMount } from 'svelte';
  import { app, engine, updateSettings } from '../lib/store.svelte';
  import Toggle from './Toggle.svelte';
  import Meter from './Meter.svelte';
  import Icon from './Icon.svelte';

  let { compact = false }: { compact?: boolean } = $props();
  const s = $derived(app.settings!);
  const e = $derived(app.engine);
  const recording = $derived(e?.status === 'buffering');
  const track = (name: string) => e?.tracks.find(t => t.name === name);
  const display = $derived(s.target.kind === 'display');
  const gameName = $derived(display ? 'System Audio' : 'Game Audio');
  const gameT = $derived(track(gameName));
  const micT = $derived(track('Microphone'));
  const voiceT = $derived(track('Voice Chat'));
  const otherT = $derived(track('All Desktop Audio'));
  const defaultMic = $derived(app.inputs.find(d => d.is_default));
  const chosenMissing = $derived(!!s.mic_device && app.inputs.length > 0 && !app.inputs.some(d => d.id === s.mic_device));
  const micError = $derived(micT?.error ?? e?.mic_monitor?.error ?? null);
  const micName = $derived(s.mic_device ? (app.inputs.find(d => d.id === s.mic_device)?.name ?? 'Chosen microphone') : (defaultMic?.name ?? 'Windows default'));
  const voiceApps = $derived.by(() => {
    const list = app.apps.map(a => ({ exe: a.exe, name: a.name }));
    if (s.voice_app && !list.some(a => a.exe.toLowerCase() === s.voice_app!.toLowerCase())) list.push({ exe: s.voice_app, name: `${s.voice_app} (not running)` });
    return list;
  });
  const voiceRunning = $derived(!!s.voice_app && app.apps.some(a => a.exe.toLowerCase() === s.voice_app!.toLowerCase()));

  type St = 'live' | 'off' | 'muted' | 'error' | 'idle';
  const gameState = $derived<St>(!s.game_audio ? 'off' : gameT?.error ? 'error' : gameT ? 'live' : 'idle');
  const micState = $derived<St>(!s.mic_enabled ? 'off' : (micError || chosenMissing) ? 'error' : s.mic_muted ? 'muted' : (micT || e?.mic_monitor) ? 'live' : 'idle');
  const voiceState = $derived<St>(!s.voice_enabled ? 'off' : !s.voice_app ? 'idle' : voiceT?.error ? 'error' : voiceT ? 'live' : 'idle');
  const otherState = $derived<St>(!s.other_enabled ? 'off' : otherT?.error ? 'error' : otherT ? 'live' : 'idle');

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
  const pct = (v: number) => `${Math.round(v * 100)}%`;
  const testing = $derived(app.micTest && (app.micTest.phase === 'recording' || app.micTest.phase === 'playing'));
  function test() { app.micTest = { phase: 'recording', peak: 0, message: null }; engine('mic_test', { device: s.mic_device }); }
  const stateText = (st: St, idle: string) => st === 'off' ? 'Off' : st === 'muted' ? 'Muted' : st === 'idle' ? idle : '';
</script>

<div class="srcs" class:compact>
  {#if recording && !compact}
    <div class="src mix">
      <span class="ic"><Icon name="audio" size={18} /></span>
      <div class="body"><div class="name">Preview Mix</div><div class="sub">What you hear when you play the clip</div></div>
      <div class="m"><Meter source="Preview Mix" label="Preview Mix level" /></div>
      <span class="ctl"></span>
    </div>
  {/if}

  <!-- Game / computer sound -->
  <div class="src" class:dim={gameState === 'off'}>
    <span class="ic"><Icon name={display ? 'monitor' : 'speaker'} size={18} /></span>
    <div class="body">
      <div class="name">{display ? 'Computer sound' : 'Game sound'}</div>
      <div class="sub" class:warn={gameState === 'error'}>
        {#if gameState === 'error'}{gameT!.error}
        {:else}{display ? 'Everything you hear, without CAPTURE’s own sounds' : 'Only the game’s own sound'}{/if}
      </div>
    </div>
    <div class="m">
      <Meter source={gameName} label="Game sound level" state={gameState} />
      {#if gameState === 'idle'}<span class="st">Waiting for a game</span>{/if}
    </div>
    <span class="ctl"><Toggle checked={s.game_audio} label={display ? 'Computer sound' : 'Game sound'} onchange={v => updateSettings({ game_audio: v })} /></span>
    {#if !compact && s.game_audio}
      <div class="extra row">
        <span class="faint xs lbl">Level in mix</span>
        <input type="range" min="0" max="2" step="0.05" value={s.volumes.game} aria-label="Game sound level in the mix" oninput={ev => (app.settings!.volumes.game = Number((ev.target as HTMLInputElement).value))} onchange={ev => vol('game', Number((ev.target as HTMLInputElement).value))} />
        <span class="xs mono val">{pct(s.volumes.game)}</span>
      </div>
    {/if}
  </div>

  <!-- Microphone: off until enabled -->
  <div class="src" class:dim={micState === 'off'}>
    <span class="ic" class:warn={micState === 'error'}><Icon name={s.mic_enabled && !s.mic_muted ? 'mic' : 'mic-off'} size={18} /></span>
    <div class="body">
      <div class="name">Microphone
        {#if s.mic_enabled && recording && !s.mic_muted && micT}<span class="tag rec">Recording</span>{/if}
      </div>
      <div class="sub" class:ellipsis={micState !== 'error'} class:warn={micState === 'error'} title={micError ?? micName}>
        {#if !s.mic_enabled}Off. Nothing from your microphone is recorded.
        {:else if chosenMissing}Disconnected. Reconnect it or choose another.
        {:else if micError}{micError}
        {:else}{micName}{/if}
      </div>
    </div>
    <div class="m">
      <Meter source={micT ? 'Microphone' : 'mic_monitor'} label="Microphone level" state={micState} />
      {#if stateText(micState, 'No signal yet')}<span class="st">{stateText(micState, 'No signal yet')}</span>{/if}
    </div>
    <span class="ctl">
      {#if s.mic_enabled}
        <button class="btn ghost sm icon" class:on={s.mic_muted} aria-pressed={s.mic_muted} title={s.mic_muted ? 'Unmute microphone' : 'Mute microphone'} aria-label={s.mic_muted ? 'Unmute microphone' : 'Mute microphone'}
          onclick={() => updateSettings({ mic_muted: !s.mic_muted })}><Icon name={s.mic_muted ? 'mic-off' : 'mic'} size={16} /></button>
      {/if}
      <Toggle checked={s.mic_enabled} label="Your microphone" onchange={v => updateSettings({ mic_enabled: v })} />
    </span>
    {#if !compact && s.mic_enabled}
      <div class="extra row wrap">
        <select class="select grow" aria-label="Microphone" value={s.mic_device ?? ''} onchange={ev => updateSettings({ mic_device: (ev.target as HTMLSelectElement).value || null })}>
          <option value="">Windows default{defaultMic ? ` (now: ${defaultMic.name})` : ''}</option>
          {#each app.inputs as d (d.id)}<option value={d.id}>{d.name}{d.is_virtual ? ' (virtual device)' : ''}{d.muted ? ' (muted in Windows)' : ''}</option>{/each}
          {#if chosenMissing}<option value={s.mic_device}>Your chosen microphone (disconnected)</option>{/if}
        </select>
        <button class="btn sm" disabled={!!testing} onclick={test}>{testing ? (app.micTest?.phase === 'playing' ? 'Playing back…' : 'Speak now…') : 'Test'}</button>
      </div>
      <div class="extra row">
        <span class="faint xs lbl">Level in mix</span>
        <input type="range" min="0" max="2" step="0.05" value={s.volumes.mic} aria-label="Microphone level in the mix" oninput={ev => (app.settings!.volumes.mic = Number((ev.target as HTMLInputElement).value))} onchange={ev => vol('mic', Number((ev.target as HTMLInputElement).value))} />
        <span class="xs mono val">{pct(s.volumes.mic)}</span>
      </div>
      {#if app.micTest}
        <div class="extra small" class:warn={app.micTest.phase === 'failed'}>
          {#if app.micTest.phase === 'recording'}Recording 3 seconds. Say something.
          {:else if app.micTest.phase === 'playing'}Playing it back…
          {:else}{app.micTest.message}{/if}
        </div>
      {/if}
    {/if}
  </div>

  <!-- Voice chat app -->
  <div class="src" class:dim={voiceState === 'off'}>
    <span class="ic"><Icon name="chat" size={18} /></span>
    <div class="body">
      <div class="name">Voice chat</div>
      <div class="sub" class:warn={voiceState === 'error'}>
        {#if !s.voice_enabled}Off
        {:else if voiceT?.error}{voiceT.error}
        {:else if !s.voice_app}Choose the app below
        {:else}{s.voice_app}{voiceRunning ? '' : ' (not running)'}{/if}
      </div>
    </div>
    <div class="m">
      <Meter source="Voice Chat" label="Voice chat level" state={voiceState} />
      {#if voiceState === 'idle'}<span class="st">{s.voice_app ? 'Not recording' : 'No app chosen'}</span>{/if}
    </div>
    <span class="ctl"><Toggle checked={s.voice_enabled} label="Voice chat" onchange={v => updateSettings({ voice_enabled: v })} /></span>
    {#if s.voice_enabled && (!compact || !s.voice_app)}
      <div class="extra row wrap">
        <select class="select grow" aria-label="Voice chat app" value={s.voice_app ?? ''} onchange={ev => updateSettings({ voice_app: (ev.target as HTMLSelectElement).value || null })}>
          <option value="">Choose the app (Discord, Teams…)</option>
          {#each voiceApps as a (a.exe)}<option value={a.exe}>{a.name} ({a.exe})</option>{/each}
        </select>
      </div>
      {#if !compact}
        <div class="extra row">
          <span class="faint xs lbl">Level in mix</span>
          <input type="range" min="0" max="2" step="0.05" value={s.volumes.voice} aria-label="Voice chat level in the mix" oninput={ev => (app.settings!.volumes.voice = Number((ev.target as HTMLInputElement).value))} onchange={ev => vol('voice', Number((ev.target as HTMLInputElement).value))} />
          <span class="xs mono val">{pct(s.volumes.voice)}</span>
        </div>
        <p class="extra faint xs">Recorded from that app only. Voices inside a game’s own chat are part of the game sound.</p>
      {/if}
    {/if}
  </div>

  {#if !compact && !display}
    <div class="src" class:dim={otherState === 'off'}>
      <span class="ic"><Icon name="monitor" size={18} /></span>
      <div class="body">
        <div class="name">All computer audio (extra track)</div>
        <div class="sub">Everything you hear, as an extra editing track. It also contains the game and voice chat (Windows can leave out only one app), so it is never added to Preview Mix.</div>
      </div>
      <div class="m"><Meter source="All Desktop Audio" label="All computer audio level" state={otherState} /></div>
      <span class="ctl"><Toggle checked={s.other_enabled} label="All computer audio" onchange={v => updateSettings({ other_enabled: v })} /></span>
    </div>
  {/if}
</div>

<style>
  .srcs { display: flex; flex-direction: column; }
  .src { display: grid; grid-template-columns: 28px minmax(0, 1fr) minmax(90px, 160px) auto; align-items: center; column-gap: var(--s3); row-gap: var(--s2);
         padding: var(--s3) 0; border-top: 1px solid var(--line); }
  .src:first-child { border-top: none; padding-top: 0; }
  .ic { color: var(--text-2); display: grid; place-items: center; }
  .ic.warn { color: var(--warn); }
  .body { min-width: 0; }
  .name { font-weight: 550; display: flex; align-items: center; flex-wrap: wrap; gap: 2px var(--s2); min-width: 0; }
  .sub { color: var(--text-3); font-size: var(--fs-xs); margin-top: 1px; }
  .sub.warn { color: var(--warn); }
  .m { display: flex; flex-direction: column; gap: 3px; }
  .st { font-size: var(--fs-xs); color: var(--text-3); }
  .ctl { display: flex; align-items: center; gap: var(--s2); justify-self: end; }
  .dim .name { color: var(--text-2); }
  .extra { grid-column: 2 / -1; }
  .extra.row { gap: var(--s2); }
  .extra.warn { color: var(--warn); }
  .lbl { width: 76px; flex: none; }
  .extra input[type='range'] { flex: 1; max-width: 220px; }
  .val { width: 40px; color: var(--text-2); }
  .tag { font-size: var(--fs-xs); font-weight: 600; padding: 0 var(--s2); border-radius: var(--r-pill); }
  .tag.rec { color: var(--rec); background: var(--rec-soft); }
  .btn.on { color: var(--warn); }
  .mix .name { color: var(--text); }
  .compact .src { padding: var(--s2) 0; }
  .compact .src:first-child { padding-top: 0; }
</style>
