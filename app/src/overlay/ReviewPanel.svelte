<script lang="ts">
  // Review Recording: watch recent gameplay while recording continues, jump between markers,
  // pick a range with two handles and save it. A replay tool, not an editor.
  import { onMount } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { app, prepareReview, saveRange, engine, fmtDuration } from '../lib/store.svelte';
  import type { Marker } from '../lib/types';

  let { onclose }: { onclose: () => void } = $props();

  let video = $state<HTMLVideoElement | null>(null);
  let track = $state<HTMLDivElement | null>(null);
  let t = $state(0);
  let dur = $state(0);
  let paused = $state(true);
  let sel = $state<{ a: number; b: number } | null>(null);
  let drag: 'seek' | 'a' | 'b' | null = null;
  let renaming = $state<number | null>(null);
  let pendingSeekEnd = false;

  const rv = $derived(app.review);
  const s = $derived(app.settings!);
  const markers = $derived<(Marker & { rel: number })[]>(
    rv ? (app.engine?.markers ?? []).filter(m => m.t >= rv.start && m.t <= rv.end).map(m => ({ ...m, rel: m.t - rv.start })) : []);
  const before = $derived(Math.max(5, Math.min(60, s.markers.protect_before_secs || 20)));
  const after = $derived(Math.max(3, Math.min(60, s.markers.protect_after_secs || 10)));

  onMount(() => {
    prepareReview();
    const onKey = (ev: KeyboardEvent) => {
      if ((ev.target as HTMLElement)?.tagName === 'INPUT') return;
      if (ev.key === ' ') { ev.preventDefault(); toggle(); }
      if (ev.key === 'ArrowLeft') { ev.preventDefault(); step(-5); }
      if (ev.key === 'ArrowRight') { ev.preventDefault(); step(5); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // When a fresh snapshot arrives after "Live", start near its end.
  $effect(() => {
    if (rv && video && pendingSeekEnd) {
      const v = video;
      const go = () => { v.currentTime = Math.max(0, v.duration - 3); pendingSeekEnd = false; };
      if (v.readyState >= 1) go(); else v.addEventListener('loadedmetadata', go, { once: true });
    }
  });

  const clamp = (x: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, x));
  const fmt = (x: number) => { const v = Math.max(0, Math.round(x)); return `${Math.floor(v / 60)}:${String(v % 60).padStart(2, '0')}`; };
  const pct = (x: number) => (dur ? (x / dur) * 100 : 0);

  function seek(x: number) { if (video) video.currentTime = clamp(x, 0, dur || 0); }
  function step(d: number) { seek(t + d); }
  function toggle() { if (video) (video.paused ? video.play() : video.pause()); }
  function live() { pendingSeekEnd = true; sel = null; prepareReview(); }
  function at(ev: PointerEvent) {
    const r = track!.getBoundingClientRect();
    return clamp(((ev.clientX - r.left) / r.width) * dur, 0, dur);
  }
  function down(ev: PointerEvent, what: 'seek' | 'a' | 'b') {
    ev.stopPropagation();
    drag = what;
    (ev.currentTarget as HTMLElement).setPointerCapture(ev.pointerId);
    move(ev);
  }
  function move(ev: PointerEvent) {
    if (!drag) return;
    const x = at(ev);
    if (drag === 'seek') seek(x);
    else if (sel && drag === 'a') sel.a = Math.min(x, sel.b - 1);
    else if (sel && drag === 'b') sel.b = Math.max(x, sel.a + 1);
  }
  function up() { drag = null; }

  function createClip() { sel = { a: clamp(t - 15, 0, dur), b: clamp(t + 5, 0, dur) }; }
  function aroundMarker(m: { rel: number }) {
    seek(m.rel - 3);
    sel = { a: clamp(m.rel - before, 0, dur), b: clamp(m.rel + after, 0, dur) };
  }
  function jumpMarker(dir: 1 | -1) {
    const list = dir > 0 ? markers.filter(m => m.rel > t + 0.5) : markers.filter(m => m.rel < t - 0.5).reverse();
    if (list[0]) seek(list[0].rel - 3);
  }
  async function saveSel() {
    if (!rv || !sel) return;
    await saveRange(rv.start + sel.a, rv.start + sel.b);
    sel = null;
  }
  const saveAround = (m: Marker) => saveRange(m.t - before, m.t + after);
</script>

<section class="rv">
  {#if app.reviewError}
    <div class="card">
      <p>{app.reviewError}</p>
      <button onclick={onclose}>Back</button>
    </div>
  {:else if !rv}
    <div class="card muted">Opening your recent gameplay… (recording continues)</div>
  {:else}
    <div class="stage">
      <!-- svelte-ignore a11y_media_has_caption -->
      <video bind:this={video} src={convertFileSrc(rv.path)} bind:currentTime={t} bind:duration={dur} bind:paused onclick={toggle}></video>
      {#if app.reviewLoading}<div class="busy">Updating to live…</div>{/if}
    </div>

    <!-- Timeline: click/drag to scrub; markers are clickable; handles appear when making a clip. -->
    <div class="timeline">
      <div class="track" bind:this={track} role="slider" tabindex="0" aria-label="Position" aria-valuemin={0} aria-valuemax={Math.round(dur)} aria-valuenow={Math.round(t)}
        onpointerdown={ev => down(ev, 'seek')} onpointermove={move} onpointerup={up}>
        <div class="fill" style:width="{pct(t)}%"></div>
        {#if sel}<div class="sel" style:left="{pct(sel.a)}%" style:width="{pct(sel.b - sel.a)}%"></div>{/if}
        {#each markers as m (m.id)}
          <button class="mk" class:prot={m.footage === 'protected'} style:left="{pct(m.rel)}%" title="{m.at}{m.label ? ` · ${m.label}` : ''}"
            aria-label="Marker at {m.at}" onpointerdown={ev => ev.stopPropagation()} onclick={() => aroundMarker(m)}></button>
        {/each}
        <div class="head" style:left="{pct(t)}%"></div>
        {#if sel}
          <div class="handle" role="slider" tabindex="0" aria-label="Clip start" aria-valuenow={Math.round(sel.a)} style:left="{pct(sel.a)}%" onpointerdown={ev => down(ev, 'a')} onpointermove={move} onpointerup={up}></div>
          <div class="handle" role="slider" tabindex="0" aria-label="Clip end" aria-valuenow={Math.round(sel.b)} style:left="{pct(sel.b)}%" onpointerdown={ev => down(ev, 'b')} onpointermove={move} onpointerup={up}></div>
        {/if}
      </div>
      <div class="row ctl">
        <button title="Beginning" aria-label="Beginning" onclick={() => seek(0)}>⏮</button>
        <button title="Back 5 s" aria-label="Back 5 seconds" onclick={() => step(-5)}>−5s</button>
        <button class="play" aria-label={paused ? 'Play' : 'Pause'} onclick={toggle}>{paused ? '▶' : '❚❚'}</button>
        <button title="Forward 5 s" aria-label="Forward 5 seconds" onclick={() => step(5)}>+5s</button>
        <button title="Latest footage" onclick={live}>Live ⏭</button>
        <span class="sep"></span>
        <button disabled={!markers.length} onclick={() => jumpMarker(-1)} aria-label="Previous marker">◀ Marker</button>
        <button disabled={!markers.length} onclick={() => jumpMarker(1)} aria-label="Next marker">Marker ▶</button>
        <span class="mono muted">{fmt(t)} / {fmt(dur)}</span>
        <span class="grow"></span>
        {#if sel}
          <span class="muted small">Clip {fmt(sel.a)}–{fmt(sel.b)} ({fmtDuration(sel.b - sel.a)})</span>
          <button onclick={() => (sel = null)}>Cancel</button>
          <button class="primary" onclick={saveSel}>Save clip</button>
        {:else}
          <button class="primary" onclick={createClip}>Create clip</button>
        {/if}
      </div>
    </div>

    {#if markers.length}
      <div class="markers">
        {#each markers as m (m.id)}
          <div class="mrow">
            <button class="link" onclick={() => aroundMarker(m)}>{m.at}</button>
            {#if renaming === m.id}
              <input type="text" value={m.label} placeholder="Name (optional)" aria-label="Marker name"
                onkeydown={ev => { if (ev.key === 'Enter') { engine('rename_marker', { id: m.id, label: (ev.target as HTMLInputElement).value }); renaming = null; } if (ev.key === 'Escape') { ev.stopPropagation(); renaming = null; } }} />
            {:else}
              <span class="ellipsis grow">{m.label || 'Marker'}</span>
            {/if}
            <span class="small {m.footage === 'protected' ? 'ok' : 'muted'}" title={m.note ?? ''}>
              {m.saved_in.length ? 'Saved in a clip' : m.footage === 'protected' ? 'Kept until saved or session ends' : m.footage === 'buffered' ? 'Temporary' : 'Footage gone'}
            </span>
            <button onclick={() => saveAround(m)} title="Save {fmtDuration(before)} before to {fmtDuration(after)} after">Save around</button>
            <button onclick={() => (renaming = m.id)}>Rename</button>
            <button onclick={() => engine('remove_marker', { id: m.id })}>Remove</button>
          </div>
        {/each}
      </div>
    {/if}
    <p class="muted small">Showing footage up to {fmt(dur)} into the buffer. Recording continues in the background; press Live to catch up.</p>
  {/if}
</section>

<style>
  .rv { display: flex; flex-direction: column; gap: 10px; flex: 1; min-height: 0; }
  .stage { position: relative; flex: 1; min-height: 0; background: #000; border-radius: 10px; overflow: hidden; }
  video { width: 100%; height: 100%; object-fit: contain; display: block; }
  .busy { position: absolute; top: 10px; right: 10px; background: #000b; padding: 4px 10px; border-radius: 6px; font-size: 13px; }
  .timeline { display: flex; flex-direction: column; gap: 8px; }
  .track { position: relative; height: 26px; background: #232833; border-radius: 6px; cursor: pointer; touch-action: none; }
  .fill { position: absolute; inset: 0 auto 0 0; background: #2f3644; border-radius: 6px; pointer-events: none; }
  .sel { position: absolute; top: 0; bottom: 0; background: #ef444440; border: 1px solid var(--accent); border-radius: 4px; pointer-events: none; }
  .head { position: absolute; top: -3px; bottom: -3px; width: 2px; background: white; transform: translateX(-1px); pointer-events: none; }
  .mk { position: absolute; top: 3px; width: 10px; height: 20px; padding: 0; transform: translateX(-5px); background: var(--warn); border: none; border-radius: 3px; }
  .mk.prot { box-shadow: 0 0 0 2px #fbbf2455; }
  .handle { position: absolute; top: -4px; bottom: -4px; width: 10px; transform: translateX(-5px); background: var(--accent); border-radius: 3px; cursor: ew-resize; }
  .ctl { gap: 6px; }
  .ctl button { padding: 5px 10px; }
  .play { width: 52px; }
  .sep { width: 10px; }
  .markers { display: flex; flex-direction: column; gap: 4px; max-height: 120px; overflow: auto; }
  .mrow { display: flex; align-items: center; gap: 8px; font-size: 14px; }
  .mrow button { padding: 3px 8px; font-size: 13px; }
  .mrow input { padding: 3px 8px; }
  .ok { color: var(--ok); }
</style>
