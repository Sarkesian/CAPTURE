<script lang="ts">
  // A saved clip, recording or screenshot, with the same controls as the background-recording
  // viewer (no browser-default player): play, seek, volume, speed and fullscreen. Fullscreen
  // takes the whole viewer, so previous / next and the controls stay with the picture.
  // A clip can also be trimmed here: drag the two ends, save, and the file itself gets shorter.
  import { onMount, untrack } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import type { ClipEntry } from '../lib/types';
  import { clock } from '../lib/timeline';
  import { openPath, trimClip, toast } from '../lib/store.svelte';
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
  /** Bumped when the file itself changed (a saved trim), so the player loads it afresh. */
  let rev = $state(0);
  // One volume for every player in the app.
  const storedVolume = Number(globalThis.localStorage?.getItem('capture.volume') ?? 1);
  let volume = $state(Number.isFinite(storedVolume) ? Math.min(1, Math.max(0, storedVolume)) : 1);
  $effect(() => { try { localStorage.setItem('capture.volume', String(volume)); } catch { /* private mode */ } });

  // Another item or a changed file: start clean (the element itself is new, see the key block).
  $effect(() => { item.path; rev; untrack(() => { paused = true; time = 0; length = 0; failed = false; }); });
  $effect(() => { item.path; untrack(() => { trim = null; trimError = ''; }); });
  $effect(() => { if (video) { video.volume = volume; video.muted = muted; video.playbackRate = speed; } });
  // The seek bar moves smoothly while playing (timeupdate alone arrives about four times a second).
  $effect(() => {
    if (paused || !video) return;
    const v = video;
    let frame = requestAnimationFrame(function tick() {
      time = v.currentTime;
      // Trimming: what will be kept plays round and round.
      if (trim && time >= trim.b) { v.currentTime = trim.a; time = trim.a; }
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  });

  function toggle() {
    if (!video || failed || saving) return;
    if (video.paused || video.ended) {
      if (trim && (time < trim.a - .01 || time >= trim.b - .05)) seek(trim.a);
      video.play().catch(() => {});
    } else video.pause();
  }
  function seek(t: number) {
    if (!video || !length) return;
    time = Math.min(Math.max(0, t), length);
    video.currentTime = time;
  }

  // Trim: the part between the two handles is kept; the rest is removed from the saved file.
  const SHORTEST = 1;
  let trim = $state<{ a: number; b: number } | null>(null);
  let saving = $state(false);
  let trimError = $state('');
  let bar = $state<HTMLElement>();
  const changed = $derived(!!trim && (trim.a > .05 || trim.b < length - .05));
  const pct = (t: number) => `${length ? Math.min(100, Math.max(0, t / length * 100)) : 0}%`;
  function startTrim() {
    if (image || failed || saving || length < SHORTEST * 2) return;
    trim = { a: 0, b: length }; trimError = '';
  }
  function cancelTrim() { if (!saving) { trim = null; trimError = ''; } }
  function setEdge(edge: 'a' | 'b', at = time) {
    if (!trim || saving) return;
    at = Math.min(Math.max(0, at), length);
    trim = edge === 'a' ? { a: Math.max(0, Math.min(at, trim.b - SHORTEST)), b: trim.b } : { a: trim.a, b: Math.min(length, Math.max(at, trim.a + SHORTEST)) };
  }
  const timeAt = (e: PointerEvent) => { const r = bar!.getBoundingClientRect(); return (e.clientX - r.left) / Math.max(1, r.width) * length; };
  /** Drag on the trim bar: a handle moves that end (the picture follows it), anywhere else moves the playhead. */
  function drag(e: PointerEvent, edge?: 'a' | 'b') {
    if (!trim || saving || e.button !== 0) return;
    e.preventDefault(); e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    video?.pause();
    const move = (ev: PointerEvent) => {
      if (!trim) return;
      if (!edge) { seek(timeAt(ev)); return; }
      setEdge(edge, timeAt(ev));
      seek(edge === 'a' ? trim.a : Math.min(trim.b, length - .05));
    };
    const up = () => { el.removeEventListener('pointermove', move); el.removeEventListener('pointerup', up); el.removeEventListener('pointercancel', up); };
    el.addEventListener('pointermove', move); el.addEventListener('pointerup', up); el.addEventListener('pointercancel', up);
    move(e);
  }
  function nudge(e: KeyboardEvent, edge: 'a' | 'b') {
    if (!trim || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return;
    e.preventDefault(); e.stopPropagation();
    setEdge(edge, trim[edge] + (e.key === 'ArrowRight' ? 1 : -1) * (e.shiftKey ? 1 : .1));
    seek(edge === 'a' ? trim.a : Math.min(trim.b, length - .05));
  }
  async function saveTrim() {
    if (!trim || !changed || saving) return;
    const { a, b } = trim, path = item.path;
    saving = true; trimError = '';
    video?.pause();
    const error = await trimClip(path, a, b, () => { if (video) { video.removeAttribute('src'); video.load(); } });
    saving = false;
    if (item.path !== path) return;
    rev++;
    if (error) { trimError = error; return; }
    trim = null;
    toast('Clip trimmed. The original is in the Recycle Bin.', 'ok', 5000);
  }

  let full = $state(false);
  let idle = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let overDock = false;
  // In fullscreen the controls leave after a moment of playing and return with the mouse or a key.
  function wake() {
    idle = false; clearTimeout(idleTimer);
    if (full) idleTimer = setTimeout(() => { if (full && !overDock && !trim && (image || !paused)) idle = true; }, 2200);
  }
  $effect(() => { paused; trim; untrack(wake); });
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
      else if (k === 'pageup') { if (!trim) onstep(prev); }
      else if (k === 'pagedown') { if (!trim) onstep(next); }
      else if (k === 'f') fullscreen();
      else if (k === 'm' && !image) muted = !muted;
      else if (k === 't' && !image) { if (!trim) startTrim(); }
      else if (k === 'i' && trim) setEdge('a');
      else if (k === 'o' && trim) setEdge('b');
      else if (k === 'enter' && trim && !(e.target as HTMLElement)?.closest('button,a')) saveTrim();
      else return;
      e.preventDefault();
    };
    // Esc leaves the trim first; only the next one leaves the viewer.
    const escape = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || !trim || document.fullscreenElement) return;
      cancelTrim(); e.preventDefault(); e.stopPropagation();
    };
    document.addEventListener('fullscreenchange', change);
    window.addEventListener('keydown', key);
    window.addEventListener('keydown', escape, true);
    return () => { document.removeEventListener('fullscreenchange', change); window.removeEventListener('keydown', key); window.removeEventListener('keydown', escape, true); clearTimeout(idleTimer); };
  });
