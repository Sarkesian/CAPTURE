// Distribution test: the real portable zip and the real installer, end to end.
//
//   node scripts/e2e-distribution.mjs <version> <workDir on a non-C drive>
//
// Portable: extract -> every file matches the manifest -> first run (Data\ created, setup card)
// -> record a replay clip and a screenshot from the packaged engine -> --quit -> move the folder
// -> settings kept -> a saved folder on a missing drive is reported, not changed -> in-app update
// from a release zip -> delete the folder -> nothing CAPTURE-named left in AppData/Temp.
// Installer: silent install to a folder with a space on this drive -> files, shortcuts, startup
// entry, Installed-apps entry -> record -> close keeps it in the tray -> upgrade while running
// (closed, settings kept) -> uninstall keeping recordings -> reinstall -> uninstall with all
// data (only CAPTURE's own files removed).
//
// Touches CAPTURE's own per-user entries only (Run\CAPTURE, Uninstall\CAPTURE, CAPTURE.lnk in
// the Start menu / on the desktop) and restores the Run entry. Recordings go to <workDir>.
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import path from 'node:path';

const [version, work] = process.argv.slice(2);
const root = path.resolve(import.meta.dirname, '..');
const dist = path.join(root, 'dist');
const zip = path.join(dist, `CAPTURE-${version}-portable.zip`);
const setup = path.join(dist, `CAPTURE-${version}-setup.exe`);
const ffprobe = path.join(root, 'third_party', 'ffmpeg', 'bin', 'ffprobe.exe');
const sleep = ms => new Promise(r => setTimeout(r, ms));
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; return ok; };
const ps = cmd => execFileSync('powershell', ['-NoProfile', '-Command', cmd], { encoding: 'utf8' }).trim();
const RUN = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run';
const UNINST = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\CAPTURE';
const runEntry = () => ps(`(Get-ItemProperty '${RUN}' -Name CAPTURE -ErrorAction SilentlyContinue).CAPTURE`);
const uninstValue = n => ps(`(Get-ItemProperty '${UNINST}' -Name ${n} -ErrorAction SilentlyContinue).${n}`);
const savedEntry = runEntry();
const startMenu = path.join(process.env.APPDATA, 'Microsoft', 'Windows', 'Start Menu', 'Programs', 'CAPTURE.lnk');
const desktop = path.join(ps('[Environment]::GetFolderPath("Desktop")'), 'CAPTURE.lnk');
const testDesktop = !existsSync(desktop); // never replace someone's own shortcut

if (!work || /^c:/i.test(work)) { console.log('Give a work folder on a non-C drive.'); process.exit(2); }
if (ps(`@(Get-Process CAPTURE -ErrorAction SilentlyContinue).Count`) !== '0') { console.log('Quit CAPTURE first (it would receive this test\'s launches).'); process.exit(2); }
if (existsSync(path.join(process.env.APPDATA, '..', 'Local', 'Programs', 'CAPTURE')) || uninstValue('InstallLocation')) { console.log('CAPTURE is installed on this account; the installer test would update it. Uninstall it first.'); process.exit(2); }

rmSync(work, { recursive: true, force: true });
const capRoot = path.join(work, 'captures');
mkdirSync(capRoot, { recursive: true });

