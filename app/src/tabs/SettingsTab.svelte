<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import Toggle from '../components/Toggle.svelte';
  import Segmented from '../components/Segmented.svelte';
  import HotkeyField from '../components/HotkeyField.svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { app, updateSettings, clearTempCache, openCaptureFolder, fmtBytes, fmtDuration, engine, toast } from '../lib/store.svelte';
  import type { GameRule, Quality } from '../lib/types';

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

  const s = $derived(app.settings!);
  const e = $derived(app.engine);
  let advanced = $state(false);

  async function changeFolder() {
    const dir = await open({ directory: true, title: 'Choose where CAPTURE saves clips', defaultPath: s.capture_root });
    if (typeof dir === 'string') updateSettings({ capture_root: dir });
  }
  const num = (ev: Event) => Number((ev.target as HTMLInputElement).value);
</script>

<section class="card">
  <h2>Capture &amp; hotkeys</h2>
  <div class="field">
    <div class="grow"><div>Save replay hotkey</div><div class="muted small">Works while you play, even when CAPTURE is in the background.</div></div>
    <HotkeyField value={s.save_hotkey} onchange={v => updateSettings({ save_hotkey: v })} />
  </div>
  {#if e?.hotkey}
    <p class="small" class:warn={!e.hotkey.registered} class:ok={e.hotkey.registered}>
      {e.hotkey.registered ? `✓ ${e.hotkey.combo} is active.` : e.hotkey.message}
    </p>
  {/if}
  {#each [['add_marker', 'Add marker', 'Bookmarks the moment without saving a clip.', 'marker'], ['toggle_overlay', 'Open overlay', 'Shows CAPTURE on top of your game.', 'overlay'], ['toggle_session', 'Start/stop recording', 'Records a full session of any length (optional).', 'session'], ['screenshot', 'Screenshot', 'Saves the game picture as PNG in Screenshots.', 'screenshot'], ['toggle_mic_mute', 'Mute / unmute microphone', 'Works while you play.', 'mic_mute']] as [action, title, hint, key] (action)}
    {@const hs = e?.hotkeys.find(h => h.action === action)}
    <div class="field">
      <div class="grow"><div>{title}</div><div class="muted small">{hint}</div></div>
      <HotkeyField value={s.hotkeys[key as 'marker' | 'overlay' | 'session' | 'screenshot' | 'mic_mute']} onchange={v => updateSettings({ hotkeys: { ...s.hotkeys, [key]: v } })} />
    </div>
    {#if hs && hs.combo && !hs.registered && hs.message}<p class="small warn">{hs.message}</p>{/if}
  {/each}
  <div class="field">
    <div class="grow">Frame rate</div>
    <Segmented label="Frame rate" value={s.fps} options={[{ value: 30, label: '30' }, { value: 60, label: '60' }, { value: 120, label: '120' }]} onchange={v => updateSettings({ fps: v })} />
  </div>
  <div class="field">
    <div class="grow">Show the mouse cursor in recordings</div>
    <Toggle checked={s.capture_cursor} label="Show the mouse cursor" onchange={v => updateSettings({ capture_cursor: v })} />
  </div>
</section>

<section class="card">
  <h2>Markers</h2>
  <div class="field">
    <div class="grow"><div>Play a short sound when a marker is added</div><div class="muted small">CAPTURE keeps this sound out of your recordings. A microphone near your speakers can still hear it.</div></div>
    <Toggle checked={s.markers.sound} label="Marker sound" onchange={v => updateSettings({ markers: { ...s.markers, sound: v } })} />
  </div>
  {#if s.markers.sound}
    <div class="field">
      <label class="grow" for="mdev">Play the sound on</label>
      <select id="mdev" value={s.markers.sound_device ?? ''} onfocus={() => engine('list_audio')} onchange={ev => updateSettings({ markers: { ...s.markers, sound_device: (ev.target as HTMLSelectElement).value || null } })}>
        <option value="">Windows default output</option>
        {#each app.outputs as d (d.id)}<option value={d.id}>{d.name}</option>{/each}
        {#if s.markers.sound_device && !app.outputs.some(d => d.id === s.markers.sound_device)}<option value={s.markers.sound_device}>Chosen device (not connected: the default is used)</option>{/if}
      </select>
    </div>
    <div class="field">
      <label class="grow" for="mvol">Sound volume</label>
      <input id="mvol" type="range" min="0" max="1" step="0.05" value={s.markers.sound_volume} onchange={ev => updateSettings({ markers: { ...s.markers, sound_volume: Number((ev.target as HTMLInputElement).value) } })} />
    </div>
  {/if}
  <div class="field">
    <div class="grow"><div>Keep footage around markers from being overwritten</div><div class="muted small">Still temporary until you save a clip. Uses at most half of the temporary buffer limit.</div></div>
    <Toggle checked={s.markers.protect} label="Protect footage around markers" onchange={v => updateSettings({ markers: { ...s.markers, protect: v } })} />
  </div>
  {#if s.markers.protect}
    <div class="field">
      <span class="grow">Keep before / after the marker (seconds)</span>
      <input type="number" min="0" max="600" aria-label="Seconds before marker" value={s.markers.protect_before_secs} onchange={ev => updateSettings({ markers: { ...s.markers, protect_before_secs: num(ev) } })} />
      <input type="number" min="0" max="600" aria-label="Seconds after marker" value={s.markers.protect_after_secs} onchange={ev => updateSettings({ markers: { ...s.markers, protect_after_secs: num(ev) } })} />
    </div>
  {/if}
</section>

<section class="card">
  <h2>Games</h2>
  <p class="muted small">Auto mode records games it recognises. These lists override it.</p>
  <div class="field">
    <div class="grow">Always treat as a game</div>
    <div class="chips">
      {#each s.games as g (g)}<span class="chip">{g} <button class="link" aria-label="Remove {g}" onclick={() => updateSettings({ games: s.games.filter(x => x !== g) })}>✕</button></span>
      {:else}<span class="muted small">None. Add one from “Missed a game?” on the Capture tab.</span>{/each}
    </div>
  </div>
  {#if ruleGames.length}
    <div class="field col">
      <div><div>Per-game settings</div><div class="muted small">Leave on “Default” to use your normal settings.</div></div>
      {#each ruleGames as g (g)}
        {@const r = ruleFor(g)}
        <div class="rule">
          <span class="grow ellipsis">{g}</span>
          <select aria-label="{g} replay length" value={r.replay_secs == null ? '' : String(r.replay_secs)} onchange={ev => { const v = (ev.target as HTMLSelectElement).value; setRule(g, { replay_secs: v ? Number(v) : null }); }}>
            <option value="">Length: default</option>
            {#each [30, 60, 120, 300, 600, 1200] as d (d)}<option value={String(d)}>{fmtDuration(d)}</option>{/each}
          </select>
          <select aria-label="{g} quality" value={r.quality ?? ''} onchange={ev => { const v = (ev.target as HTMLSelectElement).value; setRule(g, { quality: (v || null) as Quality | null }); }}>
            <option value="">Quality: default</option><option value="balanced">Balanced</option><option value="high">High</option><option value="maximum">Maximum</option>
          </select>
          <select aria-label="{g} game sound" value={r.game_audio == null ? '' : String(r.game_audio)} onchange={ev => { const v = (ev.target as HTMLSelectElement).value; setRule(g, { game_audio: v === '' ? null : v === 'true' }); }}>
            <option value="">Sound: default</option><option value="true">Record sound</option><option value="false">No sound</option>
          </select>
        </div>
      {/each}
    </div>
  {/if}
  <div class="field">
    <div class="grow">Never record</div>
    <div class="chips">
      {#each s.ignored as g (g)}<span class="chip">{g} <button class="link" aria-label="Remove {g}" onclick={() => updateSettings({ ignored: s.ignored.filter(x => x !== g) })}>✕</button></span>
      {:else}<span class="muted small">None</span>{/each}
    </div>
  </div>
</section>

<section class="card">
  <h2>Video quality</h2>
  <div class="field">
    <div class="grow">Quality preset</div>
    <Segmented label="Quality" value={s.quality}
      options={[{ value: 'balanced', label: 'Balanced' }, { value: 'high', label: 'High' }, { value: 'maximum', label: 'Maximum' }] as { value: Quality; label: string }[]}
      onchange={v => updateSettings({ quality: v })} />
  </div>
  <p class="muted small">H.264 MP4 at the game’s own resolution, playable and editable everywhere.</p>
</section>

<section class="card">
  <h2>Storage &amp; privacy</h2>
  <div class="field">
    <div class="grow"><div>Capture folder</div><div class="mono muted ellipsis">{s.capture_root}</div></div>
    <button onclick={changeFolder}>Change…</button>
    <button onclick={openCaptureFolder}>Open</button>
  </div>
  <p class="muted small">Changing the folder does not move existing clips.</p>
  <div class="field">
    <div class="grow"><div>Temporary buffer location</div><div class="mono muted ellipsis">{s.buffer_root ? `${s.buffer_root}\Buffer` : `${s.capture_root}\Buffer (same as capture folder)`}</div></div>
    <button onclick={async () => { const dir = await open({ directory: true, title: 'Folder for temporary footage' }); if (typeof dir === 'string') updateSettings({ buffer_root: dir }); }}>Change…</button>
    {#if s.buffer_root}<button onclick={() => updateSettings({ buffer_root: null })}>Use capture folder</button>{/if}
  </div>
  <div class="field">
    <div class="grow"><div>Create separate audio files for editors</div><div class="muted small">After each save, also write one WAV per audio track next to the clip. Every track is already inside the MP4; you can make these files for any clip later from the Library.</div></div>
    <Toggle checked={s.editor_audio_files} label="Create separate audio files for editors" onchange={v => updateSettings({ editor_audio_files: v })} />
  </div>
  <div class="field">
    <label class="grow" for="cap">Temporary buffer limit (GB)</label>
    <input id="cap" type="number" min="1" max="2000" step="1" value={s.buffer_cap_gb} onchange={ev => updateSettings({ buffer_cap_gb: num(ev) })} />
  </div>
  <div class="field">
    <label class="grow" for="free">Pause the buffer when free space drops below (GB)</label>
    <input id="free" type="number" min="1" max="2000" step="1" value={s.min_free_gb} onchange={ev => updateSettings({ min_free_gb: num(ev) })} />
  </div>
  <div class="field">
    <div class="grow">
      <div>Leftover temporary footage</div>
      <div class="muted small">From earlier sessions that did not close normally. Saved clips are never touched.</div>
    </div>
    <span class="muted">{fmtBytes(e?.leftover_buffer_bytes ?? 0)}</span>
    <button disabled={!e || e.leftover_buffer_bytes === 0} onclick={() => engine('recover_leftovers')}>Save as clip</button>
    <button disabled={!e || e.leftover_buffer_bytes === 0} onclick={clearTempCache}>Delete</button>
  </div>
  <div class="field">
    <div class="grow"><div>Start CAPTURE when I sign in to Windows</div><div class="muted small">Starts in the tray, ready to record.</div></div>
    <Toggle checked={s.start_with_windows} label="Start with Windows" onchange={startup} />
  </div>
  <div class="field">
    <div class="grow"><div>Keep running in the tray when the window is closed</div><div class="muted small">Off: closing the window stops recording and quits.</div></div>
    <Toggle checked={s.close_to_tray} label="Keep running in the tray" onchange={v => updateSettings({ close_to_tray: v })} />
  </div>
  <div class="field">
    <div class="grow"><div>Minimize to the tray</div><div class="muted small">Off: minimizing keeps CAPTURE on the taskbar. Recording continues either way.</div></div>
    <Toggle checked={s.minimize_to_tray} label="Minimize to the tray" onchange={v => updateSettings({ minimize_to_tray: v })} />
  </div>
  <div class="field">
    <div class="grow"><div>Show small on-screen notices</div><div class="muted small">Brief corner messages over your game. They never appear in your recordings.</div></div>
    <Toggle checked={s.notify_on_save} label="Show on-screen notices" onchange={v => updateSettings({ notify_on_save: v })} />
  </div>
  {#if s.notify_on_save}
    <div class="field notices">
      {#each [['recording_started', 'Recording started'], ['saved', 'Clip / session / screenshot saved'], ['marker', 'Marker added'], ['problems', 'Problems (not saved, low disk…)']] as [k, label] (k)}
        <label class="row small"><input type="checkbox" checked={s.notices[k as keyof typeof s.notices]} onchange={ev => updateSettings({ notices: { ...s.notices, [k]: (ev.target as HTMLInputElement).checked } })} /> {label}</label>
      {/each}
    </div>
  {/if}
  <div class="field">
    <span class="grow">Overlay size</span>
    <Segmented label="Overlay size" value={s.overlay_scale} options={[{ value: 90, label: 'Small' }, { value: 100, label: 'Normal' }, { value: 115, label: 'Large' }]} onchange={v => updateSettings({ overlay_scale: v })} />
  </div>
</section>

<section class="card">
  <button class="link" aria-expanded={advanced} onclick={() => (advanced = !advanced)}>{advanced ? '▾' : '▸'} Advanced details</button>
  {#if advanced}
    <dl>
      <dt>Encoder choice</dt>
      <dd>
        <Segmented label="Encoder choice" value={s.encoder}
          options={[{ value: 'auto', label: 'Automatic (GPU first)' }, { value: 'software', label: 'Software (CPU)' }] as { value: 'auto' | 'software'; label: string }[]}
          onchange={v => updateSettings({ encoder: v })} />
        <div class="muted small">Software works on any PC but uses the processor; use it only if the GPU encoder causes problems.</div>
      </dd>
      <dt>Encoder</dt>
      <dd>{e?.encoder ? `${e.encoder.name} (${e.encoder.hardware ? 'hardware' : 'software'})` : 'Chosen when replay starts'}</dd>
      {#if e?.encoder?.rejected.length}
        <dt>Encoders not used</dt><dd>{#each e.encoder.rejected as r}<div class="small">{r}</div>{/each}</dd>
      {/if}
      <dt>Capture rate (frames from Windows)</dt><dd>{e?.capture_fps ?? 0} fps — Windows only sends frames when the picture changes; recordings are still a constant {s.fps} fps.</dd>
      <dt>Audio tracks</dt>
      <dd>{#each e?.tracks ?? [] as t}<div class="small">{t.name}: {t.mode} — {t.source}{t.error ? ` — ${t.error}` : ''}</div>{:else}None while replay is off{/each}</dd>
      <dt>Free space on capture drive</dt><dd>{e?.free_bytes != null ? fmtBytes(e.free_bytes) : 'unknown'}</dd>
      <dt>CAPTURE’s own data folder</dt><dd class="mono">{app.dataDir}</dd>
    </dl>
  {/if}
</section>

<style>
  section { margin-bottom: 14px; }
  .field { display: flex; align-items: center; gap: 12px; padding: 9px 0; border-top: 1px solid var(--line); flex-wrap: wrap; }
  h2 + .field { border-top: none; }
  input[type="number"] { width: 90px; }
  p { margin: 4px 0 8px; }
  .warn { color: var(--warn); }
  .ok { color: var(--ok); }
  dl { display: grid; grid-template-columns: 240px 1fr; gap: 6px 16px; margin: 12px 0 0; }
  dt { color: var(--muted); }
  dd { margin: 0; }
  .col { flex-direction: column; align-items: stretch; }
  .notices { gap: 18px; padding-left: 4px; }
  .rule { display: flex; gap: 8px; align-items: center; }
  .rule select { font-size: 13px; padding: 4px 8px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; justify-content: flex-end; }
  .chip { background: var(--panel-2); border: 1px solid var(--line); border-radius: 999px; padding: 2px 4px 2px 10px; font-size: 13px; }
  .chip .link { padding: 0 6px; }
</style>
