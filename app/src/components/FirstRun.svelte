<script lang="ts">
  // First run: only the four choices that matter before recording. Everything applies at once
  // and can be changed later in Settings; everything else uses the defaults.
  import { onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import { app, updateSettings, engine, toast } from '../lib/store.svelte';
  import Toggle from './Toggle.svelte';
  import Icon from './Icon.svelte';
  import HotkeyField from './HotkeyField.svelte';

  const s = $derived(app.settings!);
  const e = $derived(app.engine);
  // The keys people use while playing; the rest (screenshot, mic mute) stay in Settings.
  type HkKey = 'marker' | 'session' | 'overlay';
  const extraKeys: [string, string, HkKey][] = [
    ['add_marker', 'Add marker', 'marker'],
    ['toggle_session', 'Start or stop a recording', 'session'],
    ['toggle_overlay', 'Open overlay', 'overlay'],
  ];
  const auto = $derived(s.replay_enabled && s.target.kind === 'auto');
  const rootMissing = $derived(app.pathIssues.some(p => p.which === 'capture_root'));
  const micValue = $derived(!s.mic_enabled ? 'off' : (s.mic_device ?? 'default'));

  onMount(() => { engine('list_audio'); });

  async function changeFolder() {
    const dir = await open({ directory: true, title: 'Choose where CAPTURE saves recordings', defaultPath: s.capture_root });
    if (typeof dir === 'string') updateSettings({ capture_root: dir });
  }
  function setAuto(on: boolean) {
    updateSettings(on ? { replay_enabled: true, target: { kind: 'auto' } } : { replay_enabled: false });
  }
  async function setStartup(on: boolean) {
    try { await invoke('set_start_with_windows', { enabled: on }); } catch (err) { toast(String(err), 'error'); }
  }
  function setMic(v: string) {
    if (v === 'off') updateSettings({ mic_enabled: false });
    else updateSettings({ mic_enabled: true, mic_device: v === 'default' ? null : v });
  }
</script>

<section class="setup" aria-labelledby="setup-title">
  <div class="head">
    <h2 id="setup-title">Set up CAPTURE</h2>
    <p class="muted small">A few quick choices. You can change them, and everything else, in Settings.</p>
  </div>

  <div class="field">
    <div class="lbl">
      <div>Save recordings to</div>
      <div class="hint mono ellipsis" title={s.capture_root}>{s.capture_root}</div>
      {#if rootMissing}<div class="hint warn">This drive isn't connected. Choose another folder.</div>{/if}
    </div>
    <button class="btn" onclick={changeFolder}><Icon name="folder" size={16} />Change…</button>
  </div>

  <div class="field">
    <div class="lbl"><div>Record games automatically</div><div class="hint">Keeps your recent gameplay whenever a game is running, ready to save with one key.</div></div>
    <Toggle checked={auto} label="Record games automatically" onchange={setAuto} />
  </div>

  <div class="field">
    <div class="lbl">
      <div>Start with Windows</div>
      <div class="hint">{app.info?.installed ? 'Starts in the tray when you sign in.' : 'Starts this portable copy in the tray when you sign in. Turn it off before you delete the folder.'}</div>
    </div>
    <Toggle checked={s.start_with_windows} label="Start with Windows" onchange={setStartup} />
  </div>

  <div class="field">
    <div class="lbl"><div>Microphone</div><div class="hint">Off unless you choose one. It is always saved as its own track.</div></div>
    <select class="select" aria-label="Microphone" value={micValue} onchange={ev => setMic((ev.target as HTMLSelectElement).value)}>
      <option value="off">Off</option>
      <option value="default">Windows default microphone</option>
      {#each app.inputs as d (d.id)}<option value={d.id}>{d.name}{d.is_virtual ? ' (virtual device)' : ''}</option>{/each}
    </select>
  </div>

  <div class="keys">
    <div class="keys-head"><div>Hotkeys</div><div class="hint">Click a key to change it, then press the new combination. They work while you play.</div></div>
    <div class="key-row">
      <span class="small">Save replay</span>
      <div class="slot"><HotkeyField value={s.save_hotkey} label="Save replay" onchange={v => updateSettings({ save_hotkey: v })} /></div>
    </div>
    {#if e?.hotkey && !e.hotkey.registered && e.hotkey.message}<p class="hint warn"><Icon name="alert" size={14} />{e.hotkey.message}</p>{/if}
    {#each extraKeys as [action, title, key] (action)}
      {@const hs = e?.hotkeys.find(h => h.action === action)}
      <div class="key-row">
        <span class="small">{title}</span>
        <div class="slot"><HotkeyField value={s.hotkeys[key]} label={title} onchange={v => updateSettings({ hotkeys: { ...s.hotkeys, [key]: v } })} /></div>
      </div>
      {#if hs && hs.combo && !hs.registered && hs.message}<p class="hint warn"><Icon name="alert" size={14} />{hs.message}</p>{/if}
    {/each}
  </div>

  <div class="foot row">
    <span class="small muted grow">Nothing leaves this PC.</span>
    <button class="btn primary" onclick={() => updateSettings({ first_run_done: true })}><Icon name="check" size={16} />Done</button>
  </div>
</section>

<style>
  .setup { background: var(--surface); border-radius: var(--r-lg); padding: var(--s5) var(--s6); position: relative; overflow: hidden; }
  .setup::before { content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: 3px; background: var(--accent); }
  .head { margin-bottom: var(--s2); }
  h2 { font-size: var(--fs-lg); font-weight: 650; margin: 0 0 var(--s1); }
  .field { display: flex; align-items: center; justify-content: space-between; gap: var(--s4); padding: var(--s3) 0; border-top: 1px solid var(--line); flex-wrap: wrap; }
  .lbl { flex: 1 1 240px; min-width: 0; }
  .hint { color: var(--text-3); font-size: var(--fs-xs); margin-top: 2px; }
  .hint.warn { color: var(--warn); }
  .select { max-width: 320px; }
  .keys { padding: var(--s3) 0; border-top: 1px solid var(--line); }
  .keys-head { margin-bottom: var(--s2); }
  .key-row { display: flex; align-items: center; justify-content: space-between; gap: var(--s4); padding: var(--s1) 0; flex-wrap: wrap; }
  .slot { width: 240px; max-width: 100%; }
  .keys .hint.warn { display: flex; align-items: center; gap: var(--s1); }
  .foot { gap: var(--s3); padding-top: var(--s4); border-top: 1px solid var(--line); flex-wrap: wrap; }
</style>
