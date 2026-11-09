// UI performance check against a running app that is recording a game (WebView2 DevTools on 9333).
//   node scripts/ui-perf.mjs <runDir>
// Measures: level-meter event rate and draw rate on Home, CPU of the UI (CAPTURE.exe + its WebView2
// processes) and of the engine with Home visible vs the window minimised, and page-switch time.
import { execFileSync } from 'node:child_process';
const [runDir] = process.argv.slice(2);
const sleep = ms => new Promise(r => setTimeout(r, ms));
let failures = 0;
const check = (ok, msg) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${msg}`); if (!ok) failures++; };
const ps = s => execFileSync('powershell', ['-NoProfile', '-Command', s]).toString().trim();

const pg = (await (await fetch('http://127.0.0.1:9333/json')).json()).find(t => t.type === 'page' && !t.url.includes('#overlay'));
const ws = new WebSocket(pg.webSocketDebuggerUrl); await new Promise(r => ws.addEventListener('open', r, { once: true }));
let id = 0; const pend = new Map(); ws.addEventListener('message', e => { const m = JSON.parse(e.data); if (pend.has(m.id)) { pend.get(m.id)(m); pend.delete(m.id); } });
const cdp = (method, params = {}) => new Promise(r => { const i = ++id; pend.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async e => (await cdp('Runtime.evaluate', { expression: e, returnByValue: true, awaitPromise: true })).result?.result?.value;
const go = t => js(`(() => { const el = [...document.querySelectorAll('[data-nav]')].find(b => b.textContent.trim() === '${t}'); el?.click(); return !!el })()`);

// CPU seconds used so far by the UI process tree and by the engine, for this run folder only.
const dir = runDir.replace(/'/g, "''");
const cpu = () => JSON.parse(ps(`
$ui = 0; $eng = 0
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -like '${dir}\\*' -or $_.CommandLine -like '*${dir}\\data*' } | ForEach-Object {
  $p = Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue
  if ($p) { if ($_.Name -eq 'capture-worker.exe') { $eng += $p.TotalProcessorTime.TotalSeconds } else { $ui += $p.TotalProcessorTime.TotalSeconds } }
}
@{ ui = $ui; eng = $eng } | ConvertTo-Json -Compress`));
// The main window is the visible top-level window titled exactly "CAPTURE" in this run's process.
// (Process.MainWindowHandle is not reliable here: while recording, the corner notice is also a
// top-level window of the same process.)
const WIN = `Add-Type -Namespace W -Name S -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool ShowWindow(System.IntPtr h, int c);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern System.IntPtr FindWindowEx(System.IntPtr p, System.IntPtr a, string c, string t);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint pid);
'@
$ids = @(Get-Process CAPTURE | Where-Object { $_.Path -like '${dir}\*' } | ForEach-Object Id)
$h = [IntPtr]::Zero; $w = [IntPtr]::Zero
while (($w = [W.S]::FindWindowEx([IntPtr]::Zero, $w, [NullString]::Value, 'CAPTURE')) -ne [IntPtr]::Zero) { $o = 0; [void][W.S]::GetWindowThreadProcessId($w, [ref]$o); if ($ids -contains $o) { $h = $w; break } }`;
const show = cmd => ps(`${WIN}; if ($h -eq [IntPtr]::Zero) { throw 'main window not found' }; [void][W.S]::ShowWindow($h, ${cmd})`);
const cores = Number(ps('(Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors'));
async function sample(secs) {
  const a = cpu(), m0 = await js('({ ...window.__captureMeters })'), t0 = Date.now();
  await sleep(secs * 1000);
  const b = cpu(), m1 = await js('({ ...window.__captureMeters })'), dt = (Date.now() - t0) / 1000;
  const pct = x => ((x / dt) * 100) / cores;
  return { ui: pct(b.ui - a.ui), eng: pct(b.eng - a.eng), events: (m1.events - m0.events) / dt, frames: (m1.frames - m0.frames) / dt };
}
const f = x => x.toFixed(2);

await go('Home');
await sleep(2000);
const vis = await sample(15);
console.log(`Home visible:   meter events ${f(vis.events)}/s, meter frames ${f(vis.frames)}/s, UI CPU ${f(vis.ui)}%, engine CPU ${f(vis.eng)}% (of ${cores} logical CPUs)`);
check(vis.events >= 25 && vis.events <= 40, `meters receive ${f(vis.events)} level updates/s (target 30)`);
check(vis.frames >= 25 && vis.frames <= 62, `meters redraw at ${f(vis.frames)} frames/s (capped near 60 whatever the display rate)`);

show(6); // SW_MINIMIZE
await sleep(2000);
const hid = await sample(15);
show(9); // SW_RESTORE
console.log(`Minimised:      meter events ${f(hid.events)}/s, meter frames ${f(hid.frames)}/s, UI CPU ${f(hid.ui)}%, engine CPU ${f(hid.eng)}%`);
check(hid.events < 1 && hid.frames < 1, 'no meter traffic or drawing while minimised');
check(hid.ui <= vis.ui, `UI CPU drops when minimised (${f(vis.ui)}% -> ${f(hid.ui)}%)`);
console.log(`Meter cost on the engine: ${f(vis.eng - hid.eng)}% CPU`);

// Restored: the page must report visible again and the meters must resume.
const t0 = Date.now();
let back = false;
while (Date.now() - t0 < 5000) { if (await js('document.visibilityState') === 'visible') { back = true; break; } await sleep(50); }
check(back, `page visible again ${Date.now() - t0} ms after restore`);
const ev0 = (await js('window.__captureMeters')).events;
await sleep(1500);
check((await js('window.__captureMeters')).events - ev0 > 20, 'meters resume after restore');

// Page switches: time from click to the next painted frame.
const times = [];
for (const p of ['Library', 'Settings', 'Replay', 'Home', 'Settings', 'Home']) {
  times.push(await js(`new Promise(r => { const t = performance.now(); [...document.querySelectorAll('[data-nav]')].find(b => b.textContent.trim() === '${p}').click(); requestAnimationFrame(() => requestAnimationFrame(() => r(performance.now() - t))); setTimeout(() => r(9999), 3000); })`));
  await sleep(400);
}
const worst = Math.max(...times);
console.log(`Page switch (click -> painted): ${times.map(t => t.toFixed(0)).join(', ')} ms`);
check(worst < 100, `every page switch paints within 100 ms (worst ${worst.toFixed(0)} ms)`);
ws.close();
console.log(failures ? `${failures} FAILED` : 'ALL PASSED');
process.exit(failures ? 1 : 0);
