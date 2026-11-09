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
  import Icon from '../components/Icon.svelte';
  import Toggle from '../components/Toggle.svelte';
  import Segmented from '../components/Segmented.svelte';
  import ReviewPanel from './ReviewPanel.svelte';
  import AudioSources from '../components/AudioSources.svelte';
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
        return { label: sessionOn ? `Recording ${fmtDuration(e.session_secs!)}` : 'Replay on', tone: 'live', detail: `${e.target_name} · last ${fmtDuration(e.buffered_secs)} ready` };
      case 'ready': return { label: 'Off', tone: 'idle', detail: 'Recording is off.' };
      case 'waiting': return { label: 'Watching for games', tone: 'idle', detail: e.attention ?? '' };
      case 'starting': return { label: 'Starting…', tone: 'idle', detail: e.attention ?? '' };
      default: return { label: 'Needs attention', tone: 'warn', detail: e.attention ?? '' };
    }
  });
  const live = $derived(e?.status === 'buffering');
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

<div class="ov" style:zoom={(s?.overlay_scale ?? 100) / 100}>
  <header>
    <span class="dot tone-{status.tone}" aria-hidden="true"></span>
    <div class="grow hd">
      <div class="st">{status.label}</div>
      <div class="muted small ellipsis">{status.detail}</div>
    </div>
    {#if view !== 'home'}<button class="btn" onclick={back}><Icon name="back" size={16} />Back</button>{/if}
    <button class="btn icon ghost" aria-label="Close overlay (Esc)" title="Close (Esc)" onclick={() => hideOverlay()}><Icon name="x" size={18} /></button>
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
        <button class="btn" onclick={() => revealPath(playing!.path)}><Icon name="folder" size={16} />Show in folder</button>
        <button class="btn" onclick={() => openPath(playing!.path)}>Open in player</button>
      </div>
    </section>
  {:else if view === 'quick'}
    <section class="quick card">
      <div class="field"><span class="grow">Recording</span><Toggle checked={s.replay_enabled} label="Recording" onchange={setReplay} /></div>
      <div class="field">
        <span class="grow">Replay length</span>
        <select class="select" value={String(s.replay_secs)} aria-label="Replay length" onchange={ev => updateSettings({ replay_secs: Number((ev.target as HTMLSelectElement).value) })}>
          {#each [30, 60, 120, 180, 300, 600, 900, 1200] as d (d)}<option value={String(d)}>{fmtDuration(d)}</option>{/each}
        </select>
      </div>
      <div class="field">
        <span class="grow">Quality</span>
        <Segmented label="Quality" value={s.quality}
          options={[{ value: 'balanced', label: 'Balanced' }, { value: 'high', label: 'High' }, { value: 'maximum', label: 'Maximum' }] as { value: Quality; label: string }[]}
          onchange={v => updateSettings({ quality: v })} />
      </div>
      <div class="field audio-full"><AudioSources /></div>
      <div class="field"><span class="grow">Hotkey sound</span><Toggle checked={s.markers.sound} label="Hotkey sound" onchange={v => updateSettings({ markers: { ...s.markers, sound: v } })} /></div>
      <div class="field"><span class="grow">On-screen notices</span><Toggle checked={s.notify_on_save} label="On-screen notices" onchange={v => updateSettings({ notify_on_save: v })} /></div>
      <div class="field">
        <div class="grow"><div>Saving to</div><div class="mono faint xs ellipsis">{s.capture_root}</div></div>
        <button class="btn" onclick={changeFolder}>Change…</button>
      </div>
      <div class="row end">
        <button class="btn ghost" onclick={() => showMain('settings')}>All settings<Icon name="chevron" size={14} /></button>
      </div>
    </section>
  {:else}
    <div class="home">
      <section class="actions">
        <button class="act primary" disabled={!live} onclick={saveReplay}>
          <Icon name="save" size={22} />
          <span class="txt"><span class="big">Save Replay</span>
          <span class="sub">Last {fmtDuration(s.replay_secs)}{hk('save_replay') ? ` · ${hk('save_replay')}` : ''}</span></span>
        </button>
        <button class="act" disabled={!live} onclick={addMarker}>
          <Icon name="marker" size={22} />
          <span class="txt"><span class="big">Add Marker</span>
          <span class="sub">Bookmark this moment{hk('add_marker') ? ` · ${hk('add_marker')}` : ''}</span></span>
        </button>
        <button class="act" disabled={!live} onclick={() => (view = 'review')}>
          <Icon name="replay" size={22} />
          <span class="txt"><span class="big">Review Recording</span>
          <span class="sub">{e?.markers.length ? `${e.markers.length} marker${e.markers.length > 1 ? 's' : ''} · ` : ''}find a moment, save it</span></span>
        </button>
        <button class="act" class:rec={sessionOn} disabled={!live && !sessionOn} onclick={toggleSession}>
          <span class="recdot" aria-hidden="true"></span>
          <span class="txt"><span class="big">{sessionOn ? 'Stop Recording' : 'Start Recording'}</span>
          <span class="sub">{sessionOn ? `${fmtDuration(e!.session_secs!)} · saves to Sessions` : 'Full session, any length'}</span></span>
        </button>
      </section>

      {#if !s.replay_enabled}
        <div class="card row off">
          <div class="grow">Recording is off. Turn it on to keep your recent gameplay.</div>
          <button class="btn primary" onclick={() => setReplay(true)}>Turn on</button>
        </div>
      {/if}

      <div class="two">
        <section class="card">
          <h2>Audio</h2>
          <AudioSources compact />
        </section>
        <section class="card">
          <h2>Replay length</h2>
          <div class="chips" role="radiogroup" aria-label="Replay length">
            {#each presets as p (p)}
              <button role="radio" aria-checked={s.replay_secs === p} class:on={s.replay_secs === p} onclick={() => updateSettings({ replay_secs: p })}>{fmtDuration(p)}</button>
            {/each}
          </div>
        </section>
      </div>

      <section class="card recent">
        <div class="row">
          <h2 class="grow">Recent clips</h2>
          <button class="btn ghost sm" onclick={() => showMain('library')}>View all</button>
        </div>
        {#if recent.length === 0}
          <p class="muted small">No clips yet. Press Save Replay{hk('save_replay') ? ` (${hk('save_replay')})` : ''} during a game.</p>
        {/if}
        <div class="clips">
          {#each recent as c (c.path)}
            {@const info = app.clipInfo[c.path]}
            <article class="clip">
              <button class="thumb" aria-label="Play {c.name}" onclick={() => { playing = c; view = 'player'; }}>
                <span class="play"><Icon name="play" size={18} /></span>
                {#if info?.duration}<span class="dur">{fmtDuration(info.duration)}</span>{/if}
              </button>
              <div class="row meta">
                <div class="grow">
                  <div class="ellipsis small">{c.game}</div>
                  <div class="faint xs">{fmtDate(c.modified_ms)} · {fmtBytes(c.bytes)}</div>
                </div>
                <button class="btn ghost sm icon" title="Show in folder" aria-label="Show {c.name} in folder" onclick={() => revealPath(c.path)}><Icon name="folder" size={15} /></button>
                {#if confirmDelete === c.path}
                  <button class="btn sm danger" onclick={() => { deleteClip(c.path); confirmDelete = null; }}>Delete?</button>
                {:else}
                  <button class="btn ghost sm icon" title="Delete" aria-label="Delete {c.name}" onclick={() => (confirmDelete = c.path)}><Icon name="trash" size={15} /></button>
                {/if}
              </div>
            </article>
          {/each}
        </div>
      </section>

      <footer class="row">
        <button class="btn ghost sm" onclick={() => (view = 'quick')}><Icon name="settings" size={15} />Quick settings</button>
        <button class="btn ghost sm" onclick={openCaptureFolder}><Icon name="folder" size={15} />Open folder</button>
        <span class="faint xs grow ellipsis right">
          {s.capture_root}{e?.free_bytes != null ? ` · ${fmtBytes(e.free_bytes)} free` : ''} · temporary {fmtBytes(e?.buffer_bytes ?? 0)}
        </span>
      </footer>
    </div>
  {/if}

  <div class="toasts" aria-live="polite">
    {#each app.toasts as t (t.id)}<div class="toast {t.kind}">{t.text}</div>{/each}
  </div>
</div>

<style>
  :global(html.overlay), :global(html.overlay body) { background: var(--bg); overflow: hidden; }
  .ov { height: 100vh; display: flex; flex-direction: column; padding: var(--s4) var(--s5); gap: var(--s3); border: 1px solid var(--line-strong); }
  header { display: flex; align-items: center; gap: var(--s3); }
  .hd { min-width: 0; }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: var(--text-3); flex: none; }
  .dot.tone-live { background: var(--rec); box-shadow: 0 0 0 4px var(--rec-soft); }
  .dot.tone-warn { background: var(--warn); }
  .st { font-weight: 650; font-size: var(--fs-lg); }
  .home { display: flex; flex-direction: column; gap: var(--s3); min-height: 0; flex: 1; }
  .card { background: var(--surface); border-radius: var(--r-lg); padding: var(--s4); }
  h2 { font-size: var(--fs-sm); font-weight: 600; color: var(--text-2); margin: 0 0 var(--s2); }
  .actions { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s2); }
  .act { display: flex; align-items: center; gap: var(--s4); padding: var(--s4) var(--s5); border-radius: var(--r-lg); text-align: left;
         border: 1px solid var(--line); background: var(--surface); color: var(--text-2); cursor: pointer;
         transition: background var(--t-fast) var(--ease), border-color var(--t-fast) var(--ease), transform var(--t-fast) var(--ease); }
  .act:hover:not(:disabled) { background: var(--surface-2); border-color: var(--line-strong); }
  .act:active:not(:disabled) { transform: scale(0.985); }
  .act:disabled { opacity: 0.45; cursor: default; }
  .act .txt { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .act .big { font-size: var(--fs-lg); font-weight: 650; color: var(--text); }
  .act .sub { font-size: var(--fs-sm); color: var(--text-3); }
  .act.primary { background: var(--accent); border-color: transparent; color: var(--text-on-accent); }
  .act.primary .big, .act.primary .sub { color: var(--text-on-accent); }
  .act.primary .sub { opacity: 0.85; }
  .act.primary:hover:not(:disabled) { background: var(--accent-hover); }
  .recdot { width: 16px; height: 16px; margin: 0 3px; border-radius: 50%; background: var(--rec); flex: none; }
  .act.rec { border-color: var(--rec); }
  .act.rec .recdot { border-radius: 3px; }
  .off { gap: var(--s3); }
  .two { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--s3); align-items: start; }
  .two .card:last-child .chips { display: grid; grid-template-columns: repeat(2, auto); }
  .audio-full { display: block; }
  .chips { display: flex; gap: var(--s1); flex-wrap: wrap; }
  .chips button { height: var(--h-sm); padding: 0 var(--s3); border: 1px solid var(--line); border-radius: var(--r-pill); background: none; color: var(--text-2); font-size: var(--fs-sm); cursor: pointer; }
  .chips button:hover { color: var(--text); border-color: var(--line-strong); }
  .chips button.on { background: var(--accent-soft); border-color: var(--accent); color: var(--text); }
  .recent { min-height: 0; }
  .recent h2 { margin: 0; }
  .clips { display: grid; grid-template-columns: repeat(3, minmax(0, 240px)); gap: var(--s3); margin-top: var(--s2); }
  .clip { display: flex; flex-direction: column; gap: var(--s1); min-width: 0; }
  .thumb { position: relative; padding: 0; aspect-ratio: 16 / 9; overflow: hidden; border: none; border-radius: var(--r-md); background: var(--surface-2); cursor: pointer; }
  .play { position: absolute; inset: 0; display: grid; place-items: center; color: var(--text-2); transition: color var(--t-fast) var(--ease); }
  .thumb:hover .play, .thumb:focus-visible .play { color: var(--text); }
  .dur { position: absolute; right: 6px; bottom: 6px; background: rgba(0, 0, 0, 0.72); color: #fff; font-size: var(--fs-xs); padding: 0 6px; border-radius: 4px; }
  .meta { gap: 2px; min-width: 0; }
  .meta .grow { min-width: 0; }
  .btn.danger { color: var(--danger); }
  footer { gap: var(--s2); }
  .right { text-align: right; }
  .end { justify-content: flex-end; margin-top: var(--s2); }
  .player { display: flex; flex-direction: column; gap: var(--s3); flex: 1; min-height: 0; }
  .player video { width: 100%; flex: 1; min-height: 0; background: #000; border-radius: var(--r-lg); }
  .quick { max-width: 680px; overflow: auto; }
  .quick .field { display: flex; align-items: center; gap: var(--s3); padding: var(--s3) 0; border-top: 1px solid var(--line); }
  .quick .field:first-child { border-top: none; padding-top: 0; }
  .toasts { top: 72px; bottom: auto; }
  @media (max-width: 760px) { .two { grid-template-columns: minmax(0, 1fr); } }
</style>
