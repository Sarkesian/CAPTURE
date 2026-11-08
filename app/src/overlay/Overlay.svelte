<script lang="ts">
  // The in-game overlay: one compact home screen, plus Review / clip player / quick settings.
  // Everything here talks to the same engine and settings store as the main window.
  import { onMount } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { open } from '@tauri-apps/plugin-dialog';
  import {
    app, init, saveReplay, addMarker, toggleSession, setReplay, updateSettings, refreshClips, requestClipInfo,
    openPath, revealPath, deleteClip, openCaptureFolder, showMain, hideOverlay, releaseReview, fmtDuration, fmtBytes,
  } from '../lib/store.svelte';
  import Meter from '../components/Meter.svelte';
  import Toggle from '../components/Toggle.svelte';
  import Segmented from '../components/Segmented.svelte';
  import ReviewPanel from './ReviewPanel.svelte';
  import type { ClipEntry, Quality } from '../lib/types';

  type View = 'home' | 'review' | 'player' | 'quick';
  let view = $state<View>('home');
  let playing = $state<ClipEntry | null>(null);
  let confirmDelete = $state<string | null>(null);
  let failed = $state<string | null>(null);

  const s = $derived(app.settings);
  const e = $derived(app.engine);
  const recent = $derived(app.clips.slice(0, 3));
  const sessionOn = $derived(e?.session_secs != null);
  const hk = (action: string) => e?.hotkeys.find(h => h.action === action && h.registered)?.combo ?? '';

  const status = $derived.by(() => {
    if (app.engineDown) return { label: 'Needs attention', tone: 'warn', detail: app.engineDown };
    if (!e) return { label: 'Starting…', tone: 'idle', detail: '' };
    switch (e.status) {
      case 'buffering':
        return { label: sessionOn ? 'Recording' : 'Buffering', tone: 'live', detail: `${e.target_name} · last ${fmtDuration(e.buffered_secs)} available${sessionOn ? ` · session ${fmtDuration(e.session_secs!)}` : ''}` };
      case 'ready': return { label: 'Off', tone: 'idle', detail: 'Recording is off.' };
      case 'waiting': return { label: 'Watching for games', tone: 'idle', detail: e.attention ?? '' };
      case 'starting': return { label: 'Starting…', tone: 'idle', detail: e.attention ?? '' };
      default: return { label: 'Needs attention', tone: 'warn', detail: e.attention ?? '' };
    }
  });
  const live = $derived(e?.status === 'buffering');
  const gameTrack = $derived(e?.tracks.find(t => t.name === 'Game Audio' || t.name === 'System Audio'));
  const presets = [30, 60, 120, 300, 600, 1200];

  onMount(() => {
    init().catch(err => (failed = String(err)));
    const un = listen('overlay_shown', () => { refreshClips(); if (view === 'player') view = 'home'; });
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== 'Escape') return;
      ev.preventDefault();
      if (view === 'home') hideOverlay();
      else back();
    };
    // Hidden overlay: stop playback and release the review snapshot (frees protected footage).
    const onVis = () => {
      if (document.hidden) {
        document.querySelectorAll('video').forEach(v => v.pause());
        if (app.review) releaseReview();
        if (view === 'review' || view === 'player') view = 'home';
      }
    };
    window.addEventListener('keydown', onKey);
    document.addEventListener('visibilitychange', onVis);
    return () => { un.then(f => f()); window.removeEventListener('keydown', onKey); document.removeEventListener('visibilitychange', onVis); };
  });

  $effect(() => { for (const c of recent) requestClipInfo(c.path); });

  function back() {
    if (view === 'review') releaseReview();
    view = 'home';
    playing = null;
  }

  async function changeFolder() {
    const dir = await open({ directory: true, title: 'Choose where CAPTURE saves clips', defaultPath: s?.capture_root });
    if (typeof dir === 'string') updateSettings({ capture_root: dir });
  }
  const fmtDate = (ms: number) => new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
</script>

