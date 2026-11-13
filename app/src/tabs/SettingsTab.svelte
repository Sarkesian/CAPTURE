<script lang="ts">
  // Settings grouped by task, with a section index. Every control applies immediately.
  import { onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import Toggle from '../components/Toggle.svelte';
  import Segmented from '../components/Segmented.svelte';
  import HotkeyField from '../components/HotkeyField.svelte';
  import AudioSources from '../components/AudioSources.svelte';
  import Icon from '../components/Icon.svelte';
  import { app, updateSettings, clearTempCache, openCaptureFolder, openAppFolder, fmtBytes, fmtDuration, engine, toast } from '../lib/store.svelte';
  import type { GameRule, Quality, StagedUpdate } from '../lib/types';

  const s = $derived(app.settings!);
  const e = $derived(app.engine);
  const bitrate = $derived(e && e.history_buffered_secs>8 ? e.buffer_bytes/e.history_buffered_secs : null);
  const estimate = $derived(bitrate != null ? bitrate*s.history_secs : e?.encoder && s.quality!=='archival' ? (e.encoder.width*e.encoder.height*e.encoder.fps*({balanced:.07,high:.12,maximum:.20}[s.quality]??.12)/8+48000)*s.history_secs*1.05 : null);

  const sections = [
    { id: 'recording', label: 'Recording' },
    { id: 'audio', label: 'Audio' },
    { id: 'hotkeys', label: 'Hotkeys' },
    { id: 'markers', label: 'Markers' },
    { id: 'overlay', label: 'Overlay & notices' },
    { id: 'games', label: 'Games' },
    { id: 'storage', label: 'Storage' },
    { id: 'startup', label: 'Startup' },
    { id: 'advanced', label: 'Advanced' },
    { id: 'about', label: 'About & updates' },
  ];
  let active = $state('recording');
  let root = $state<HTMLElement | null>(null);
  onMount(() => {
    engine('history');
    const scroller = root?.closest('main');
    if (!root || !scroller) return;
    const io = new IntersectionObserver(entries => {
      const vis = entries.filter(x => x.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
      if (vis[0]) active = vis[0].target.id;
    }, { root: scroller, rootMargin: '0px 0px -70% 0px' });
    root.querySelectorAll('section[id]').forEach(el => io.observe(el));
    return () => io.disconnect();
  });
  function jump(id: string) {
    active = id;
    document.getElementById(id)?.scrollIntoView({ block: 'start', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
  }

  const qualities = $derived<{ value: Quality; label: string; hint: string }[]>([
    { value: 'balanced', label: 'Balanced', hint: 'Smaller files; great for sharing.' },
    { value: 'high', label: 'High', hint: 'Sharp detail in fast motion. Recommended.' },
    { value: 'maximum', label: 'Maximum', hint: 'High-quality lossy H.264; large files.' },
    ...(s.quality === 'archival' ? [{ value: 'archival' as Quality, label: 'Archival (separate build)', hint: 'Saved preference preserved. Select Balanced, High or Maximum for this history-focused build.' }] : []),
  ]);
  const lengths = [30, 60, 120, 180, 300, 600, 900, 1200];

  // Games that can have their own settings: your games list, existing rules, and the current game.
  const ruleGames = $derived.by(() => {
    const set = new Map<string, string>();
    for (const g of s.games) set.set(g.toLowerCase(), g);
    for (const r of s.per_game) set.set(r.exe.toLowerCase(), r.exe);
    if (e?.target_exe) set.set(e.target_exe.toLowerCase(), e.target_exe);
    return [...set.values()].sort((a, b) => a.localeCompare(b));
  });
  const ruleFor = (exe: string): GameRule => s.per_game.find(r => r.exe.toLowerCase() === exe.toLowerCase()) ?? { exe, replay_secs: null, quality: null, game_audio: null };
  function setRule(exe: string, patch: Partial<GameRule>) {
    const next = { ...ruleFor(exe), ...patch };
    updateSettings({ per_game: [...s.per_game.filter(r => r.exe.toLowerCase() !== exe.toLowerCase()), next] });
  }
  async function startup(v: boolean) {
    try { await invoke('set_start_with_windows', { enabled: v }); } catch (err) { toast(String(err), 'error'); }
  }
  async function changeFolder() {
    const dir = await open({ directory: true, title: 'Choose where CAPTURE saves clips', defaultPath: s.capture_root });
    if (typeof dir === 'string') updateSettings({ capture_root: dir });
  }
  async function changeBuffer() {
    const dir = await open({ directory: true, title: 'Folder for the background recording', defaultPath: s.buffer_root ?? s.capture_root });
    if (typeof dir === 'string') updateSettings({ buffer_root: dir });
  }
  // Updates: choose a downloaded release zip; CAPTURE checks every file before offering to install.
  let staged = $state<StagedUpdate | null>(null);
  let updBusy = $state<'' | 'checking' | 'installing'>('');
  const recordingNow = $derived(e?.session_secs != null);
  async function chooseUpdate() {
    const file = await open({ title: 'Choose a CAPTURE release (.zip)', filters: [{ name: 'CAPTURE release', extensions: ['zip'] }] });
    if (typeof file !== 'string') return;
    staged = null; updBusy = 'checking';
    try { staged = await invoke<StagedUpdate>('stage_update', { path: file }); }
    catch (err) { toast(String(err), 'error', 10000); }
    finally { updBusy = ''; }
  }
  async function installUpdate() {
    updBusy = 'installing';
    try { await invoke('apply_update'); }
    catch (err) { toast(String(err), 'error', 10000); updBusy = ''; }
  }
  const num = (ev: Event) => Number((ev.target as HTMLInputElement).value);
  const join = (dir: string, leaf: string) => `${dir.replace(/[\\/]+$/, '')}\\${leaf}`;
  type HkKey = 'marker' | 'overlay' | 'session' | 'screenshot' | 'mic_mute';
  const hotkeys: [string, string, string, HkKey][] = [
    ['add_marker', 'Add marker', 'Bookmarks the moment without saving a clip.', 'marker'],
    ['toggle_overlay', 'Open overlay', 'Shows CAPTURE on top of your game.', 'overlay'],
    ['toggle_session', 'Start or stop a recording', 'Records a full session of any length.', 'session'],
    ['screenshot', 'Screenshot', 'Saves the game picture as a PNG.', 'screenshot'],
    ['toggle_mic_mute', 'Mute or unmute microphone', 'Works while you play.', 'mic_mute'],
  ];
</script>

<div class="settings" bind:this={root}>
  <aside>
    <h1>Settings</h1>
    <nav aria-label="Settings sections">
      {#each sections as sec (sec.id)}
        <button class:active={active === sec.id} aria-current={active === sec.id ? 'true' : undefined} onclick={() => jump(sec.id)}>{sec.label}</button>
      {/each}
    </nav>
  </aside>

  <div class="body">
    <section id="recording">
      <h2>Recording</h2>
      <div class="field">
        <div class="lbl"><div>Quality</div><div class="hint">{qualities.find(q => q.value === s.quality)?.hint}</div></div>
        <Segmented label="Quality" value={s.quality} options={qualities} onchange={v => updateSettings({ quality: v })} />
      </div>
      <div class="field">
        <div class="lbl"><div>Frame rate</div><div class="hint">Recordings are always a constant frame rate.</div></div>
        <Segmented label="Frame rate" value={s.fps} options={[{ value: 30, label: '30 fps' }, { value: 60, label: '60 fps' }, { value: 120, label: '120 fps' }]} onchange={v => updateSettings({ fps: v })} />
      </div>
      <div class="field">
        <div class="lbl"><div>Instant Replay length</div><div class="hint">How much recent gameplay the hotkey saves.{s.background_history ? ' Background history is kept separately.' : ' This is all that is recorded.'}</div></div>
        <select class="select" aria-label="Replay length" value={String(s.replay_secs)} onchange={ev => updateSettings({ replay_secs: Number((ev.target as HTMLSelectElement).value) })}>
          {#each lengths as d (d)}<option value={String(d)}>{fmtDuration(d)}</option>{/each}
          {#if !lengths.includes(s.replay_secs)}<option value={String(s.replay_secs)}>{fmtDuration(s.replay_secs)}</option>{/if}
        </select>
      </div>
      <div class="field">
        <div class="lbl"><div>Background history</div><div class="hint">{s.background_history ? 'Keeps hours of gameplay across games as temporary background recordings you can clip from later.' : `Off: only the last ${fmtDuration(s.replay_secs)} of the game you are playing is kept, and it is removed when the game closes.`} Turning this off deletes the background recordings kept so far; saved clips are not touched.</div></div>
        <div class="row">
          {#if s.background_history}
            <select class="select" aria-label="Background history duration" value={s.history_secs} onchange={ev=>updateSettings({history_secs:Number(ev.currentTarget.value)})}>
              {#each [3600,7200,14400,21600] as d}<option value={d}>{d/3600} hours</option>{/each}
            </select>
          {/if}
          <Toggle checked={s.background_history} label="Background history" onchange={v => updateSettings({ background_history: v })} />
        </div>
      </div>
      <div class="field">
        <div class="lbl"><div>Show the mouse cursor</div></div>
        <Toggle checked={s.capture_cursor} label="Show the mouse cursor" onchange={v => updateSettings({ capture_cursor: v })} />
      </div>
      <p class="foot">{#if !s.background_history}Only the replay window is on disk while you play, so storage use stays small.{:else if estimate != null}Estimated {fmtBytes(estimate)} for {s.history_secs/3600} hours ({bitrate!=null?'measured current recording rate':'current resolution/FPS ceiling estimate'}). Budget: {s.buffer_cap_gb} GB. Content changes affect actual usage.{:else}Storage depends on source resolution, FPS and content. Archival storage cannot be predicted from a quality label.{/if}</p>
      <p class="foot">Original H.264 packets are kept for history and saved clips. Capture is 8-bit SDR; RGB conversion prevents an end-to-end zero-loss claim. The recording format remains H.264 SDR for broad compatibility.</p>
      {#if e}
        <p class="foot">Source arrivals {e.frames_captured} · video packets {e.frames_encoded} · missed recording ticks {e.frames_dropped} · ticks without a picture {e.frames_without_source} · repeated source samples {e.frames_repeated} · source arrivals not selected {e.source_frames_unselected}. Game frames never delivered by Windows cannot be counted here.</p>
      {/if}
    </section>

    <section id="audio">
      <h2>Audio</h2>
      <AudioSources />
      <div class="field">
        <div class="lbl"><div>Create separate audio files for editors</div><div class="hint">After each save, also write one WAV per audio track next to the clip. Every track is already inside the MP4; you can export these files for any clip later from the Library.</div></div>
        <Toggle checked={s.editor_audio_files} label="Create separate audio files for editors" onchange={v => updateSettings({ editor_audio_files: v })} />
      </div>
    </section>

    <section id="hotkeys">
      <h2>Hotkeys</h2>
      <div class="field">
        <div class="lbl"><div>Save replay</div><div class="hint">Works while you play, even when CAPTURE is in the background.</div></div>
        <HotkeyField value={s.save_hotkey} onchange={v => updateSettings({ save_hotkey: v })} />
      </div>
      {#if e?.hotkey && !e.hotkey.registered}<p class="msg warn"><Icon name="alert" size={14} />{e.hotkey.message}</p>{/if}
      {#each hotkeys as [action, title, hint, key] (action)}
        {@const hs = e?.hotkeys.find(h => h.action === action)}
        <div class="field">
          <div class="lbl"><div>{title}</div><div class="hint">{hint}</div></div>
          <HotkeyField value={s.hotkeys[key]} onchange={v => updateSettings({ hotkeys: { ...s.hotkeys, [key]: v } })} />
        </div>
        {#if hs && hs.combo && !hs.registered && hs.message}<p class="msg warn"><Icon name="alert" size={14} />{hs.message}</p>{/if}
      {/each}
    </section>

    <section id="markers">
      <h2>Markers</h2>
      <div class="field">
        <div class="lbl"><div>Keep footage around markers</div><div class="hint">Stops it being overwritten until you save or the session ends. Uses at most half the background recording limit.</div></div>
        <Toggle checked={s.markers.protect} label="Protect footage around markers" onchange={v => updateSettings({ markers: { ...s.markers, protect: v } })} />
      </div>
      {#if s.markers.protect}
        <div class="field sub">
          <div class="lbl"><div>Seconds before and after</div></div>
          <div class="row">
            <input class="input num" type="number" min="0" max="600" aria-label="Seconds before marker" value={s.markers.protect_before_secs} onchange={ev => updateSettings({ markers: { ...s.markers, protect_before_secs: num(ev) } })} />
            <span class="faint">/</span>
            <input class="input num" type="number" min="0" max="600" aria-label="Seconds after marker" value={s.markers.protect_after_secs} onchange={ev => updateSettings({ markers: { ...s.markers, protect_after_secs: num(ev) } })} />
          </div>
        </div>
      {/if}
      <div class="field">
        <div class="lbl"><div>Play a sound when a hotkey works</div><div class="hint">Instant confirmation when you save a clip, add a marker, take a screenshot or start/stop a session. Kept out of your recordings. A microphone near your speakers can still hear it.</div></div>
        <Toggle checked={s.markers.sound} label="Hotkey sound" onchange={v => updateSettings({ markers: { ...s.markers, sound: v } })} />
      </div>
      {#if s.markers.sound}
        <div class="field sub">
          <label class="lbl" for="mdev">Play it on</label>
          <select class="select" id="mdev" value={s.markers.sound_device ?? ''} onfocus={() => engine('list_audio')} onchange={ev => updateSettings({ markers: { ...s.markers, sound_device: (ev.target as HTMLSelectElement).value || null } })}>
            <option value="">Windows default output</option>
            {#each app.outputs as d (d.id)}<option value={d.id}>{d.name}</option>{/each}
            {#if s.markers.sound_device && !app.outputs.some(d => d.id === s.markers.sound_device)}<option value={s.markers.sound_device}>Chosen device (not connected: default used)</option>{/if}
          </select>
        </div>
        <div class="field sub">
          <label class="lbl" for="mvol">Volume</label>
          <div class="row"><input id="mvol" type="range" min="0" max="1" step="0.05" value={s.markers.sound_volume} oninput={ev => (app.settings!.markers.sound_volume = num(ev))} onchange={ev => updateSettings({ markers: { ...s.markers, sound_volume: num(ev) } })} />
            <span class="xs mono faint">{Math.round(s.markers.sound_volume * 100)}%</span></div>
        </div>
      {/if}
    </section>

    <section id="overlay">
      <h2>Overlay &amp; notices</h2>
      <div class="field">
        <div class="lbl"><div>Overlay size</div><div class="hint">Open it in game with <kbd>{s.hotkeys.overlay || 'the overlay hotkey'}</kbd>.</div></div>
        <Segmented label="Overlay size" value={s.overlay_scale} options={[{ value: 90, label: 'Small' }, { value: 100, label: 'Normal' }, { value: 115, label: 'Large' }]} onchange={v => updateSettings({ overlay_scale: v })} />
      </div>
      <div class="field">
        <div class="lbl"><div>Small on-screen notices</div><div class="hint">Brief corner messages over your game. They never appear in recordings.</div></div>
        <Toggle checked={s.notify_on_save} label="Show on-screen notices" onchange={v => updateSettings({ notify_on_save: v })} />
      </div>
      {#if s.notify_on_save}
        <div class="field sub checks">
          {#each [['recording_started', 'Recording started'], ['saved', 'Capture triggered'], ['marker', 'Marker added'], ['problems', 'Problems']] as [k, label] (k)}
            <label class="row small"><input type="checkbox" checked={s.notices[k as keyof typeof s.notices]} onchange={ev => updateSettings({ notices: { ...s.notices, [k]: (ev.target as HTMLInputElement).checked } })} />{label}</label>
          {/each}
        </div>
      {/if}
    </section>

    <section id="games">
      <h2>Games</h2>
      <p class="foot top">Auto mode records games it recognises. These lists override it.</p>
      <div class="field">
        <div class="lbl"><div>Always treat as a game</div></div>
        <div class="chips">
          {#each s.games as g (g)}<span class="chip">{g}<button aria-label="Remove {g}" title="Remove" onclick={() => updateSettings({ games: s.games.filter(x => x !== g) })}><Icon name="x" size={12} /></button></span>
          {:else}<span class="faint small">None. Add one with “Missed a game?” on Home.</span>{/each}
        </div>
      </div>
      <div class="field">
        <div class="lbl"><div>Never record</div></div>
        <div class="chips">
          {#each s.ignored as g (g)}<span class="chip">{g}<button aria-label="Remove {g}" title="Remove" onclick={() => updateSettings({ ignored: s.ignored.filter(x => x !== g) })}><Icon name="x" size={12} /></button></span>
          {:else}<span class="faint small">None</span>{/each}
        </div>
      </div>
      {#if ruleGames.length}
        <div class="field col">
          <div class="lbl"><div>Per-game settings</div><div class="hint">“Default” uses your normal settings.</div></div>
          <div class="rules">
            {#each ruleGames as g (g)}
              {@const r = ruleFor(g)}
              <div class="rule">
                <span class="ellipsis" title={g}>{g}</span>
                <select class="select" aria-label="{g} replay length" value={r.replay_secs == null ? '' : String(r.replay_secs)} onchange={ev => { const v = (ev.target as HTMLSelectElement).value; setRule(g, { replay_secs: v ? Number(v) : null }); }}>
                  <option value="">Length: default</option>
                  {#each [30, 60, 120, 300, 600, 1200] as d (d)}<option value={String(d)}>{fmtDuration(d)}</option>{/each}
                </select>
                <select class="select" aria-label="{g} quality" value={r.quality ?? ''} onchange={ev => { const v = (ev.target as HTMLSelectElement).value; setRule(g, { quality: (v || null) as Quality | null }); }}>
                  <option value="">Quality: default</option><option value="balanced">Balanced</option><option value="high">High</option><option value="maximum">Maximum</option>
                </select>
                <select class="select" aria-label="{g} game sound" value={r.game_audio == null ? '' : String(r.game_audio)} onchange={ev => { const v = (ev.target as HTMLSelectElement).value; setRule(g, { game_audio: v === '' ? null : v === 'true' }); }}>
                  <option value="">Sound: default</option><option value="true">Record sound</option><option value="false">No sound</option>
                </select>
              </div>
            {/each}
          </div>
        </div>
      {/if}
    </section>

    <section id="storage">
      <h2>Storage</h2>
      <div class="field">
        <div class="lbl"><div>Save clips to</div><div class="hint mono ellipsis" title={s.capture_root}>{s.capture_root}</div></div>
        <div class="row"><button class="btn" onclick={changeFolder}>Change…</button><button class="btn ghost" onclick={openCaptureFolder}>Open</button></div>
      </div>
      <p class="foot">Changing the folder does not move existing clips.</p>
      <div class="field">
        <div class="lbl"><div>Background recording</div><div class="hint mono ellipsis">{s.buffer_root ? join(s.buffer_root, 'Background Recording') : `${join(s.capture_root, 'Background Recording')} (with your clips)`}</div></div>
        <div class="row">
          <button class="btn" onclick={changeBuffer}>Change…</button>
          {#if s.buffer_root}<button class="btn ghost" onclick={() => updateSettings({ buffer_root: null })}>Use clip folder</button>{/if}
        </div>
      </div>
      <div class="field">
        <label class="lbl" for="cap"><div>History and playback storage budget</div><div class="hint">Now using {fmtBytes(app.history?.bytes ?? e?.buffer_bytes ?? 0)}. The oldest footage is removed first; footage a clip is being saved from is never removed.</div></label>
        <div class="row"><input class="input num" id="cap" type="number" min="1" max="2000" step="1" value={s.buffer_cap_gb} onchange={ev => updateSettings({ buffer_cap_gb: num(ev) })} /><span class="faint small">GB</span>
          <select class="select" aria-label="History storage preset" value="" onchange={ev=>{if(ev.currentTarget.value)updateSettings({buffer_cap_gb:Number(ev.currentTarget.value)});}}><option value="">Presets…</option>{#each [20,50,100,200] as gb}<option value={gb}>{gb} GB</option>{/each}</select>
        </div>
      </div>
      <div class="field">
        <label class="lbl" for="free"><div>Pause when free space is below</div><div class="hint">{e?.free_bytes != null ? `${fmtBytes(e.free_bytes)} free now.` : ''}</div></label>
        <div class="row"><input class="input num" id="free" type="number" min="1" max="2000" step="1" value={s.min_free_gb} onchange={ev => updateSettings({ min_free_gb: num(ev) })} /><span class="faint small">GB</span></div>
      </div>
      <div class="field">
        <div class="lbl"><div>Earlier background recordings</div><div class="hint">{app.history?.reason ?? 'Temporary footage of finished sessions. Open them in the Library to clip what you want to keep; clips and screenshots are never deleted here.'}</div></div>
        <div class="row">
          <span class="small muted">{fmtBytes(e?.leftover_buffer_bytes ?? 0)}</span>
          <button class="btn sm ghost danger" disabled={!e || e.leftover_buffer_bytes === 0} onclick={clearTempCache}>Delete footage</button>
        </div>
      </div>
    </section>

    <section id="startup">
      <h2>Startup</h2>
      <div class="field">
        <div class="lbl">
          <div>Start when I sign in to Windows</div>
          <div class="hint">{app.info?.installed ? 'Starts in the tray, ready to record.' : 'Starts this portable copy in the tray. This adds a startup entry to Windows for this folder: turn it off here before you delete the folder.'}</div>
        </div>
        <Toggle checked={s.start_with_windows} label="Start with Windows" onchange={startup} />
      </div>
      <div class="field">
        <div class="lbl"><div>Keep running in the tray when closed</div><div class="hint">Off: closing the window stops recording and quits.</div></div>
        <Toggle checked={s.close_to_tray} label="Keep running in the tray" onchange={v => updateSettings({ close_to_tray: v })} />
      </div>
      <div class="field">
        <div class="lbl"><div>Minimize to the tray</div><div class="hint">Off: minimizing keeps CAPTURE on the taskbar. Recording continues either way.</div></div>
        <Toggle checked={s.minimize_to_tray} label="Minimize to the tray" onchange={v => updateSettings({ minimize_to_tray: v })} />
      </div>
    </section>

    <section id="advanced">
      <h2>Advanced</h2>
      <div class="field">
        <div class="lbl"><div>Encoder</div><div class="hint">Software works on any PC but uses the processor. Use it only if the GPU encoder causes problems.</div></div>
        <Segmented label="Encoder choice" value={s.encoder}
          options={[{ value: 'auto', label: 'Automatic (GPU)' }, { value: 'software', label: 'Software' }] as { value: 'auto' | 'software'; label: string }[]}
          onchange={v => updateSettings({ encoder: v })} />
      </div>
      <dl>
        <dt>Encoder in use</dt><dd>{e?.encoder ? `${e.encoder.name} (${e.encoder.hardware ? 'hardware' : 'software'})` : 'Chosen when recording starts'}</dd>
        {#if e?.encoder?.rejected.length}<dt>Encoders not used</dt><dd>{#each e.encoder.rejected as r}<div>{r}</div>{/each}</dd>{/if}
        <dt>Frames from Windows</dt><dd>{e?.capture_fps ?? 0} fps. Windows sends frames only when the picture changes; recordings stay at {s.fps} fps.</dd>
        <dt>Audio tracks</dt><dd>{#each e?.tracks ?? [] as t}<div>{t.name}: {t.mode}, {t.source}{t.error ? ` (${t.error})` : ''}</div>{:else}None while recording is off{/each}</dd>
        <dt>Free space</dt><dd>{e?.free_bytes != null ? fmtBytes(e.free_bytes) : 'Unknown'}</dd>
      </dl>
    </section>

    <section id="about">
      <h2>About &amp; updates</h2>
      <div class="field">
        <div class="lbl">
          <div>CAPTURE {app.info?.version ?? ''} · {app.info?.installed ? 'Installed' : 'Portable'}</div>
          <div class="hint mono ellipsis" title={app.info?.app_dir}>{app.info?.app_dir}</div>
        </div>
        <button class="btn ghost" onclick={() => openAppFolder('app')}><Icon name="folder" size={16} />Open</button>
      </div>
      <div class="field">
        <div class="lbl">
          <div>Settings, logs and cache</div>
          <div class="hint mono ellipsis" title={app.info?.data_dir}>{app.info?.data_dir}</div>
          {#if app.info?.data_fallback}<div class="hint warn">Kept here because Windows doesn’t allow changes in CAPTURE’s own folder. Move CAPTURE to a folder you can write to, to keep everything together.</div>{/if}
        </div>
        <div class="row"><button class="btn ghost" onclick={() => openAppFolder('data')}>Open</button><button class="btn ghost" onclick={() => openAppFolder('logs')}>Logs</button></div>
      </div>
      <div class="field">
        <div class="lbl">
          <div>Update CAPTURE</div>
          <div class="hint">Choose a downloaded CAPTURE release (.zip). Every file is checked first; your settings and recordings stay as they are, and CAPTURE restarts.</div>
        </div>
        <button class="btn" disabled={updBusy !== ''} onclick={chooseUpdate}><Icon name="refresh" size={16} />{updBusy === 'checking' ? 'Checking…' : 'Choose file…'}</button>
      </div>
      {#if staged}
        <div class="field sub update" data-test="staged-update">
          <div class="lbl">
            {#if staged.order > 0}<div><b>CAPTURE {staged.version}</b> is ready to install (you have {staged.current}).</div>
            {:else if staged.order === 0}<div>This is the version you already have ({staged.version}). Installing it again repairs CAPTURE’s program files.</div>
            {:else}<div class="warn">This is an older version ({staged.version}; you have {staged.current}). Your settings are backed up first, but newer options may be lost.</div>{/if}
            {#if recordingNow}<div class="hint warn">A recording is running. Stop it before installing.</div>{/if}
          </div>
          <div class="row">
            <button class="btn ghost" disabled={updBusy === 'installing'} onclick={() => (staged = null)}>Cancel</button>
            <button class="btn primary" disabled={recordingNow || updBusy === 'installing'} onclick={installUpdate}>{updBusy === 'installing' ? 'Restarting…' : staged.order > 0 ? 'Install and restart' : 'Install anyway'}</button>
          </div>
        </div>
      {/if}
      <p class="foot">Program files: {app.info?.app_dir}. Recordings are never stored there. {app.info?.installed ? 'To remove CAPTURE, use Installed apps in Windows Settings.' : 'To remove this portable copy, quit CAPTURE from the tray and delete its folder.'}</p>
    </section>
  </div>
</div>

<style>
  .settings { display: grid; grid-template-columns: 172px minmax(0, 1fr); gap: var(--s8); align-items: start; }
  aside { position: sticky; top: 0; }
  h1 { font-size: var(--fs-xl); font-weight: 650; margin: 0 0 var(--s4); }
  aside nav { display: flex; flex-direction: column; gap: 1px; }
  aside nav button { height: 32px; padding: 0 var(--s3); border: none; border-radius: var(--r-sm); background: none; color: var(--text-2);
                     font-size: var(--fs-sm); text-align: left; cursor: pointer; transition: background var(--t-fast) var(--ease), color var(--t-fast) var(--ease); }
  aside nav button:hover { background: var(--surface-2); color: var(--text); }
  aside nav button.active { background: var(--surface); color: var(--text); font-weight: 550; }
  .body { display: flex; flex-direction: column; gap: var(--s4); padding-top: 2px; }
  section { background: var(--surface); border-radius: var(--r-lg); padding: var(--s5) var(--s6); scroll-margin-top: var(--s6); }
  h2 { font-size: var(--fs-md); font-weight: 650; margin: 0 0 var(--s3); }
  .field { display: flex; align-items: center; justify-content: space-between; gap: var(--s4); padding: var(--s3) 0; border-top: 1px solid var(--line); flex-wrap: wrap; }
  h2 + .field, .foot.top + .field { border-top: none; padding-top: 0; }
  .field.sub { border-top: none; padding: 0 0 var(--s3) var(--s4); }
  .field.col { flex-direction: column; align-items: stretch; }
  .lbl { flex: 1 1 240px; min-width: 0; }
  .hint { color: var(--text-3); font-size: var(--fs-xs); margin-top: 2px; }
  .foot { color: var(--text-3); font-size: var(--fs-xs); margin: var(--s1) 0 0; }
  .foot.top { margin: 0 0 var(--s3); }
  .msg { display: flex; align-items: center; gap: var(--s2); font-size: var(--fs-sm); margin: 0 0 var(--s2); }
  .msg.warn { color: var(--warn); }
  .num { width: 84px; }
  .row { gap: var(--s2); }
  .checks { justify-content: flex-start; gap: var(--s5); }
  .chips { display: flex; flex-wrap: wrap; gap: var(--s1); justify-content: flex-end; }
  .chip { display: inline-flex; align-items: center; gap: 2px; height: 26px; padding: 0 2px 0 var(--s3); border-radius: var(--r-pill); background: var(--surface-2); font-size: var(--fs-sm); }
  .chip button { width: 22px; height: 22px; display: grid; place-items: center; border: none; border-radius: 50%; background: none; color: var(--text-3); cursor: pointer; }
  .chip button:hover { background: var(--surface-3); color: var(--text); }
  .rules { display: flex; flex-direction: column; gap: var(--s2); }
  .rule { display: grid; grid-template-columns: minmax(0, 1fr) repeat(3, auto); gap: var(--s2); align-items: center; }
  .rule .select { height: var(--h-sm); font-size: var(--fs-sm); }
  .btn.danger { color: var(--danger); }
  .update { background: var(--surface-2); border-radius: var(--r-md); padding: var(--s3) var(--s4); margin-bottom: var(--s2); }
  .warn { color: var(--warn); }
  .hint.warn { color: var(--warn); }
  dl { display: grid; grid-template-columns: 180px minmax(0, 1fr); gap: var(--s2) var(--s4); margin: var(--s3) 0 0; padding-top: var(--s3); border-top: 1px solid var(--line); font-size: var(--fs-sm); }
  dt { color: var(--text-3); }
  dd { margin: 0; overflow-wrap: anywhere; }
  @media (max-width: 1000px) {
    .settings { grid-template-columns: minmax(0, 1fr); gap: var(--s4); }
    aside { top: 0; z-index: 3; background: var(--bg); padding: var(--s2) 0; }
    section { scroll-margin-top: 120px; }
    aside nav { flex-direction: row; flex-wrap: wrap; }
    .rule { grid-template-columns: minmax(0, 1fr); }
  }
</style>