// --- process / webview helpers (as in e2e-lifecycle.mjs) ------------------------------------
const procs = name => ps(`@(Get-Process ${name} -ErrorAction SilentlyContinue | Where-Object { $_.Path -like '${work}\\*' } | ForEach-Object Id) -join ','`).split(',').filter(Boolean).map(Number);
const ENV = { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=9335' };
const launch = (exe, ...args) => spawn(exe, args, { cwd: 'C:\\', detached: true, stdio: 'ignore', env: ENV }).unref(); // cwd deliberately elsewhere
const waitUntil = async (pred, ms) => { const u = Date.now() + ms; while (Date.now() < u) { if (await pred()) return true; await sleep(300); } return false; };
let ws, cdpId = 0;
const pending = new Map();
async function connect() {
  for (let i = 0; i < 120; i++) {
    try {
      // The main window's page (the overlay is index.html#overlay, or about:blank while loading).
      const page = (await (await fetch('http://127.0.0.1:9335/json')).json()).find(t => t.type === 'page' && /index\.html$|localhost\/?$/.test(t.url));
      if (page) {
        ws = new WebSocket(page.webSocketDebuggerUrl);
        await new Promise(r => ws.addEventListener('open', r, { once: true }));
        ws.addEventListener('message', ev => { const m = JSON.parse(ev.data); if (pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } });
        for (let j = 0; j < 60 && !(await js('typeof window.__TAURI_INTERNALS__ === "object" && !!document.querySelector("main")').catch(() => false)); j++) await sleep(250);
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
const boot = async () => (await invoke('bootstrap')).ok;
const gone = () => waitUntil(() => procs('CAPTURE').length === 0 && procs('capture-worker').length === 0, 45000);
async function quitApp() { await invoke('quit_app').catch(() => {}); try { ws.close(); } catch {} return gone(); }

const sha = f => createHash('sha256').update(readFileSync(f)).digest('hex');
function matchesManifest(dir) {
  const m = JSON.parse(readFileSync(path.join(dir, 'manifest.json'), 'utf8'));
  const bad = m.files.filter(f => !existsSync(path.join(dir, f.path)) || sha(path.join(dir, f.path)) !== f.sha256);
  return { m, bad };
}
const walk = d => existsSync(d) ? readdirSync(d, { withFileTypes: true }).flatMap(e => e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]) : [];
const outsideSnapshot = () => [process.env.LOCALAPPDATA, process.env.APPDATA, process.env.TEMP, path.join(process.env.LOCALAPPDATA, 'Programs')]
  .flatMap(d => { try { return readdirSync(d).filter(n => /capture/i.test(n)).map(n => path.join(d, n)); } catch { return []; } });

// Turn recording on (whole display, so no game is needed), save a replay and a screenshot.
async function recordClip(label) {
  const s = (await boot()).settings;
  let r = await invoke('update_settings', { settings: { ...s, capture_root: capRoot, replay_enabled: true, target: { kind: 'display', index: 0, label: 'Display 1' }, first_run_done: true } });
  if (!check(!r.err, `${label}: recording settings accepted`)) console.log(r.err);
  const buffering = await waitUntil(async () => { const b = await boot(); return b.last_state?.status === 'buffering' && b.last_state.buffered_secs >= 4; }, 30000);
  check(buffering, `${label}: packaged engine is buffering (whole display)`);
  const before = new Set(walk(capRoot));
  await invoke('save_replay');
  await invoke('engine_command', { command: { cmd: 'screenshot' } });
  let clip = null, shot = null;
  await waitUntil(() => { const n = walk(capRoot).filter(f => !before.has(f)); clip = n.find(f => f.endsWith('.mp4')); shot = n.find(f => f.endsWith('.png')); return clip && shot; }, 40000);
  check(!!clip, `${label}: replay clip saved ${clip ? path.relative(capRoot, clip) : ''}`);
  check(!!shot, `${label}: screenshot saved`);
  if (clip) {
    const p = spawnSync(ffprobe, ['-v', 'error', '-show_entries', 'format=duration:format_tags=comment:stream=codec_type', '-of', 'json', clip], { encoding: 'utf8' });
    const j = JSON.parse(p.stdout || '{}');
    const dur = Number(j.format?.duration ?? 0);
    check(dur >= 3 && j.streams?.some(s => s.codec_type === 'video'), `${label}: clip plays (${dur.toFixed(1)} s, ${j.streams?.length} streams)`);
    check(j.format?.tags?.comment === 'Recorded with CAPTURE', `${label}: clip carries CAPTURE's ownership tag`);
  }
  return clip;
}

const outsideBefore = outsideSnapshot();
try {
  // =========================================================================================
  console.log('--- Portable');
  const pdir = path.join(work, 'portable', 'CAPTURE');
  ps(`Expand-Archive -LiteralPath '${zip}' -DestinationPath '${pdir}'`);
  let { m, bad } = matchesManifest(pdir);
  check(bad.length === 0 && m.version === version, `zip: ${m.files.length} program files match the manifest (${version})`);
  check(!existsSync(path.join(pdir, 'Data')), 'zip carries no Data folder (nobody\'s settings)');
  let exe = path.join(pdir, 'CAPTURE.exe');

  launch(exe);
  await connect();
  let b = await boot();
  check(!b.app.installed && b.app.data_dir === path.join(pdir, 'Data'), `first run: portable, data in ${b.app.data_dir}`);
  check(['Settings', 'Cache\\WebView', 'Cache\\Temp', 'Logs'].every(d => existsSync(path.join(pdir, 'Data', d))), 'Data\\Settings, Data\\Cache\\WebView, Data\\Cache\\Temp, Data\\Logs created');
  check(existsSync(path.join(pdir, 'Data', 'Cache', 'WebView', 'EBWebView')), 'WebView2 profile is inside Data\\Cache\\WebView');
  check(!b.settings.first_run_done && await js('!!document.querySelector("#setup-title")'), 'first-run setup card shown');
  check(procs('CAPTURE').length === 1 && procs('capture-worker').length === 1, 'engine runs from Runtime\\');
  await recordClip('portable');

  // Close button keeps it running in the tray; a second launch reveals it; --quit ends it.
  launch(exe, '--quit');
  check(await gone(), '--quit from a second launch: CAPTURE and its engine end');
  const log = readFileSync(path.join(pdir, 'Data', 'Logs', 'capture.log'), 'utf8');
  check(/CAPTURE \S+ starting \(portable\)/.test(log), 'log written to Data\\Logs\\capture.log');

  // Move the folder.
  const moved = path.join(work, 'elsewhere', 'Tools', 'CAPTURE (moved)');
  mkdirSync(path.dirname(moved), { recursive: true });
  renameSync(pdir, moved);
  exe = path.join(moved, 'CAPTURE.exe');
  launch(exe);
  await connect();
  b = await boot();
  check(b.app.data_dir === path.join(moved, 'Data') && b.settings.capture_root === capRoot && b.settings.replay_enabled && b.settings.first_run_done, 'moved folder: same settings, data follows the app');
  check(b.path_issues.length === 0, 'no folder warnings when everything is reachable');
  await quitApp();

  // Another PC / unplugged drive: the saved folder is reported, not silently changed.
  const sp = path.join(moved, 'Data', 'Settings', 'settings.json');
  const saved = JSON.parse(readFileSync(sp, 'utf8'));
  writeFileSync(sp, JSON.stringify({ ...saved, capture_root: 'Q:\\Videos\\Captures' }, null, 2));
  launch(exe);
  await connect();
  b = await boot();
  check(b.settings.capture_root === 'Q:\\Videos\\Captures' && b.path_issues.some(i => i.which === 'capture_root'), 'missing drive: setting kept and reported');
  if (!check(await waitUntil(() => js('!!document.querySelector("[data-test=path-issue]")'), 8000), 'missing drive: Home shows the warning'))
    console.log('   page:', (await js('document.querySelector("main")?.innerText.slice(0, 400)')));
  await invoke('update_settings', { settings: { ...b.settings, capture_root: capRoot } });

  // In-app update from a release zip (same files, version 9.9.9 in its manifest).
  const rel = path.join(work, 'release', 'CAPTURE');
  cpSync(moved, rel, { recursive: true, filter: s => !/[\\/]Data([\\/]|$)/i.test(s.slice(moved.length)) });
  const relManifest = JSON.parse(readFileSync(path.join(rel, 'manifest.json'), 'utf8'));
  relManifest.version = '9.9.9';
  writeFileSync(path.join(rel, 'manifest.json'), JSON.stringify(relManifest, null, 2));
  const relZip = path.join(work, 'release', 'CAPTURE-9.9.9-portable.zip');
  ps(`Compress-Archive -Path '${rel}\\*' -DestinationPath '${relZip}'`);
  // A damaged release is refused before anything changes.
  const badRel = path.join(work, 'release', 'bad');
  cpSync(rel, badRel, { recursive: true });
  writeFileSync(path.join(badRel, 'Resources', 'QUICKSTART.md'), 'tampered');
  ps(`Compress-Archive -Path '${badRel}\\*' -DestinationPath '${badRel}.zip'`);
  let r = await invoke('stage_update', { path: `${badRel}.zip` });
  check(!!r.err && /damaged|incomplete/i.test(r.err), `damaged release refused: ${r.err}`);
  r = await invoke('stage_update', { path: relZip });
  check(r.ok?.version === '9.9.9' && r.ok.order === 1, 'release zip verified and staged');
  r = await invoke('apply_update');
  check(!r.err, 'update started');
  try { ws.close(); } catch {}
  await waitUntil(async () => { try { const p = (await (await fetch('http://127.0.0.1:9335/json')).json()); return p.length === 0; } catch { return true; } }, 30000);
  const manifestVersion = d => { try { return JSON.parse(readFileSync(path.join(d, 'manifest.json'), 'utf8')).version; } catch { return null; } }; // absent mid-swap
  await waitUntil(() => manifestVersion(moved) === '9.9.9' && procs('CAPTURE').length === 1, 60000);
  await connect();
  b = await boot();
  check(JSON.parse(readFileSync(path.join(moved, 'manifest.json'), 'utf8')).version === '9.9.9', 'update applied (manifest now 9.9.9)');
  check(matchesManifest(moved).bad.length === 0, 'every updated program file verified');
  const toastText = await waitUntil(() => js('document.body.innerText.includes("to 9.9.9")'), 8000);
  check(toastText, 'update reported in the main window');
  check(b.settings.capture_root === capRoot && b.settings.replay_enabled, 'settings kept across the update');
  check(readdirSync(path.join(moved, 'Data', 'Settings', 'Backups')).some(n => n.startsWith('settings-before-9.9.9')), 'settings backed up before the update');
  check(await waitUntil(() => !existsSync(path.join(moved, 'Data', 'Cache', 'Update')), 40000), 'staged files and rollback copies cleaned up');
  check(await quitApp(), 'Quit: everything ends');

  // Delete the folder: nothing CAPTURE-named elsewhere.
  rmSync(path.join(work, 'elsewhere'), { recursive: true, force: true, maxRetries: 20, retryDelay: 500 });
  check(!existsSync(moved), 'portable folder deleted (nothing held open)');
  const leftovers = outsideSnapshot().filter(p => !outsideBefore.includes(p));
  check(leftovers.length === 0, `no CAPTURE files in AppData/Temp: ${leftovers.join(', ') || 'none'}`);
  check(runEntry() === savedEntry, 'no startup entry created by the portable copy');

  // =========================================================================================
  console.log('--- Installer');
  const idir = path.join(work, 'Install Dir', 'CAPTURE');
  const runSetup = (...args) => spawnSync(`"${setup}" ${args.join(' ')}`, { shell: true, windowsVerbatimArguments: true, stdio: 'ignore' }).status;
  // A folder holding another program's Runtime/Resources/Data is refused, untouched.
  const foreign = path.join(work, 'Shared Folder');
  for (const d of ['Runtime', 'Resources', 'Data']) { mkdirSync(path.join(foreign, d), { recursive: true }); writeFileSync(path.join(foreign, d, 'theirs.txt'), 'keep'); }
  const stForeign = runSetup('/S', `/D=${foreign}`);
  check(stForeign !== 0 && ['Runtime', 'Resources', 'Data'].every(d => readFileSync(path.join(foreign, d, 'theirs.txt'), 'utf8') === 'keep')
    && !existsSync(path.join(foreign, 'CAPTURE.exe')) && !uninstValue('InstallLocation'),
    `install into a folder with another program's Runtime/Resources/Data is refused, nothing changed (exit ${stForeign})`);
  const st = runSetup('/S', '/STARTUP=1', testDesktop ? '/DESKTOP=1' : '', `/D=${idir}`);
  check(st === 0, `silent install to ${idir} (exit ${st})`);
  ({ m, bad } = matchesManifest(idir));
  check(bad.length === 0, 'installed files are exactly the portable payload (manifest verified)');
  check(existsSync(path.join(idir, 'Uninstall CAPTURE.exe')) && !existsSync(path.join(idir, 'Data')), 'uninstaller added; no Data until first run');
  check(uninstValue('InstallLocation') === idir && uninstValue('DisplayVersion') === version, 'Installed apps entry (per user)');
  check(existsSync(startMenu), 'Start menu shortcut');
  if (testDesktop) check(existsSync(desktop), 'desktop shortcut (chosen)');
  check(runEntry() === `"${path.join(idir, 'CAPTURE.exe')}" --tray`, `Start with Windows entry: ${runEntry()}`);

  exe = path.join(idir, 'CAPTURE.exe');
  launch(exe);
  await connect();
  b = await boot();
  check(b.app.installed && b.app.data_dir === path.join(idir, 'Data'), 'installed copy: data next to the app');
  check(b.settings.start_with_windows, 'app shows Start with Windows on (from the installer)');
  await recordClip('installed');
  // Close button (WM_CLOSE to the main window): hidden in the tray, still recording.
  ps(`Add-Type -Name V -Namespace W3 -MemberDefinition @'
public delegate bool EnumProc(System.IntPtr h, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool PostMessage(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint p);
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode)] public static extern int GetWindowText(System.IntPtr h, System.Text.StringBuilder s, int n);
'@
[W3.V]::EnumWindows({ param($h, $l) $o = 0; [W3.V]::GetWindowThreadProcessId($h, [ref]$o) | Out-Null; if ($o -eq ${procs('CAPTURE')[0]}) { $sb = New-Object Text.StringBuilder 64; [W3.V]::GetWindowText($h, $sb, 64) | Out-Null; if ($sb.ToString() -eq 'CAPTURE') { [W3.V]::PostMessage($h, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null } }; $true }, [IntPtr]::Zero) | Out-Null`);
  await sleep(1500);
  check(!(await js('document.visibilityState === "visible"')), 'window hidden after close');
  check(procs('CAPTURE').length === 1 && procs('capture-worker').length === 1, 'closing the window keeps CAPTURE recording in the tray');
  const s = (await boot()).settings;
  await invoke('update_settings', { settings: { ...s, replay_secs: 300 } });

  // Upgrade (same version reinstall) while CAPTURE runs: closed first, settings kept.
  const st2 = runSetup('/S');
  check(st2 === 0 && await gone(), 'upgrade while running: CAPTURE was closed, install succeeded');
  check(matchesManifest(idir).bad.length === 0 && uninstValue('InstallLocation') === idir, 'one installation, same folder, files verified');
  launch(exe);
  await connect();
  b = await boot();
  check(b.settings.replay_secs === 300 && b.settings.capture_root === capRoot, 'settings kept across the upgrade');
  await quitApp();

  // Uninstall, keeping recordings.
  const clipsBefore = walk(capRoot).filter(f => f.endsWith('.mp4'));
  const waitUninstalled = () => waitUntil(() => !uninstValue('InstallLocation') && !existsSync(path.join(idir, 'Uninstall CAPTURE.exe')), 90000);
  spawnSync(path.join(idir, 'Uninstall CAPTURE.exe'), ['/S']);
  check(await waitUninstalled(), 'uninstall (keep recordings) finished');
  check(!existsSync(idir), 'install folder removed (program files, runtime, settings, cache, logs)');
  check(!existsSync(startMenu) && !existsSync(desktop) && runEntry() === savedEntry, 'shortcuts and startup entry removed');
  check(clipsBefore.length >= 2 && clipsBefore.every(existsSync), `recordings kept (${clipsBefore.length} clips)`);
  check(!existsSync(path.join(capRoot, 'Buffer')), 'temporary replay footage removed');

  // Reinstall, record, uninstall with all data.
  check(runSetup('/S', `/D=${idir}`) === 0, 'reinstall');
  launch(exe);
  await connect();
  await recordClip('reinstalled');
  await quitApp();
  const clipDir = path.dirname(walk(capRoot).find(f => f.endsWith('.mp4')));
  writeFileSync(path.join(clipDir, 'my notes.txt'), 'not CAPTURE\'s');
  writeFileSync(path.join(capRoot, 'keep-me.mp4'), 'someone else\'s video');
  spawnSync(path.join(idir, 'Uninstall CAPTURE.exe'), ['/S', '/ALLDATA']);
  check(await waitUninstalled(), 'uninstall (all data) finished');
  const left = walk(capRoot);
  check(left.filter(f => /\.(mp4|png)$/i.test(f) && !f.endsWith('keep-me.mp4')).length === 0, 'CAPTURE\'s recordings and screenshots removed (to the Recycle Bin)');
  check(existsSync(path.join(clipDir, 'my notes.txt')) && existsSync(path.join(capRoot, 'keep-me.mp4')), 'other files in those folders untouched');
  check(!existsSync(idir) && !uninstValue('InstallLocation'), 'nothing of the installation left');
} catch (e) {
  console.log('ERROR', e.stack ?? e.message);
  fail++;
} finally {
  for (const n of ['CAPTURE', 'capture-worker']) for (const p of procs(n)) ps(`Stop-Process -Id ${p} -Force -ErrorAction SilentlyContinue`);
  if (savedEntry) ps(`Set-ItemProperty '${RUN}' -Name CAPTURE -Value '${savedEntry.replace(/'/g, "''")}'`);
  else ps(`Remove-ItemProperty '${RUN}' -Name CAPTURE -ErrorAction SilentlyContinue; exit 0`);
  if (uninstValue('InstallLocation')?.startsWith(work)) ps(`Remove-Item '${UNINST}' -Recurse -Force`);
  for (const l of [startMenu, ...(testDesktop ? [desktop] : [])]) rmSync(l, { force: true });
  console.log('startup entry restored to:', runEntry() || '(none)');
}
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
process.exit(fail ? 1 : 0);
