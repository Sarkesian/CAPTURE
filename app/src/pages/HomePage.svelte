<script lang="ts">
  // Home: is it recording, what, how much replay is ready, is audio OK, and the main actions.
  import { open } from '@tauri-apps/plugin-dialog';
  import {
    app, setReplay, saveReplay, addMarker, toggleSession, updateSettings, engine, clearTempCache,
    openCaptureFolder, openPath, revealPath, fmtDuration, fmtBytes,
  } from '../lib/store.svelte';
  import Icon from '../components/Icon.svelte';
  import Toggle from '../components/Toggle.svelte';
  import SourcePicker from '../components/SourcePicker.svelte';
  import AudioSources from '../components/AudioSources.svelte';
  import RecentClips from '../components/RecentClips.svelte';
  import FirstRun from '../components/FirstRun.svelte';

  let { onnavigate }: { onnavigate: (p: 'home' | 'replay' | 'library' | 'settings') => void } = $props();

  const s = $derived(app.settings!);
  const e = $derived(app.engine);
  const live = $derived(e?.status === 'buffering');
  const sessionOn = $derived(e?.session_secs != null);
  const saving = $derived((e?.saves_in_progress ?? 0) > 0);
  const hk = (a: string) => e?.hotkeys.find(h => h.action === a && h.registered)?.combo ?? '';
  const lengths = [30, 60, 120, 180, 300, 600, 900, 1200];

  // One clear state, with a title, an explanation and (when needed) the fix.
  const view = $derived.by(() => {
    if (app.engineDown) return { tone: 'warn', kicker: 'Needs attention', title: 'The capture engine stopped', detail: app.engineDown };
    if (!e) return { tone: 'idle', kicker: 'Starting', title: 'Starting CAPTURE…', detail: '' };
    switch (e.status) {
      case 'ready': return { tone: 'off', kicker: 'Off', title: 'Recording is off', detail: 'Turn it on and CAPTURE keeps your recent gameplay, ready to save with one key.' };
      case 'waiting': return { tone: 'idle', kicker: 'Watching for games', title: s.target.kind === 'app' ? `Waiting for ${s.target.label}` : 'No game running', detail: e.attention ?? 'Start a game and recording begins automatically.' };
      case 'starting': return { tone: 'idle', kicker: 'Starting', title: e.target_name ?? 'Starting…', detail: e.attention ?? 'Waiting for the first frames.' };
      case 'buffering': return { tone: sessionOn ? 'rec' : 'live', kicker: sessionOn ? `Recording session · ${fmtDuration(e.session_secs!)}` : 'Replay on', title: e.target_name ?? '', detail: [e.detected_by && `Auto-detected (${e.detected_by})`, e.encoder && `${e.encoder.width}×${e.encoder.height}`, e.encoder && `${e.encoder.fps} fps`].filter(Boolean).join(' · ') };
      default: return { tone: 'warn', kicker: 'Needs attention', title: e.target_name ?? 'Recording paused', detail: e.attention ?? '' };
    }
  });
  const ready = $derived(e?.buffered_secs ?? 0);
  const fill = $derived(Math.min(1, ready / Math.max(1, s.replay_secs)));
  const measured = $derived(e && live && e.buffered_secs > 8 ? (e.buffer_bytes / e.buffered_secs) * 60 : null);

  async function changeFolder() {
    const dir = await open({ directory: true, title: 'Choose where CAPTURE saves clips', defaultPath: s.capture_root });
    if (typeof dir === 'string') updateSettings({ capture_root: dir });
  }
</script>

