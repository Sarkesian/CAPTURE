// Windows lifecycle test against the real CAPTURE.exe (an isolated copy; never your own install).
//
//   node scripts/e2e-lifecycle.mjs <buildDir containing CAPTURE.exe + capture-worker.exe + DLLs> <workDir>
//
// <workDir>\CAPTURE-0.3.0\data\settings.json is created as "the previous version" and
// <workDir>\CAPTURE-0.3.1\ as the new one, to check that an update keeps settings.
// Touches only CAPTURE's own per-user startup entry (HKCU\...\Run\CAPTURE), and restores it.
import { execFileSync, spawn } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const [build, work] = process.argv.slice(2);
const sleep = ms => new Promise(r => setTimeout(r, ms));
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };
const ps = cmd => execFileSync('powershell', ['-NoProfile', '-Command', cmd], { encoding: 'utf8' }).trim();
const RUN = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run';
const runEntry = () => ps(`(Get-ItemProperty '${RUN}' -Name CAPTURE -ErrorAction SilentlyContinue).CAPTURE`);
const savedEntry = runEntry();

const oldDir = path.join(work, 'CAPTURE-0.3.0');
const app = path.join(work, 'CAPTURE-0.3.1');
const capRoot = path.join(work, 'captures');
rmSync(work, { recursive: true, force: true });
mkdirSync(path.join(oldDir, 'data'), { recursive: true });
mkdirSync(app, { recursive: true });
for (const f of ['CAPTURE.exe', 'capture-worker.exe', 'avcodec-62.dll', 'avformat-62.dll', 'avutil-60.dll', 'swresample-6.dll', 'swscale-9.dll']) cpSync(path.join(build, f), path.join(app, f));
const previous = { capture_root: capRoot, replay_enabled: false, replay_secs: 600, mic_enabled: true, mic_device: '{0.0.1.00000000}.{test-mic}', other_enabled: true,
  save_hotkey: 'Alt+F9', games: ['MyGame.exe'], per_game: [{ exe: 'MyGame.exe', replay_secs: 300, quality: null, game_audio: null }], first_run_done: true, close_to_tray: true };
writeFileSync(path.join(oldDir, 'data', 'settings.json'), JSON.stringify(previous));
const exe = path.join(app, 'CAPTURE.exe');

