<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import Toggle from '../components/Toggle.svelte';
  import Segmented from '../components/Segmented.svelte';
  import HotkeyField from '../components/HotkeyField.svelte';
  import { app, updateSettings, clearTempCache, openCaptureFolder, fmtBytes } from '../lib/store.svelte';
  import type { Quality } from '../lib/types';

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
  <h2>Games</h2>
  <p class="muted small">Auto mode records games it recognises. These lists override it.</p>
  <div class="field">
    <div class="grow">Always treat as a game</div>
    <div class="chips">
      {#each s.games as g (g)}<span class="chip">{g} <button class="link" aria-label="Remove {g}" onclick={() => updateSettings({ games: s.games.filter(x => x !== g) })}>✕</button></span>
      {:else}<span class="muted small">None. Add one from “Missed a game?” on the Capture tab.</span>{/each}
    </div>
  </div>
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
    <button disabled={!e || e.leftover_buffer_bytes === 0} onclick={clearTempCache}>Delete temporary footage</button>
  </div>
  <div class="field">
    <div class="grow"><div>Keep running in the tray when the window is closed</div><div class="muted small">Off: closing the window stops recording and quits.</div></div>
    <Toggle checked={s.close_to_tray} label="Keep running in the tray" onchange={v => updateSettings({ close_to_tray: v })} />
  </div>
  <div class="field">
    <div class="grow"><div>Show a small on-screen notice</div><div class="muted small">When recording starts and when a clip is saved. It never appears in your recordings.</div></div>
    <Toggle checked={s.notify_on_save} label="Show on-screen notices" onchange={v => updateSettings({ notify_on_save: v })} />
  </div>
</section>

<section class="card">
  <button class="link" aria-expanded={advanced} onclick={() => (advanced = !advanced)}>{advanced ? '▾' : '▸'} Advanced details</button>
  {#if advanced}
    <dl>
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
  .chips { display: flex; flex-wrap: wrap; gap: 6px; justify-content: flex-end; }
  .chip { background: var(--panel-2); border: 1px solid var(--line); border-radius: 999px; padding: 2px 4px 2px 10px; font-size: 13px; }
  .chip .link { padding: 0 6px; }
</style>
