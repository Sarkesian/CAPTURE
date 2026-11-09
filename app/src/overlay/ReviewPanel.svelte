<script lang="ts">
  // Review Recording: watch recent gameplay while recording continues, jump between markers,
  // pick a range with two handles and save it. A replay tool, not an editor.
  import { onMount } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { app, prepareReview, saveRange, engine, fmtDuration } from '../lib/store.svelte';
  import type { Marker } from '../lib/types';
  import Icon from '../components/Icon.svelte';

  let { onclose }: { onclose: () => void } = $props();

  let video = $state<HTMLVideoElement | null>(null);
  let track = $state<HTMLDivElement | null>(null);
  let t = $state(0);
  let dur = $state(0);
  let paused = $state(true);
  // The clip being made, in recording time (seconds since the recording started), so it stays
  // put when newer footage is loaded.
  let sel = $state<{ a: number; b: number } | null>(null);
  let drag: 'seek' | 'a' | 'b' | null = null;
  let renaming = $state<number | null>(null);
  // Where to be once the footage being loaded is ready (recording time; Infinity = the latest
  // moment) and whether to play from there. Replay opens at the latest moment.
  let want: { abs: number; play: boolean } | null = { abs: Infinity, play: false };

  const rv = $derived(app.review);
  const s = $derived(app.settings!);
  // The loaded footage is a snapshot; the recording goes on. `total` is how far the recording
  // has got since the snapshot's start, so the timeline keeps growing to the right and the part
  // beyond `dur` is footage that is loaded when the player gets there.
  // (A previous recording, watched while nothing is being recorded, is complete as it is.)
  const total = $derived(rv && !rv.past ? Math.max(dur, (app.engine?.now_secs ?? 0) - rv.start, 0.001) : Math.max(dur, 0.001));
  const fresh = $derived(Math.max(0, total - dur));
  const markers = $derived<(Marker & { rel: number })[]>(
    rv ? (rv.past ? rv.markers ?? [] : app.engine?.markers ?? []).filter(m => m.t >= rv.start && m.t <= rv.start + total).map(m => ({ ...m, rel: m.t - rv.start })) : []);
  // What has already been saved from this stretch: clips as bars, screenshots as dots.
  const saved = $derived(rv && !rv.past ? (app.engine?.saved ?? []).filter(x => (x.end ?? x.t) >= rv.start && x.t <= rv.start + total) : []);
  const before = $derived(Math.max(5, Math.min(60, s.markers.protect_before_secs || 20)));
  const after = $derived(Math.max(3, Math.min(60, s.markers.protect_after_secs || 10)));

  onMount(() => {
    prepareReview();
    const onKey = (ev: KeyboardEvent) => {
      // Leave keys to fields and lists that use them, and Space to a focused button.
      const t = ev.target as HTMLElement;
      if (t?.closest?.('input, textarea, select, [contenteditable]')) return;
      if (ev.key === ' ' && t?.closest?.('button')) return;
      if (ev.key === ' ') { ev.preventDefault(); toggle(); }
      if (ev.key === 'ArrowLeft') { ev.preventDefault(); step(-5); }
      if (ev.key === 'ArrowRight') { ev.preventDefault(); step(5); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const clamp = (x: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, x));
  const fmt = (x: number) => { const v = Math.max(0, Math.round(x)); return `${Math.floor(v / 60)}:${String(v % 60).padStart(2, '0')}`; };
  const pct = (x: number) => clamp((x / total) * 100, 0, 100);
  const rel = (abs: number) => abs - (rv?.start ?? 0);

  // Newly loaded footage is ready: go where the player was heading.
  function loaded() {
    if (!video || !rv || !want) return;
    const w = want;
    want = null;
    const end = video.duration || 0;
    video.currentTime = w.abs === Infinity ? Math.max(0, end - 3) : clamp(w.abs - rv.start, 0, Math.max(0, end - 0.05));
    if (w.play) video.play().catch(() => {});
  }
  // Load the footage recorded since the snapshot and continue at `abs`.
  function catchUp(abs: number, play: boolean) {
    if (app.reviewLoading) return;
    want = { abs, play };
    prepareReview();
  }
  function seek(x: number) {
    if (!video || !rv) return;
    // Into footage newer than the snapshot (it needs a couple of seconds to be on disk).
    if (x > dur + 0.25 && fresh > 2.5) catchUp(rv.start + Math.min(x, total - 2.5), !video.paused);
    else video.currentTime = clamp(x, 0, dur || 0);
  }
  function step(d: number) { seek(t + d); }
  function toggle() { if (video) (video.paused ? video.play() : video.pause()); }
  function live() { sel = null; catchUp(Infinity, false); }
  // Played to the end of the snapshot: carry on into what was recorded meanwhile.
  function ended() { if (rv && fresh > 2.5) catchUp(rv.start + dur, true); }
  function at(ev: PointerEvent) {
    const r = track!.getBoundingClientRect();
    return clamp(((ev.clientX - r.left) / r.width) * total, 0, total);
  }
  function down(ev: PointerEvent, what: 'seek' | 'a' | 'b') {
    ev.stopPropagation();
    drag = what;
    (ev.currentTarget as HTMLElement).setPointerCapture(ev.pointerId);
    move(ev);
  }
  function move(ev: PointerEvent) {
    if (!drag || !rv) return;
    const x = at(ev);
    if (drag === 'seek') { if (x <= dur + 0.25) seek(x); }
    else if (sel && drag === 'a') sel.a = Math.min(rv.start + x, sel.b - 1);
    else if (sel && drag === 'b') sel.b = Math.max(rv.start + x, sel.a + 1);
  }
  function up(ev: PointerEvent) {
    // A drag only scrubs within the loaded footage; newer footage is loaded on release.
    if (drag === 'seek' && at(ev) > dur + 0.25) seek(at(ev));
    drag = null;
  }

  const span = (a: number, b: number) => (rv ? { a: rv.start + clamp(a, 0, total), b: rv.start + clamp(b, 0, total) } : null);
  function createClip() { sel = span(t - 15, t + 5); }
  function aroundMarker(m: { rel: number }) {
    seek(m.rel - 3);
    sel = span(m.rel - before, m.rel + after);
  }
  function jumpMarker(dir: 1 | -1) {
    const list = dir > 0 ? markers.filter(m => m.rel > t + 0.5) : markers.filter(m => m.rel < t - 0.5).reverse();
    if (list[0]) seek(list[0].rel - 3);
  }
  async function saveSel() {
    if (!sel) return;
    await saveRange(sel.a, sel.b);
    sel = null;
  }
  const saveAround = (m: Marker) => saveRange(m.t - before, m.t + after);
</script>

<section class="rv">
  {#if app.reviewError}
    <div class="card">
      <p>{app.reviewError}</p>
      <button class="btn" onclick={onclose}>Back</button>
    </div>
  {:else if !rv}
    <div class="card muted"><span class="spinner" aria-hidden="true"></span>Opening your recent gameplay… recording continues.</div>
  {:else}
    <div class="stage">
      <!-- svelte-ignore a11y_media_has_caption -->
      <video bind:this={video} src={convertFileSrc(rv.path)} bind:currentTime={t} bind:duration={dur} bind:paused onclick={toggle} onloadedmetadata={loaded} onended={ended}></video>
      {#if app.reviewLoading}<div class="busy">Loading newer footage…</div>{/if}
    </div>

    <!-- Timeline: click/drag to scrub; markers are clickable; handles appear when making a clip. -->
    <div class="timeline">
      <div class="track" bind:this={track} role="slider" tabindex="0" aria-label="Position" aria-valuemin={0} aria-valuemax={Math.round(total)} aria-valuenow={Math.round(t)}
        onpointerdown={ev => down(ev, 'seek')} onpointermove={move} onpointerup={up}>
        <div class="fill" style:width="{pct(t)}%"></div>
        {#if fresh > 0.5}<div class="fresh" style:left="{pct(dur)}%" title="Recorded since you opened Replay. Click to load it."></div>{/if}
        {#if sel}<div class="sel" style:left="{pct(rel(sel.a))}%" style:width="{pct(rel(sel.b)) - pct(rel(sel.a))}%"></div>{/if}
        {#each saved as x (x.name)}
          {#if x.kind === 'clip'}
            <div class="clipbar" style:left="{pct(rel(x.t))}%" style:width="{Math.max(0.6, pct(rel(x.end ?? x.t)) - pct(rel(x.t)))}%" title="Saved clip · {x.name}"></div>
          {:else}
            <div class="shot" style:left="{pct(rel(x.t))}%" title="Screenshot · {x.name}"></div>
          {/if}
        {/each}
        {#each markers as m (m.id)}
          <button class="mk" class:prot={m.footage === 'protected'} style:left="{pct(m.rel)}%" title="{m.at}{m.label ? ` · ${m.label}` : ''}"
            aria-label="Marker at {m.at}" onpointerdown={ev => ev.stopPropagation()} onclick={() => aroundMarker(m)}></button>
        {/each}
        <div class="head" style:left="{pct(t)}%"></div>
        {#if sel}
          <div class="handle" role="slider" tabindex="0" aria-label="Clip start" aria-valuenow={Math.round(rel(sel.a))} style:left="{pct(rel(sel.a))}%" onpointerdown={ev => down(ev, 'a')} onpointermove={move} onpointerup={up}></div>
          <div class="handle" role="slider" tabindex="0" aria-label="Clip end" aria-valuenow={Math.round(rel(sel.b))} style:left="{pct(rel(sel.b))}%" onpointerdown={ev => down(ev, 'b')} onpointermove={move} onpointerup={up}></div>
        {/if}
      </div>
      <div class="row ctl">
        <button class="btn icon ghost" title="Beginning" aria-label="Beginning" onclick={() => seek(0)}><Icon name="skip-back" size={18} /></button>
        <button class="btn ghost sm" title="Back 5 seconds (Left arrow)" aria-label="Back 5 seconds" onclick={() => step(-5)}>−5s</button>
        <button class="btn icon play" title="Play or pause (Space)" aria-label={paused ? 'Play' : 'Pause'} onclick={toggle}><Icon name={paused ? 'play' : 'pause'} size={18} /></button>
        <button class="btn ghost sm" title="Forward 5 seconds (Right arrow)" aria-label="Forward 5 seconds" onclick={() => step(5)}>+5s</button>
        {#if !rv.past}<button class="btn ghost sm" title="Jump to the latest footage" onclick={live}><Icon name="live" size={16} />Live</button>{/if}
        <span class="sep"></span>
        <button class="btn ghost sm" disabled={!markers.length} onclick={() => jumpMarker(-1)} aria-label="Previous marker" title="Previous marker"><Icon name="back" size={14} />Marker</button>
        <button class="btn ghost sm" disabled={!markers.length} onclick={() => jumpMarker(1)} aria-label="Next marker" title="Next marker">Marker<Icon name="chevron" size={14} /></button>
        <span class="mono time">{fmt(t)} <span class="faint">/ {fmt(total)}</span></span>
        <span class="grow"></span>
        {#if sel}
          <span class="muted small">Clip {fmt(rel(sel.a))}–{fmt(rel(sel.b))} · {fmtDuration(sel.b - sel.a)}</span>
          <button class="btn ghost" onclick={() => (sel = null)}>Cancel</button>
          <button class="btn primary" onclick={saveSel}>Save clip</button>
        {:else}
          <button class="btn primary" onclick={createClip}>Create clip</button>
        {/if}
      </div>
    </div>

    {#if markers.length}
      <div class="markers">
        {#each markers as m (m.id)}
          <div class="mrow">
            <button class="link" onclick={() => aroundMarker(m)}>{m.at}</button>
            {#if renaming === m.id}
              <input class="input" type="text" value={m.label} placeholder="Name (optional)" aria-label="Marker name"
                onkeydown={ev => { if (ev.key === 'Enter') { engine('rename_marker', { id: m.id, label: (ev.target as HTMLInputElement).value }); renaming = null; } if (ev.key === 'Escape') { ev.stopPropagation(); renaming = null; } }} />
            {:else}
              <span class="ellipsis grow">{m.label || 'Marker'}</span>
            {/if}
            <span class="small {m.footage === 'protected' ? 'ok' : 'muted'}" title={m.note ?? ''}>
              {rv.past ? '' : m.saved_in.length ? 'Saved in a clip' : m.footage === 'protected' ? 'Kept until saved or session ends' : m.footage === 'buffered' ? 'In the background recording' : 'Footage gone'}
            </span>
            <button class="btn sm" onclick={() => saveAround(m)} title="Save {fmtDuration(before)} before to {fmtDuration(after)} after">Save around</button>
            {#if !rv.past}
              <button class="btn ghost sm icon" title="Rename" aria-label="Rename marker" onclick={() => (renaming = m.id)}><Icon name="pencil" size={15} /></button>
              <button class="btn ghost sm icon" title="Remove" aria-label="Remove marker" onclick={() => engine('remove_marker', { id: m.id })}><Icon name="x" size={15} /></button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
    <p class="faint xs">Recording continues while you watch; new footage appears at the right. On the timeline: <span class="key mkk"></span> markers, <span class="key clk"></span> saved clips, <span class="key shk"></span> screenshots.</p>
  {/if}
</section>

<style>
  .rv { display: flex; flex-direction: column; gap: var(--s3); flex: 1; min-height: 0; }
  .card { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--s3); flex: 1; padding: var(--s8);
          border-radius: var(--r-lg); background: var(--surface); text-align: center; }
  .spinner { width: 18px; height: 18px; border-radius: 50%; border: 2px solid var(--line-strong); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .stage { position: relative; flex: 1; min-height: 160px; background: #000; border-radius: var(--r-lg); overflow: hidden; }
  video { width: 100%; height: 100%; object-fit: contain; display: block; cursor: pointer; }
  .busy { position: absolute; top: var(--s3); right: var(--s3); background: rgba(0, 0, 0, 0.75); color: #fff; padding: var(--s1) var(--s3); border-radius: var(--r-sm); font-size: var(--fs-sm); }
  .timeline { display: flex; flex-direction: column; gap: var(--s2); }
  .track { position: relative; height: 28px; background: var(--surface-2); border-radius: var(--r-sm); cursor: pointer; touch-action: none; }
  .track:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .fill { position: absolute; inset: 0 auto 0 0; background: var(--surface-3); border-radius: var(--r-sm); pointer-events: none; }
  .fresh { position: absolute; top: 0; bottom: 0; right: 0; border-radius: 0 var(--r-sm) var(--r-sm) 0; pointer-events: none;
           background: repeating-linear-gradient(135deg, transparent 0 6px, var(--surface-3) 6px 12px); opacity: 0.7; }
  .sel { position: absolute; top: 0; bottom: 0; background: var(--accent-soft); border: 1px solid var(--accent); border-radius: var(--r-sm); pointer-events: none; }
  .head { position: absolute; top: -3px; bottom: -3px; width: 2px; background: var(--text); transform: translateX(-1px); pointer-events: none; }
  .mk { position: absolute; top: 4px; width: 8px; height: 20px; padding: 0; transform: translateX(-4px); background: var(--warn); border: none; border-radius: 2px; cursor: pointer; }
  .mk.prot { box-shadow: 0 0 0 2px var(--warn-soft); }
  .key { display: inline-block; vertical-align: middle; margin: 0 2px 0 4px; }
  .mkk { width: 5px; height: 11px; background: var(--warn); border-radius: 1px; }
  .clk { width: 14px; height: 4px; background: var(--ok); border-radius: 2px; }
  .shk { width: 7px; height: 7px; background: var(--accent); transform: rotate(45deg); border-radius: 1px; }
  .clipbar { position: absolute; top: 0; height: 4px; background: var(--ok); border-radius: 2px; }
  .shot { position: absolute; bottom: 2px; width: 8px; height: 8px; transform: translateX(-4px) rotate(45deg); background: var(--accent); border-radius: 1px; }
  .handle { position: absolute; top: -4px; bottom: -4px; width: 10px; transform: translateX(-5px); background: var(--accent); border-radius: 3px; cursor: ew-resize; }
  .ctl { gap: var(--s1); flex-wrap: wrap; }
  .play { width: 44px; }
  .time { font-size: var(--fs-sm); padding: 0 var(--s2); }
  .sep { width: 1px; height: 20px; background: var(--line); margin: 0 var(--s1); }
  .markers { display: flex; flex-direction: column; max-height: 132px; overflow: auto; border-top: 1px solid var(--line); }
  .mrow { display: flex; align-items: center; gap: var(--s2); font-size: var(--fs-sm); padding: var(--s1) 0; border-bottom: 1px solid var(--line); }
  .mrow .input { height: var(--h-sm); }
  .ok { color: var(--ok); }
</style>