<div class="home">
  {#if !s.first_run_done}
    <FirstRun />
  {/if}

  {#each app.pathIssues as issue (issue.which)}
    <section class="banner warn" data-test="path-issue">
      <Icon name="alert" size={18} />
      <div class="grow">
        <b>{issue.which === 'capture_root' ? 'Your recordings folder isn’t available' : 'Your background recording folder isn’t available'}</b>:
        <span class="mono">{issue.path}</span>. Its drive isn’t connected to this PC. CAPTURE keeps this setting, so it works again when the drive is back,
        {issue.which === 'capture_root' ? 'but it can’t save until then.' : 'but background recording is paused until then.'}
      </div>
      {#if issue.which === 'capture_root'}
        <button class="btn sm" onclick={changeFolder}>Choose folder…</button>
      {:else}
        <button class="btn sm" onclick={() => updateSettings({ buffer_root: null })}>Use the recordings folder</button>
      {/if}
    </section>
  {/each}

  {#if (e?.leftover_buffer_bytes ?? 0) > 0}
    <section class="banner warn">
      <Icon name="alert" size={18} />
      <div class="grow"><b>Footage from a session that didn’t close normally</b> ({fmtBytes(e!.leftover_buffer_bytes)}). Keep it as a clip, or delete it.</div>
      <button class="btn sm primary" onclick={() => engine('recover_leftovers')}>Save as clip</button>
      <button class="btn sm ghost" onclick={clearTempCache}>Delete</button>
    </section>
  {/if}

  <!-- Hero: status, replay readiness, primary actions -->
  <section class="hero tone-{view.tone}" aria-live="polite">
    <div class="info">
      <div class="kicker"><span class="dot" aria-hidden="true"></span><span data-test="status">{view.kicker}</span></div>
      <h1 class="title ellipsis" data-test="title" title={view.title}>{view.title}</h1>
      {#if view.detail}<p class="detail">{view.detail}</p>{/if}
      {#if app.lastClip}
        {@const name = app.lastClip.path.split(/[\\/]/).pop() ?? ''}
        <div class="last row small" data-test="last-clip">
          <Icon name="check" size={14} /><span class="muted">Saved</span>
          <button class="link ellipsis" title="Play {name}" onclick={() => openPath(app.lastClip!.path)}>{name}</button>
          <span class="faint">{fmtDuration(app.lastClip.duration_secs)} · {fmtBytes(app.lastClip.bytes)}</span>
          <button class="btn ghost sm icon" title="Show in folder" aria-label="Show the last clip in its folder" onclick={() => revealPath(app.lastClip!.path)}><Icon name="folder" size={14} /></button>
        </div>
      {/if}
      {#if s.replay_enabled}
        <div class="ready">
          <div class="bar" role="progressbar" aria-label="Replay ready" aria-valuemin={0} aria-valuemax={s.replay_secs} aria-valuenow={Math.round(ready)}>
            <div class="fillbar" style:transform="scaleX({fill})"></div>
          </div>
          <div class="row small">
            <span><b>{live ? fmtDuration(ready) : '—'}</b> <span class="muted">ready of</span></span>
            <select class="select inline" aria-label="Replay length" value={String(s.replay_secs)} onchange={ev => updateSettings({ replay_secs: Number((ev.target as HTMLSelectElement).value) })}>
              {#each lengths as d (d)}<option value={String(d)}>{fmtDuration(d)}</option>{/each}
              {#if !lengths.includes(s.replay_secs)}<option value={String(s.replay_secs)}>{fmtDuration(s.replay_secs)}</option>{/if}
            </select>
          </div>
        </div>
      {/if}
    </div>

    <div class="actions">
      <label class="master row">
        <span class="small muted">Recording</span>
        <Toggle checked={s.replay_enabled} label="Instant replay" onchange={setReplay} />
      </label>
      {#if s.replay_enabled}
        <button class="btn primary lg save" disabled={!live} onclick={saveReplay}>
          <Icon name="save" size={20} />{saving ? 'Saving…' : 'Save replay'}
          {#if hk('save_replay')}<kbd class="onaccent">{hk('save_replay')}</kbd>{/if}
        </button>
        <div class="row sec">
          <button class="btn" disabled={!live} onclick={addMarker} title="Bookmark this moment{hk('add_marker') ? ` (${hk('add_marker')})` : ''}"><Icon name="marker" size={16} />Marker</button>
          <button class="btn" class:recording={sessionOn} disabled={!live && !sessionOn} onclick={toggleSession} title="Record a full session of any length">
            <span class="recdot" aria-hidden="true"></span>{sessionOn ? 'Stop' : 'Record'}
          </button>
          <button class="btn" disabled={!live} onclick={() => onnavigate('replay')}><Icon name="replay" size={16} />Review</button>
        </div>
      {:else}
        <button class="btn primary lg save" onclick={() => setReplay(true)}><Icon name="record" size={16} />Turn on recording</button>
      {/if}
    </div>
  </section>

  <div class="grid">
    <section class="panel">
      <h2 class="panel-title">What to record</h2>
      <SourcePicker />
    </section>
    <section class="panel">
      <h2 class="panel-title">Audio</h2>
      <AudioSources compact />
      <p class="faint xs note">Each source is also saved as its own track for editing.</p>
    </section>
  </div>

  <section class="panel">
    <div class="row head">
      <h2 class="panel-title grow">Recent clips</h2>
      <button class="btn ghost sm" onclick={() => onnavigate('library')}>View all</button>
    </div>
    <RecentClips count={4} />
  </section>

  <footer class="meta row wrap small">
    <span><span class="faint">Quality</span> {s.quality[0].toUpperCase() + s.quality.slice(1)}{measured ? ` · ≈${fmtBytes(measured)}/min` : ''}</span>
    <span class="sep"></span>
    <button class="link plain" onclick={openCaptureFolder} title="Open the capture folder"><span class="faint">Saving to</span> <span class="mono">{s.capture_root}</span></button>
    {#if e?.free_bytes != null}<span class="sep"></span><span><span class="faint">Free</span> {fmtBytes(e.free_bytes)}</span>{/if}
    <span class="sep"></span>
    <span><span class="faint">Background recording</span> {fmtBytes(e?.buffer_bytes ?? 0)} of {s.buffer_cap_gb} GB</span>
    <span class="grow"></span>
    <button class="btn ghost sm" onclick={() => onnavigate('settings')}><Icon name="settings" size={16} />Recording settings</button>
  </footer>
</div>

<style>
  .home { display: flex; flex-direction: column; gap: var(--s4); }

  .hero { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--s6); align-items: center;
          padding: var(--s6); border-radius: var(--r-lg); background: var(--surface); position: relative; overflow: hidden; }
  .hero::before { content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: 3px; background: var(--line-strong); }
  .hero.tone-live::before, .hero.tone-rec::before { background: var(--rec); }
  .hero.tone-warn::before { background: var(--warn); }
  .kicker { display: flex; align-items: center; gap: var(--s2); font-size: var(--fs-sm); font-weight: 600; color: var(--text-2); }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--text-3); }
  .tone-live .dot, .tone-rec .dot { background: var(--rec); box-shadow: 0 0 0 4px var(--rec-soft); animation: pulse 2s ease-in-out infinite; }
  .tone-live .kicker, .tone-rec .kicker { color: var(--rec); }
  .tone-warn .dot { background: var(--warn); } .tone-warn .kicker { color: var(--warn); }
  .tone-off .dot { background: transparent; border: 1.5px solid var(--text-3); }
  @keyframes pulse { 50% { box-shadow: 0 0 0 7px transparent; } }
  @media (prefers-reduced-motion: reduce) { .dot { animation: none !important; } }
  .title { font-size: var(--fs-2xl); font-weight: 650; letter-spacing: -0.01em; margin: var(--s1) 0 var(--s1); }
  .detail { color: var(--text-2); font-size: var(--fs-sm); max-width: 60ch; }
  .ready { margin-top: var(--s4); max-width: 420px; display: flex; flex-direction: column; gap: var(--s2); }
  .bar { height: 6px; border-radius: 3px; background: var(--meter-bg); overflow: hidden; }
  .fillbar { height: 100%; background: var(--text-2); transform-origin: left; transition: transform 400ms linear; }
  .tone-live .fillbar, .tone-rec .fillbar { background: var(--text); }
  .last { gap: var(--s2); margin-top: var(--s3); color: var(--ok); min-width: 0; }
  .last .link { min-width: 0; max-width: 40ch; }
  .last .faint { white-space: nowrap; }
  .select.inline { height: var(--h-sm); font-size: var(--fs-sm); padding-left: var(--s2); }

  .actions { display: flex; flex-direction: column; align-items: stretch; gap: var(--s3); min-width: 260px; }
  .master { justify-content: flex-end; gap: var(--s2); cursor: pointer; }
  .save { justify-content: center; }
  .save kbd.onaccent { background: rgba(255, 255, 255, 0.18); border-color: transparent; color: #fff; margin-left: var(--s1); }
  .sec { gap: var(--s2); }
  .sec .btn { flex: 1; padding: 0 var(--s3); }
  .recdot { width: 10px; height: 10px; border-radius: 50%; background: var(--rec); }
  .btn.recording { border-color: var(--rec); color: var(--rec); }
  .btn.recording .recdot { border-radius: 2px; }

  .grid { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1.25fr); gap: var(--s4); }
  .note { margin-top: var(--s2); }
  .head { margin-bottom: var(--s1); }
  .head .panel-title { margin-bottom: 0; }

  .meta { gap: var(--s2) var(--s3); color: var(--text-2); padding: 0 var(--s1); }
  .sep { width: 3px; height: 3px; border-radius: 50%; background: var(--text-3); }
  .plain { color: inherit; } .plain:hover { color: var(--text); text-decoration: none; }

  @media (max-width: 1080px) { .grid { grid-template-columns: minmax(0, 1fr); } }
  @media (max-width: 820px) { .hero { grid-template-columns: minmax(0, 1fr); } .actions { min-width: 0; } .master { justify-content: flex-start; } }
</style>