const procs = name => ps(`@(Get-Process ${name} -ErrorAction SilentlyContinue | Where-Object { $_.Path -like '${work}\\*' } | ForEach-Object Id) -join ','`).split(',').filter(Boolean).map(Number);
// Visible top-level window titled exactly "CAPTURE" owned by the test copy (the main window).
const WINAPI = `Add-Type -Name V -Namespace W2 -MemberDefinition @'
public delegate bool EnumProc(System.IntPtr h, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool PostMessage(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint p);
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode)] public static extern int GetWindowText(System.IntPtr h, System.Text.StringBuilder s, int n);
'@
function Main($procId) { $script:found = [IntPtr]::Zero; [W2.V]::EnumWindows({ param($h, $l) $o = 0; [W2.V]::GetWindowThreadProcessId($h, [ref]$o) | Out-Null; if ($o -eq $procId) { $sb = New-Object Text.StringBuilder 64; [W2.V]::GetWindowText($h, $sb, 64) | Out-Null; if ($sb.ToString() -eq 'CAPTURE') { $script:found = $h } }; $true }, [IntPtr]::Zero) | Out-Null; $script:found }
`;
const mainVisible = () => { const p = procs('CAPTURE')[0]; return !!p && ps(`${WINAPI}; $h = Main ${p}; [int]($h -ne [IntPtr]::Zero -and [W2.V]::IsWindowVisible($h))`) === '1'; };
const closeMain = () => ps(`${WINAPI}; $h = Main ${procs('CAPTURE')[0]}; [W2.V]::PostMessage($h, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null`);
const launch = (...args) => spawn(exe, args, { cwd: app, detached: true, stdio: 'ignore', env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=9334' } }).unref();
const waitUntil = async (pred, ms) => { const u = Date.now() + ms; while (Date.now() < u) { if (await pred()) return true; await sleep(250); } return false; };

let ws, cdpId = 0;
const pending = new Map();
async function connect() {
  for (let i = 0; i < 80; i++) {
    try {
      const page = (await (await fetch('http://127.0.0.1:9334/json')).json()).find(t => t.type === 'page' && !t.url.includes('#overlay'));
      if (page) {
        ws = new WebSocket(page.webSocketDebuggerUrl);
        await new Promise(r => ws.addEventListener('open', r, { once: true }));
        ws.addEventListener('message', ev => { const m = JSON.parse(ev.data); if (pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } });
        for (let j = 0; j < 40 && !(await js('typeof window.__TAURI_INTERNALS__ === "object"').catch(() => false)); j++) await sleep(250);
        return;
      }
    } catch {}
    await sleep(250);
  }
  throw new Error('webview not reachable');
}
const js = expr => new Promise(r => { const i = ++cdpId; pending.set(i, r); ws.send(JSON.stringify({ id: i, method: 'Runtime.evaluate', params: { expression: expr, returnByValue: true, awaitPromise: true } })); })
  .then(m => { if (m.result?.exceptionDetails) throw new Error(JSON.stringify(m.result.exceptionDetails)); return m.result?.result?.value; });
const invoke = (cmd, args = {}) => js(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)}).then(v => ({ ok: v ?? null }), e => ({ err: String(e) }))`);
const settings = async () => (await invoke('bootstrap')).ok.settings;
async function quitAndWait() {
  await invoke('quit_app').catch(() => {});
  try { ws.close(); } catch {}
  return waitUntil(() => procs('CAPTURE').length === 0 && procs('capture-worker').length === 0, 40000);
}

try {
  // 1. First launch of a new version, started like Windows sign-in does.
  launch('--tray');
  await connect();
  await waitUntil(() => procs('capture-worker').length === 1, 10000);
  let s = await settings();
  check(s.save_hotkey === 'Alt+F9' && s.mic_enabled && s.mic_device === previous.mic_device && s.replay_secs === 600 && s.per_game.length === 1 && s.games[0] === 'MyGame.exe',
    'update: settings copied from the previous version folder (mic, hotkey, replay, games, per-game)');
  check(existsSync(path.join(oldDir, 'data', 'settings.json')), 'update: previous version folder left untouched');
  check(!mainVisible(), 'started with --tray: no window shown');
  check(procs('CAPTURE').length === 1 && procs('capture-worker').length === 1, 'one UI process, one capture engine');

  // 2. Repeated launches never start a second engine.
  launch('--tray');
  await sleep(2500);
  check(procs('CAPTURE').length === 1 && procs('capture-worker').length === 1 && !mainVisible(), 'second --tray launch: still one engine, stays in the tray');
  launch();
  check(await waitUntil(mainVisible, 6000), 'opening CAPTURE again brings the existing window back');
  check(procs('CAPTURE').length === 1 && procs('capture-worker').length === 1, 'still one UI process and one engine');

  // 3. Close button hides to the tray; the engine keeps running.
  const worker1 = procs('capture-worker')[0];
  closeMain();
  await sleep(1500);
  check(!mainVisible() && procs('CAPTURE').length === 1 && procs('capture-worker')[0] === worker1, 'closing the window hides CAPTURE in the tray; same engine keeps running');

  // 4. Start with Windows reflects the real registry entry.
  let r = await invoke('set_start_with_windows', { enabled: true });
  check(!r.err && runEntry() === `"${exe}" --tray` && (await settings()).start_with_windows, `enable startup -> Run entry ${runEntry()}`);
  r = await invoke('set_start_with_windows', { enabled: false });
  check(!r.err && runEntry() === '' && !(await settings()).start_with_windows, 'disable startup -> Run entry removed');

  // 5. A setting change persists across a restart; Exit stops everything.
  r = await invoke('update_settings', { settings: { ...(await settings()), replay_secs: 300, minimize_to_tray: true } });
  check(!r.err, 'settings change accepted');
  check(await quitAndWait(), 'Exit: CAPTURE and its engine both end');

  // 6. A stale startup entry (CAPTURE moved) is repaired on launch; the toggle shows it.
  ps(`Set-ItemProperty '${RUN}' -Name CAPTURE -Value '"${path.join(work, 'gone', 'CAPTURE.exe')}" --tray'`);
  launch('--tray');
  await connect();
  s = await settings();
  check(s.replay_secs === 300 && s.minimize_to_tray, 'settings survived the restart');
  check(s.start_with_windows && runEntry() === `"${exe}" --tray`, `stale startup entry repointed to this copy; toggle shows On`);
  check(existsSync(path.join(app, 'data', 'settings.json.bak')), 'last-known-good settings backup exists');

  // 7. Engine crash: restarted automatically, configuration re-applied.
  const before = procs('capture-worker')[0];
  ps(`Stop-Process -Id ${before} -Force`);
  check(await waitUntil(() => { const w = procs('capture-worker'); return w.length === 1 && w[0] !== before; }, 8000), 'engine killed -> restarted automatically');

  // 8. UI killed hard: the engine notices (stdin closed) and exits by itself; nothing orphaned.
  ps(`Stop-Process -Id ${procs('CAPTURE')[0]} -Force`);
  check(await waitUntil(() => procs('capture-worker').length === 0, 15000), 'UI killed -> engine exits on its own (no orphan)');

  // 9. Damaged settings file: values kept where readable, original kept for inspection.
  const sp = path.join(app, 'data', 'settings.json');
  const cur = JSON.parse(readFileSync(sp, 'utf8'));
  writeFileSync(sp, JSON.stringify({ ...cur, quality: 'ultra-mega', target: 42 }));
  launch('--tray');
  await connect();
  s = await settings();
  check(s.replay_secs === 300 && s.save_hotkey === 'Alt+F9' && s.mic_enabled && s.quality === 'high', 'partly unreadable settings: readable values kept, bad ones defaulted');
  r = await invoke('set_start_with_windows', { enabled: false });
  check(await quitAndWait(), 'Exit again: clean');
} catch (e) {
  console.log('ERROR', e.message);
  fail++;
} finally {
  for (const n of ['CAPTURE', 'capture-worker']) for (const p of procs(n)) ps(`Stop-Process -Id ${p} -Force -ErrorAction SilentlyContinue`);
  if (savedEntry) ps(`Set-ItemProperty '${RUN}' -Name CAPTURE -Value '${savedEntry.replace(/'/g, "''")}'`);
  else ps(`try { Remove-ItemProperty '${RUN}' -Name CAPTURE -ErrorAction Stop } catch {}`);
  console.log('startup entry restored to:', runEntry() || '(none)');
}
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
process.exit(fail ? 1 : 0);