</script>

<section class="saved" class:full class:idle={full && idle} bind:this={root} aria-label={name(item)} onpointermove={wake} onpointerdown={wake}>
  <div class="stage">
    {#key item.path + rev}
      {#if image}<img src={convertFileSrc(item.path)} alt={name(item)} ondblclick={fullscreen}/>
      {:else}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={video} src={convertFileSrc(item.path) + (rev ? `?v=${rev}` : '')} preload="metadata" autoplay disablepictureinpicture aria-label={name(item)}
          onloadedmetadata={e => { length = e.currentTarget.duration || 0; e.currentTarget.playbackRate = speed; }}
          onplay={() => paused = false} onpause={() => paused = true}
          onended={e => { if (trim) { seek(trim.a); e.currentTarget.play().catch(() => {}); } else paused = true; }}
          ontimeupdate={e => time = e.currentTarget.currentTime} onerror={e => { if (e.currentTarget.getAttribute('src')) { failed = true; paused = true; } }}
          onclick={toggle} ondblclick={fullscreen}></video>
      {/if}
    {/key}
    {#if failed}<div class="stage-message" role="alert"><span>This file can’t be played here.</span><button class="btn sm" onclick={() => openPath(item.path)}>Open externally</button></div>{/if}
    {#if saving}<span class="busy" role="status"><i></i>Saving the trim…</span>{/if}
    {#if prev && !trim}<button class="nav" aria-label="Previous" title={name(prev)} onclick={() => onstep(prev)}><span><Icon name="back" size={20}/></span></button>{/if}
    {#if next && !trim}<button class="nav next" aria-label="Next" title={name(next)} onclick={() => onstep(next)}><span><Icon name="back" size={20}/></span></button>{/if}
  </div>
  <div class="dock" onpointerenter={() => overDock = true} onpointerleave={() => { overDock = false; wake(); }} role="presentation">
    {#if trim}
      <div class="trimbar" role="toolbar" aria-label="Trim">
        <output class="trim-length" aria-label="Length after trimming"><Icon name="scissors" size={15}/>{clock(trim.b - trim.a)}<span class="muted">&nbsp;of {clock(length)}</span></output>
        <span class="sep"></span>
        <button class="btn ghost sm" title="Start the clip at the playhead (I)" disabled={saving || time >= trim.b - SHORTEST} onclick={() => setEdge('a')}>Start here</button>
        <button class="btn ghost sm" title="End the clip at the playhead (O)" disabled={saving || time <= trim.a + SHORTEST} onclick={() => setEdge('b')}>End here</button>
      </div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="track" class:busy-track={saving} bind:this={bar} onpointerdown={e => drag(e)}>
        <div class="keep" style:left={pct(trim.a)} style:width="calc({pct(trim.b)} - {pct(trim.a)})"></div>
        <div class="head" style:left={pct(time)}></div>
        <button class="handle" style:left={pct(trim.a)} aria-label="Clip start, {clock(trim.a)}" title="Drag to where the clip should start" onpointerdown={e => drag(e, 'a')} onkeydown={e => nudge(e, 'a')}></button>
        <button class="handle end" style:left={pct(trim.b)} aria-label="Clip end, {clock(trim.b)}" title="Drag to where the clip should end" onpointerdown={e => drag(e, 'b')} onkeydown={e => nudge(e, 'b')}></button>
      </div>
      <p class="hint" class:error={!!trimError} role={trimError ? 'alert' : undefined}>{trimError || 'Drag the ends to keep only the part you want. The start lands on a clean frame, at most a second before your mark.'}</p>
    {:else if !image}
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
        <button class="btn ghost icon" aria-label="Previous" title="Previous (Page Up)" disabled={!prev || !!trim} onclick={() => onstep(prev)}><Icon name="skip-back" size={15}/></button>
        {#if !image}
          <button class="btn ghost icon" aria-label="Back 10 seconds" title="Back 10 seconds (← skips 5)" disabled={!length} onclick={() => seek(time - 10)}><Icon name="rewind" size={16}/></button>
          <button class="btn play" aria-label={paused ? 'Play' : 'Pause'} title={paused ? 'Play (Space)' : 'Pause (Space)'} disabled={failed || saving} onclick={toggle}><Icon name={paused ? 'play' : 'pause'} size={18}/></button>
          <button class="btn ghost icon" aria-label="Forward 10 seconds" title="Forward 10 seconds (→ skips 5)" disabled={!length} onclick={() => seek(time + 10)}><Icon name="forward" size={16}/></button>
        {/if}
        <button class="btn ghost icon" aria-label="Next" title="Next (Page Down)" disabled={!next || !!trim} onclick={() => onstep(next)}><Icon name="skip-fwd" size={15}/></button>
        {#if !image}<span class="time"><b>{clock(time)}</b><span class="muted">&nbsp;/ {clock(length)}</span></span>{/if}
      </div>
      <div class="end">
        <!-- While trimming, the Trim button becomes Save trim (a "Trim" that cancelled was easy to press by mistake); the small ✕ beside it leaves trimming. -->
        {#if trim}<button class="btn primary trim-toggle" title="Keep only the highlighted part. The untrimmed clip goes to the Recycle Bin. (Enter)" disabled={!changed || saving} onclick={saveTrim}><Icon name="save" size={16}/>{saving ? 'Saving…' : 'Save trim'}</button><button class="btn trim-x" aria-label="Cancel trim" title="Leave trimming without changing the clip (Esc)" disabled={saving} onclick={cancelTrim}><Icon name="x" size={13}/></button>
        {:else if !image}<button class="btn ghost trim-toggle" title="Trim this clip: cut off the start, the end, or both (T)" disabled={failed || saving || length < SHORTEST * 2} onclick={startTrim}><Icon name="scissors" size={16}/>Trim</button>{/if}
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
  .stage-message,.busy{position:absolute;display:flex;align-items:center;background:#1c2027ed;color:#e8ebf0;border-radius:var(--r-md);padding:8px 12px;font-size:12px;white-space:nowrap}
  .stage-message{left:50%;bottom:12px;transform:translateX(-50%);gap:10px}
  .busy{top:12px;right:12px;gap:8px}
  .busy i{width:12px;height:12px;border-radius:50%;border:2px solid #ffffff40;border-top-color:#fff;animation:spin .7s linear infinite}
  @keyframes spin{to{transform:rotate(1turn)}}
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
  .end{justify-content:flex-end;gap:4px}
  .transport .btn{height:32px}
  .transport .icon{width:32px;padding:0}
  .transport .play{width:42px;height:36px;padding:0;margin:0 4px;border-radius:var(--r-pill);background:var(--surface-3);border-color:var(--line-strong);color:var(--text);transition:transform var(--t-med) var(--ease),background var(--t-fast) var(--ease)}
  .transport .play:hover:not(:disabled){background:var(--line-strong);transform:scale(1.06)}
  .transport .play:active:not(:disabled){transform:scale(.94)}
  .speed{width:44px;padding:0;font-size:12px;font-weight:600;font-variant-numeric:tabular-nums}
  .trim-toggle{gap:6px;padding:0 10px;font-size:13px}
  /* Its own small outlined button, low on the right of Save trim. */
  .transport .trim-x{align-self:flex-end;width:22px;height:22px;padding:0;margin-right:6px;border-radius:var(--r-sm);background:transparent;border-color:var(--line-strong);color:var(--text-2)}
  .transport .trim-x:hover:not(:disabled){background:var(--surface-2);color:var(--text)}
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

  /* Trim: the tools sit right above the bar they act on (as the clip tools do in a recording). */
  .trimbar{display:flex;align-items:center;justify-content:center;gap:3px;width:fit-content;max-width:calc(100% - 28px);margin:2px auto 8px;padding:5px;background:var(--surface-2);border:1px solid var(--line-strong);border-radius:var(--r-lg);white-space:nowrap}
  .trimbar .btn{gap:6px}
  .trim-length{display:flex;align-items:center;gap:6px;padding:0 8px 0 7px;color:var(--accent);font-size:13px;font-weight:650;font-variant-numeric:tabular-nums}
  .trim-length .muted{font-weight:500}
  .sep{width:1px;height:18px;margin:0 3px;background:var(--line-strong)}
  /* What is outside the handles is dimmed: that is what goes. */
  .track{position:relative;height:34px;margin:0 20px;border-radius:var(--r-sm);background:var(--surface-3);cursor:pointer;touch-action:none}
  .track.busy-track{opacity:.6;pointer-events:none}
  .keep{position:absolute;top:0;bottom:0;background:var(--accent-soft);border-block:2px solid var(--accent);pointer-events:none}
  .head{position:absolute;top:-3px;bottom:-3px;width:2px;margin-left:-1px;background:var(--text);border-radius:1px;pointer-events:none}
  .handle{position:absolute;top:0;bottom:0;width:14px;margin-left:-14px;padding:0;border:0;border-radius:var(--r-sm) 0 0 var(--r-sm);background:var(--accent);cursor:ew-resize;touch-action:none}
  .handle.end{margin-left:0;border-radius:0 var(--r-sm) var(--r-sm) 0}
  .handle::after{content:'';position:absolute;left:6px;top:10px;bottom:10px;width:2px;border-radius:1px;background:var(--text-on-accent)}
  .handle:hover{background:var(--accent-hover)}
  .handle:focus-visible{outline:2px solid var(--text);outline-offset:1px}
  .hint{margin:7px 20px 2px;font-size:12px;color:var(--text-2);text-align:center}
  .hint.error{color:var(--warn)}
  @media(max-width:620px){.tools .slider{display:none}.trimbar{flex-wrap:wrap}}
  @media(prefers-reduced-motion:reduce){.full .dock,.nav span{transition:none}}
</style>
