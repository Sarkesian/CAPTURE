<script lang="ts">
  // A saved clip, recording or screenshot, with the same controls as the background-recording
  // viewer (no browser-default player): play, seek, volume, speed and fullscreen. Fullscreen
  // takes the whole viewer, so previous / next and the controls stay with the picture.
  import { onMount, untrack } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import type { ClipEntry } from '../lib/types';
  import { clock } from '../lib/timeline';
  import { openPath } from '../lib/store.svelte';
  import Icon from './Icon.svelte';

  let { item, prev, next, onstep }: { item: ClipEntry; prev: ClipEntry | null; next: ClipEntry | null; onstep: (to: ClipEntry | null) => void } = $props();
  const name = (c: ClipEntry) => c.name.replace(/\.(mp4|png)$/i, '');
  const image = $derived(item.kind === 'screenshot');
  const speeds = [0.5, 1, 1.5, 2];
  let root = $state<HTMLElement>();
  let video = $state<HTMLVideoElement>();
  let paused = $state(true);
  let time = $state(0);
  let length = $state(0);
  let failed = $state(false);
  let speed = $state(1);
  let muted = $state(false);
  // One volume for every player in the app.
  const storedVolume = Number(globalThis.localStorage?.getItem('capture.volume') ?? 1);
  let volume = $state(Number.isFinite(storedVolume) ? Math.min(1, Math.max(0, storedVolume)) : 1);
  $effect(() => { try { localStorage.setItem('capture.volume', String(volume)); } catch { /* private mode */ } });

  // Another item: start clean (the element itself is new, see the key block).
  $effect(() => { item.path; untrack(() => { paused = true; time = 0; length = 0; failed = false; }); });
  $effect(() => { if (video) { video.volume = volume; video.muted = muted; video.playbackRate = speed; } });
  // The seek bar moves smoothly while playing (timeupdate alone arrives about four times a second).
  $effect(() => {
    if (paused || !video) return;
    const v = video;
    let frame = requestAnimationFrame(function tick() { time = v.currentTime; frame = requestAnimationFrame(tick); });
    return () => cancelAnimationFrame(frame);
  });

  function toggle() {
    if (!video || failed) return;
    if (video.paused || video.ended) video.play().catch(() => {});
    else video.pause();
  }
  function seek(t: number) {
    if (!video || !length) return;
    time = Math.min(Math.max(0, t), length);
    video.currentTime = time;
  }

  let full = $state(false);
  let idle = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let overDock = false;
  // In fullscreen the controls leave after a moment of playing and return with the mouse or a key.
  function wake() {
    idle = false; clearTimeout(idleTimer);
    if (full) idleTimer = setTimeout(() => { if (full && !overDock && (image || !paused)) idle = true; }, 2200);
  }
  $effect(() => { paused; untrack(wake); });
  async function fullscreen() {
    try { if (document.fullscreenElement) await document.exitFullscreen(); else await root?.requestFullscreen(); } catch { /* stays windowed */ }
  }

  onMount(() => {
    const change = () => { full = document.fullscreenElement === root; wake(); };
    const key = (e: KeyboardEvent) => {
      wake();
      if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || (e.target as HTMLElement)?.closest('input,select,textarea')) return;
      const k = e.key.toLowerCase();
      // Space plays and pauses wherever the focus is (it starts on the Back button), except on the viewer's own buttons.
      if (k === ' ') { const b = (e.target as HTMLElement)?.closest('button'); if (image || (b && root?.contains(b))) return; toggle(); }
      else if (k === 'arrowleft' || k === 'arrowright') { const d = k === 'arrowright' ? 1 : -1; if (image) onstep(d > 0 ? next : prev); else seek(time + d * 5); }
      else if (k === 'pageup') onstep(prev);
      else if (k === 'pagedown') onstep(next);
      else if (k === 'f') fullscreen();
      else if (k === 'm' && !image) muted = !muted;
      else return;
      e.preventDefault();
    };
    document.addEventListener('fullscreenchange', change);
    window.addEventListener('keydown', key);
    return () => { document.removeEventListener('fullscreenchange', change); window.removeEventListener('keydown', key); clearTimeout(idleTimer); };
  });
</script>