<div class="ov">
  <header>
    <span class="dot tone-{status.tone}" aria-hidden="true"></span>
    <div class="grow">
      <div class="st">{status.label}</div>
      <div class="muted small ellipsis">{status.detail}</div>
    </div>
    {#if view !== 'home'}<button onclick={back}>← Back</button>{/if}
    <button class="x" aria-label="Close overlay (Esc)" title="Close (Esc)" onclick={() => hideOverlay()}>✕</button>
  </header>

  {#if failed}
    <div class="card">The overlay could not load: {failed}</div>
  {:else if !s}
    <p class="muted">Loading…</p>
  {:else if view === 'review'}
    <ReviewPanel onclose={back} />
  {:else if view === 'player' && playing}
    <section class="player">
      <!-- svelte-ignore a11y_media_has_caption -->
      <video src={convertFileSrc(playing.path)} controls autoplay></video>
      <div class="row">
        <div class="grow ellipsis">{playing.game} · {playing.name}</div>
        <button onclick={() => revealPath(playing!.path)}>Show in folder</button>
        <button onclick={() => openPath(playing!.path)}>Open in default player</button>
      </div>
    </section>
  {:else if view === 'quick'}
    <section class="quick card">
      <div class="field"><span class="grow">Recording</span><Toggle checked={s.replay_enabled} label="Recording" onchange={setReplay} /></div>
      <div class="field">
        <span class="grow">Save Replay length</span>
        <select value={s.replay_secs} onchange={ev => updateSettings({ replay_secs: Number((ev.target as HTMLSelectElement).value) })}>
          {#each [30, 60, 120, 180, 300, 600, 900, 1200] as d (d)}<option value={d}>{fmtDuration(d)}</option>{/each}
        </select>
      </div>
      <div class="field">
        <span class="grow">Quality</span>
        <Segmented label="Quality" value={s.quality}
          options={[{ value: 'balanced', label: 'Balanced' }, { value: 'high', label: 'High' }, { value: 'maximum', label: 'Maximum' }] as { value: Quality; label: string }[]}
          onchange={v => updateSettings({ quality: v })} />
      </div>
      <div class="field"><span class="grow">Game sound</span><Toggle checked={s.game_audio} label="Game sound" onchange={v => updateSettings({ game_audio: v })} /></div>
      <div class="field"><span class="grow">Marker sound</span><Toggle checked={s.markers.sound} label="Marker sound" onchange={v => updateSettings({ markers: { ...s.markers, sound: v } })} /></div>
      <div class="field"><span class="grow">On-screen notices</span><Toggle checked={s.notify_on_save} label="On-screen notices" onchange={v => updateSettings({ notify_on_save: v })} /></div>
      <div class="field">
        <div class="grow"><div>Saving to</div><div class="mono muted ellipsis">{s.capture_root}</div></div>
        <button onclick={changeFolder}>Change…</button>
      </div>
      <div class="row" style="justify-content:flex-end; margin-top:8px">
        <button onclick={() => showMain('settings')}>All settings…</button>
      </div>
    </section>
  {:else}
    <div class="home">
      <section class="actions">
        <button class="act primary" disabled={!live} onclick={saveReplay}>
          <span class="big">Save Replay</span>
          <span class="sub">Last {fmtDuration(s.replay_secs)}{hk('save_replay') ? ` · ${hk('save_replay')}` : ''}</span>
        </button>
        <button class="act" disabled={!live} onclick={addMarker}>
          <span class="big">Add Marker</span>
          <span class="sub">Bookmark this moment{hk('add_marker') ? ` · ${hk('add_marker')}` : ''}</span>
        </button>
        <button class="act" disabled={!live} onclick={() => (view = 'review')}>
          <span class="big">Review Recording</span>
          <span class="sub">{e?.markers.length ? `${e.markers.length} marker${e.markers.length > 1 ? 's' : ''} · ` : ''}find a moment, save it</span>
        </button>
        <button class="act" class:rec={sessionOn} disabled={!live && !sessionOn} onclick={toggleSession}>
          <span class="big">{sessionOn ? 'Stop Recording' : 'Start Recording'}</span>
          <span class="sub">{sessionOn ? `Recording ${fmtDuration(e!.session_secs!)} · saves to Sessions` : 'Full session, any length'}</span>
        </button>
      </section>

      {#if !s.replay_enabled}
        <div class="card row off">
          <div class="grow">Recording is off. Turn it on to keep your recent gameplay.</div>
          <button class="primary" onclick={() => setReplay(true)}>Turn on</button>
        </div>
      {/if}

      <section class="card strip">
        <span class="muted small">Save Replay length</span>
        <div class="chips">
          {#each presets as p (p)}
            <button class:on={s.replay_secs === p} onclick={() => updateSettings({ replay_secs: p })}>{fmtDuration(p)}</button>
          {/each}
        </div>
        {#if gameTrack}
          <span class="muted small" style="margin-left:auto">{gameTrack.name === 'System Audio' ? 'Computer sound' : 'Game sound'}</span>
          <div style="width:160px"><Meter peak={gameTrack.peak} label="Game sound level" /></div>
        {/if}
      </section>

      <section class="card recent">
        <div class="row">
          <h2 class="grow">Recent clips</h2>
          <button class="link" onclick={() => showMain('library')}>View all</button>
        </div>
        {#if recent.length === 0}
          <p class="muted small">No clips yet. Press Save Replay{hk('save_replay') ? ` (${hk('save_replay')})` : ''} during a game.</p>
        {/if}
        <div class="clips">
          {#each recent as c (c.path)}
            {@const info = app.clipInfo[c.path]}
            <article class="clip">
              <button class="thumb" aria-label="Play {c.name}" onclick={() => { playing = c; view = 'player'; }}>
                {#if info?.thumb}<img src={convertFileSrc(info.thumb)} alt="" />{:else}<span class="muted small">…</span>{/if}
                {#if info?.duration}<span class="dur">{fmtDuration(info.duration)}</span>{/if}
              </button>
              <div class="meta">
                <div class="ellipsis">{c.game}</div>
                <div class="muted small">{fmtDate(c.modified_ms)} · {fmtBytes(c.bytes)}</div>
              </div>
              <div class="row tight">
                <button onclick={() => { playing = c; view = 'player'; }}>Play</button>
                <button onclick={() => revealPath(c.path)}>Folder</button>
                {#if confirmDelete === c.path}
                  <button class="danger" onclick={() => { deleteClip(c.path); confirmDelete = null; }}>Move to Recycle Bin?</button>
                {:else}
                  <button onclick={() => (confirmDelete = c.path)}>Delete</button>
                {/if}
              </div>
            </article>
          {/each}
        </div>
      </section>

      <footer class="row">
        <button onclick={() => (view = 'quick')}>Quick settings</button>
        <button onclick={openCaptureFolder}>Open folder</button>
        <button onclick={() => showMain('settings')}>Full settings</button>
        <span class="muted small grow" style="text-align:right">
          {s.capture_root} · {e?.free_bytes != null ? `${fmtBytes(e.free_bytes)} free` : ''} · temporary {fmtBytes(e?.buffer_bytes ?? 0)}
        </span>
      </footer>
    </div>
  {/if}

  <div class="toasts" aria-live="polite">
    {#each app.toasts as t (t.id)}<div class="toast {t.kind}">{t.text}</div>{/each}
  </div>
</div>

<style>
  :global(html.overlay), :global(html.overlay body) { background: #0f1115; overflow: hidden; }
  .ov { height: 100vh; display: flex; flex-direction: column; padding: 14px 18px; gap: 12px; border: 1px solid #2c313b; }
  header { display: flex; align-items: center; gap: 12px; }
  .dot { width: 12px; height: 12px; border-radius: 50%; background: #6b7280; flex: none; }
  .dot.tone-live { background: var(--accent); box-shadow: 0 0 0 4px #ef444433; }
  .dot.tone-warn { background: var(--warn); }
  .st { font-weight: 700; font-size: 17px; }
  .x { width: 36px; padding: 6px; }
  .home { display: flex; flex-direction: column; gap: 12px; min-height: 0; flex: 1; }
  .actions { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .act { display: flex; flex-direction: column; align-items: flex-start; gap: 3px; padding: 16px 18px; border-radius: 12px; text-align: left; }
  .act .big { font-size: 19px; font-weight: 650; }
  .act .sub { font-size: 13px; color: var(--muted); }
  .act.primary .sub { color: #ffe4e4; }
  .act.rec { border-color: var(--accent); }
  .off { gap: 12px; }
  .strip { display: flex; align-items: center; gap: 12px; padding: 10px 14px; flex-wrap: wrap; }
  .chips { display: flex; gap: 6px; }
  .chips button { padding: 4px 10px; font-size: 13px; }
  .chips button.on { border-color: var(--accent); color: var(--text); background: #3a1f22; }
  .recent { flex: 1; min-height: 0; }
  .recent h2 { margin: 0; }
  .clips { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; margin-top: 8px; }
  .clip { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .thumb { position: relative; padding: 0; aspect-ratio: 16 / 9; overflow: hidden; display: grid; place-items: center; background: #000; }
  .thumb img { width: 100%; height: 100%; object-fit: cover; }
  .dur { position: absolute; right: 6px; bottom: 6px; background: #000c; font-size: 12px; padding: 1px 6px; border-radius: 4px; }
  .meta { min-width: 0; }
  .tight { gap: 6px; }
  .tight button { padding: 4px 8px; font-size: 13px; }
  .danger { border-color: var(--accent); color: var(--accent-2); }
  footer { gap: 8px; }
  .player { display: flex; flex-direction: column; gap: 10px; flex: 1; min-height: 0; }
  .player video { width: 100%; flex: 1; min-height: 0; background: #000; border-radius: 10px; }
  .quick { max-width: 640px; }
  .quick .field { display: flex; align-items: center; gap: 12px; padding: 8px 0; border-top: 1px solid var(--line); }
  .quick .field:first-child { border-top: none; }
  .toasts { position: fixed; right: 18px; top: 70px; display: flex; flex-direction: column; gap: 8px; max-width: 420px; }
  .toast { background: #262b35; border: 1px solid var(--line); border-left: 4px solid var(--focus); padding: 8px 12px; border-radius: 8px; }
  .toast.ok { border-left-color: var(--ok); }
  .toast.error { border-left-color: var(--accent); }
</style>
