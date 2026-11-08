// Acceptance run for Auto game detection, the on-screen notice and numpad hotkeys, against the
// real CAPTURE window (WebView2 DevTools on port 9333) with real Windows key events.
//
//   node scripts/ui-auto-test.mjs <gameExe> <shotsDir>
// Expects settings with save_hotkey "Num1" and default (Auto) target.
import { execFileSync, spawn } from 'node:child_process';
import { writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';

const [gameExe, shots = 'test-output/auto-shots'] = process.argv.slice(2);
mkdirSync(shots, { recursive: true });
const sleep = ms => new Promise(r => setTimeout(r, ms));
let failures = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) failures++; };

async function connect() {
  for (let i = 0; i < 60; i++) {
    try {
      const page = (await (await fetch('http://127.0.0.1:9333/json')).json()).find(t => t.type === 'page' && !t.url.includes('#overlay'));
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await sleep(500);
  }
  throw new Error('CAPTURE webview not reachable');
}
const ws = new WebSocket(await connect());
await new Promise(r => ws.addEventListener('open', r, { once: true }));
let id = 0;
const pending = new Map();
ws.addEventListener('message', ev => { const m = JSON.parse(ev.data); if (pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } });
const cdp = (method, params = {}) => new Promise(r => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async expr => (await cdp('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true })).result?.result?.value;
const shot = async name => { const r = await cdp('Page.captureScreenshot', { format: 'png' }); writeFileSync(`${shots}/${name}.png`, Buffer.from(r.result.data, 'base64')); };
const status = () => js(`document.querySelector('.pill')?.textContent?.trim() ?? ''`);
const detail = () => js(`document.querySelector('.detail')?.textContent?.trim() ?? ''`);
const lastClip = () => js(`document.querySelector('.last .ellipsis')?.textContent ?? ''`);
async function waitFor(pred, ms) { const until = Date.now() + ms; while (Date.now() < until) { if (await pred()) return true; await sleep(250); } return false; }

function ps(script) { return execFileSync('powershell', ['-NoProfile', '-Command', script]).toString().trim(); }
const KEY = `Add-Type -Namespace W -Name K -MemberDefinition '[DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, System.UIntPtr e);'`;
// flags: 1 = KEYEVENTF_EXTENDEDKEY, 2 = KEYUP
const tap = (vk, scan, extended) => ps(`${KEY}; [W.K]::keybd_event(${vk},${scan},${extended ? 1 : 0},[UIntPtr]::Zero); Start-Sleep -Milliseconds 50; [W.K]::keybd_event(${vk},${scan},${extended ? 3 : 2},[UIntPtr]::Zero)`);

await cdp('Page.enable');
await waitFor(async () => (await status()) !== '', 20000);
check(await js(`[...document.querySelectorAll('[role=radio]')].find(b => b.getAttribute('aria-checked') === 'true')?.textContent.trim() === 'Auto (games)'`), 'fresh settings default to Auto (games)');

await js(`document.querySelector('button[role="switch"][aria-label="Instant replay"]').click()`);
check(await waitFor(async () => (await status()) === 'Watching for games', 8000), `replay on with no game -> "Watching for games" (got "${await status()}")`);
await shot('1-watching');

// Launch the game; it becomes the foreground window.
const game = spawn(gameExe, [], { cwd: path.dirname(gameExe), detached: true, stdio: 'ignore' });
check(await waitFor(async () => (await status()) === 'Buffering', 30000), `game launched -> auto-detected and Buffering (got "${await status()}: ${await detail()}")`);
const rec = await js(`document.querySelector('.card p b')?.textContent ?? ''`);
const how = await js(`document.querySelector('.card p .muted')?.textContent ?? ''`);
check(/FAULTHEART/i.test(rec), `Capture tab says Recording ${rec} ${how}`);

// The on-screen notice: visible, topmost, excluded from capture.
const ov = ps(`
Add-Type -Namespace W -Name O -MemberDefinition @'
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern System.IntPtr FindWindow(string c, string t);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr h);
[DllImport("user32.dll")] public static extern bool GetWindowDisplayAffinity(System.IntPtr h, out uint a);
'@
$h = [W.O]::FindWindow('CaptureOverlay', [NullString]::Value); $a = 0; [void][W.O]::GetWindowDisplayAffinity($h, [ref]$a)
"$([W.O]::IsWindowVisible($h)) $a"`);
check(ov.startsWith('True 17'), `on-screen notice shown and excluded from capture (visible, affinity 0x11) -> "${ov}"`);

await sleep(8000);
await shot('2-buffering');

// Num1 with NumLock ON: VK_NUMPAD1 (0x61), scan 0x4F.
let before = await lastClip();
tap(0x61, 0x4f, false);
check(await waitFor(async () => (await lastClip()) !== before, 15000), 'Num1 (NumLock on) saved a clip');
await sleep(1500);
// Num1 with NumLock OFF: the numpad key arrives as VK_END (0x23) WITHOUT the extended flag.
before = await lastClip();
tap(0x23, 0x4f, false);
check(await waitFor(async () => (await lastClip()) !== before, 15000), 'Num1 (NumLock off, arrives as non-extended End) saved a clip');
await sleep(1500);
// The real End key (extended) must NOT save.
before = await lastClip();
tap(0x23, 0x4f, true);
check(!(await waitFor(async () => (await lastClip()) !== before, 5000)), 'the real End key does not trigger a save');
console.log('last clip:', (await js(`document.querySelector('.last')?.innerText ?? ''`)).replace(/\n/g, ' | '));
await shot('3-saved');

// Close the game: status returns to waiting, footage still saveable.
process.kill(game.pid);
try { ps(`Stop-Process -Name ${path.basename(gameExe, '.exe')} -ErrorAction SilentlyContinue`); } catch {}
check(await waitFor(async () => (await status()) === 'Watching for games', 10000), `game closed -> "Watching for games" (detail: ${await detail()})`);
await shot('4-after-close');
ws.close();
console.log(failures ? `${failures} FAILED` : 'ALL PASSED');
process.exit(failures ? 1 : 0);