<section class="saved" class:full class:idle={full && idle} bind:this={root} aria-label={name(item)} onpointermove={wake} onpointerdown={wake}>
  <div class="stage">
    {#key item.path}
      {#if image}<img src={convertFileSrc(item.path)} alt={name(item)} ondblclick={fullscreen}/>
      {:else}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={video} src={convertFileSrc(item.path)} preload="metadata" autoplay disablepictureinpicture aria-label={name(item)}
          onloadedmetadata={e => { length = e.currentTarget.duration || 0; e.currentTarget.playbackRate = speed; }}
          onplay={() => paused = false} onpause={() => paused = true} onended={() => paused = true}
          ontimeupdate={e => time = e.currentTarget.currentTime} onerror={() => { failed = true; paused = true; }}
          onclick={toggle} ondblclick={fullscreen}></video>
      {/if}
    {/key}
    {#if failed}<div class="stage-message" role="alert"><span>This file can’t be played here.</span><button class="btn sm" onclick={() => openPath(item.path)}>Open externally</button></div>{/if}
    {#if prev}<button class="nav" aria-label="Previous" title={name(prev)} onclick={() => onstep(prev)}><span><Icon name="back" size={20}/></span></button>{/if}
    {#if next}<button class="nav next" aria-label="Next" title={name(next)} onclick={() => onstep(next)}><span><Icon name="back" size={20}/></span></button>{/if}
  </div>
  <div class="dock" onpointerenter={() => overDock = true} onpointerleave={() => { overDock = false; wake(); }} role="presentation">
    {#if !image}
      <input class="slider seek" type="range" min="0" max={length || 1} step="0.01" aria-label="Seek" aria-valuetext={clock(time)} disabled={!length}
        style:--fill="{length ? time / length * 100 : 0}%" value={time} oninput={e => seek(Number(e.currentTarget.value))} />
    {/if}
    <div class="transport">
      <div class="tools">
        {#if !image}
          <button class="btn ghost icon" aria-label={muted ? 'Unmute' : 'Mute'} title={muted ? 'Unmute (M)' : 'Mute (M)'} aria-pressed={muted} onclick={() => muted = !muted}><Icon name={muted || volume === 0 ? 'speaker-off' : 'speaker'} size={17}/></button>
          <input class="slider" type="range" min="0" max="1" step="0.05" aria-label="Volume" title="Volume" style:--fill="{(muted ? 0 : volume) * 100}%" value={muted ? 0 : volume} oninput={e => { volume = Number(e.currentTarget.value); muted = false; }} />
          <button class="btn ghost speed" class:active={speed !== 1} aria-label="Playback speed {speed}×" title="Playback speed" onclick={() => speed = speeds[(speeds.indexOf(speed) + 1) % speeds.length]}>{speed}×</button>
        {/if}
      </div>
      <div class="playback">
        <button class="btn ghost icon" aria-label="Previous" title="Previous (Page Up)" disabled={!prev} onclick={() => onstep(prev)}><Icon name="skip-back" size={15}/></button>
        {#if !image}
          <button class="btn ghost icon" aria-label="Back 10 seconds" title="Back 10 seconds (← skips 5)" disabled={!length} onclick={() => seek(time - 10)}><Icon name="rewind" size={16}/></button>
          <button class="btn play" aria-label={paused ? 'Play' : 'Pause'} title={paused ? 'Play (Space)' : 'Pause (Space)'} disabled={failed} onclick={toggle}><Icon name={paused ? 'play' : 'pause'} size={18}/></button>
          <button class="btn ghost icon" aria-label="Forward 10 seconds" title="Forward 10 seconds (→ skips 5)" disabled={!length} onclick={() => seek(time + 10)}><Icon name="forward" size={16}/></button>
        {/if}
        <button class="btn ghost icon" aria-label="Next" title="Next (Page Down)" disabled={!next} onclick={() => onstep(next)}><Icon name="skip-fwd" size={15}/></button>
        {#if !image}<span class="time"><b>{clock(time)}</b><span class="muted">&nbsp;/ {clock(length)}</span></span>{/if}
      </div>
      <div class="end">
        <button class="btn ghost icon" aria-label={full ? 'Leave fullscreen' : 'Fullscreen'} title={full ? 'Leave fullscreen (F or Esc)' : 'Fullscreen (F)'} onclick={fullscreen}><Icon name="fullscreen" size={17}/></button>
      </div>
    </div>
  </div>
</section>

<style>
  /* Same look as the background-recording viewer (HistoryPanel). */
  .saved{display:flex;flex-direction:column;flex:1;min-height:0;min-width:0;position:relative}
  .saved:fullscreen{background:#000}
  .stage{flex:1;min-height:0;position:relative;overflow:hidden;background:#080a0e}
  .stage video,.stage img{position:absolute;inset:0;width:100%;height:100%;object-fit:contain;display:block}
  .stage video{cursor:pointer}
  .stage-message{position:absolute;left:50%;bottom:12px;transform:translateX(-50%);display:flex;align-items:center;gap:10px;background:#1c2027ed;color:#e8ebf0;border-radius:var(--r-md);padding:8px 12px;font-size:12px;white-space:nowrap}
  /* Side zones in the middle of the picture only. */
  .nav{position:absolute;left:0;top:25%;bottom:25%;width:76px;display:grid;place-items:center;border:0;padding:0;background:none;cursor:pointer}
  .nav.next{left:auto;right:0}
  .nav span{display:grid;place-items:center;width:36px;height:36px;border-radius:50%;background:#10151dcc;color:#fff;opacity:0;transition:opacity 110ms ease}
  .nav.next span{transform:rotate(180deg)}
  .nav:is(:hover,:focus-visible) span{opacity:1}

  .dock{flex:none;background:var(--surface);border-top:1px solid var(--line);padding-top:6px}
  .full .stage{position:absolute;inset:0}
  .full .dock{position:absolute;left:0;right:0;bottom:0;z-index:5;border-top:0;padding-top:26px;background:linear-gradient(transparent,color-mix(in srgb,var(--chrome) 88%,transparent) 26px,var(--chrome));transition:opacity 220ms var(--ease),transform 220ms var(--ease)}
  .full.idle .dock{opacity:0;transform:translateY(10px);pointer-events:none}
  .full.idle,.full.idle .stage video{cursor:none}
  .transport{display:grid;grid-template-columns:minmax(0,1fr) auto minmax(0,1fr);gap:8px;align-items:center;padding:2px 14px 10px}
  .tools,.playback,.end{display:flex;gap:2px;align-items:center;min-width:0}
  .end{justify-content:flex-end}
  .transport .btn{height:32px}
  .transport .icon{width:32px;padding:0}
  .transport .play{width:42px;height:36px;padding:0;margin:0 4px;border-radius:var(--r-pill);background:var(--surface-3);border-color:var(--line-strong);color:var(--text);transition:transform var(--t-med) var(--ease),background var(--t-fast) var(--ease)}
  .transport .play:hover:not(:disabled){background:var(--line-strong);transform:scale(1.06)}
  .transport .play:active:not(:disabled){transform:scale(.94)}
  .speed{width:44px;padding:0;font-size:12px;font-weight:600;font-variant-numeric:tabular-nums}
  .active{color:var(--accent)!important;background:var(--accent-soft)!important}
  .time{font-size:12.5px;font-variant-numeric:tabular-nums;white-space:nowrap;padding-inline-start:8px}
  .time b{font-weight:600}

  .slider{appearance:none;width:78px;height:20px;margin:0 4px;background:transparent;cursor:pointer}
  .slider::-webkit-slider-runnable-track{height:4px;border-radius:2px;background:linear-gradient(to right,var(--text-2) var(--fill,0%),var(--surface-3) var(--fill,0%))}
  .slider::-webkit-slider-thumb{appearance:none;width:12px;height:12px;margin-top:-4px;border-radius:50%;background:var(--text);box-shadow:0 1px 3px #0007;transition:transform var(--t-med) var(--ease)}
  .slider:hover::-webkit-slider-thumb{transform:scale(1.2)}
  .slider:hover::-webkit-slider-runnable-track{background:linear-gradient(to right,var(--accent) var(--fill,0%),var(--surface-3) var(--fill,0%))}
  .slider:focus-visible{outline:2px solid var(--accent);outline-offset:2px;border-radius:4px}
  .slider:disabled{opacity:.45;cursor:default}
  .seek{display:block;width:calc(100% - 28px);margin:0 14px}
  @media(max-width:620px){.tools .slider{display:none}}
  @media(prefers-reduced-motion:reduce){.full .dock,.nav span{transition:none}}
</style>
