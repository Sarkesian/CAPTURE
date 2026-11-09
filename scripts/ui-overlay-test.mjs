// Acceptance run for the in-game overlay against the real app (WebView2 DevTools, port 9333)
// with real Windows key events. Expects replay enabled, Auto target, a game already running,
// and hotkeys Alt+F10 (save), Alt+F8 (marker), Alt+F11 (overlay).
//   node scripts/ui-overlay-test.mjs <shotsDir> <captureRoot>
import { execFileSync } from 'node:child_process';
import { writeFileSync, mkdirSync, readdirSync, existsSync } from 'node:fs';
import path from 'node:path';

const [shots = 'test-output/overlay-shots', root] = process.argv.slice(2);
mkdirSync(shots, { recursive: true });
const sleep = ms => new Promise(r => setTimeout(r, ms));
let failures = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) failures++; };
const ps = s => execFileSync('powershell', ['-NoProfile', '-Command', s]).toString().trim();
const KEY = `Add-Type -Namespace W -Name K -MemberDefinition '[DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, System.UIntPtr e);'`;
const altKey = vk => ps(`${KEY}; [W.K]::keybd_event(0x12,0,0,[UIntPtr]::Zero); [W.K]::keybd_event(${vk},0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 60; [W.K]::keybd_event(${vk},0,2,[UIntPtr]::Zero); [W.K]::keybd_event(0x12,0,2,[UIntPtr]::Zero)`);
const overlayVisible = () => ps(`Add-Type -Namespace W -Name V -MemberDefinition '[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern System.IntPtr FindWindow(string c, string t); [DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr h);'; [W.V]::IsWindowVisible([W.V]::FindWindow([NullString]::Value, 'CAPTURE overlay'))`) === 'True';
// <root>\<Game>\<sub>\*.mp4 (each game folder holds Clips / Sessions / Screenshots).
const files = sub => existsSync(root) ? readdirSync(root, { recursive: true }).map(String).filter(f => f.endsWith('.mp4') && f.split(path.sep).at(-2) === sub && !f.startsWith('Buffer')) : [];

async function page(match) {
  for (let i = 0; i < 60; i++) {
    try {
      const p = (await (await fetch('http://127.0.0.1:9333/json')).json()).find(t => t.type === 'page' && match(t.url));
      if (p) return p.webSocketDebuggerUrl;
    } catch {}
    await sleep(500);
  }
  throw new Error('page not found');
}
async function connect(url) {
  const ws = new WebSocket(url);
  await new Promise(r => ws.addEventListener('open', r, { once: true }));
  let id = 0; const pending = new Map();
  ws.addEventListener('message', ev => { const m = JSON.parse(ev.data); if (pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } });
  const cdp = (method, params = {}) => new Promise(r => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
  const js = async e => (await cdp('Runtime.evaluate', { expression: e, returnByValue: true, awaitPromise: true })).result?.result?.value;
  const shot = async name => { const r = await cdp('Page.captureScreenshot', { format: 'png' }); writeFileSync(`${shots}/${name}.png`, Buffer.from(r.result.data, 'base64')); };
  return { js, shot, close: () => ws.close() };
}
async function waitFor(pred, ms) { const u = Date.now() + ms; while (Date.now() < u) { if (await pred()) return true; await sleep(250); } return false; }
const click = (p, text, scope = 'button') => p.js(`(() => { const b = [...document.querySelectorAll('${scope}')].find(e => e.innerText.trim().startsWith(${JSON.stringify(text)}) && !e.disabled); if (!b) return false; b.click(); return true; })()`);

const main = await connect(await page(u => !u.includes('#overlay')));
check(await waitFor(async () => (await main.js(`document.querySelector('[data-test=status]')?.textContent.trim()`)) === 'Replay on', 30000), 'main window: Replay on on the running game');
const frames0 = await main.js(`0`);

// 1) Real overlay hotkey opens it.
altKey(0x7a); // Alt+F11
check(await waitFor(async () => overlayVisible(), 5000), 'Alt+F11 opens the overlay');
const ov = await connect(await page(u => u.includes('#overlay')));
await waitFor(async () => (await ov.js(`document.querySelector('.st')?.textContent`)) === 'Replay on', 10000);
check((await ov.js(`[...document.querySelectorAll('.act .big')].map(e => e.textContent).join('|')`)) === 'Save Replay|Add Marker|Replay|Start Recording', 'four primary actions visible');
await ov.shot('1-overlay-home');

// 2) Add Marker + Save Replay from the overlay.
check(await click(ov, 'Add Marker'), 'Add Marker clicked');
check(await waitFor(async () => (await ov.js(`document.body.innerText`)).includes('Marker added'), 5000), 'marker confirmation shown');
const clips0 = files('Clips').length;
await sleep(2500);
check(await click(ov, 'Save Replay'), 'Save Replay clicked');
check(await waitFor(async () => files('Clips').length > clips0, 20000), `replay clip written (${files('Clips').length} clips)`);
check(await waitFor(async () => (await ov.js(`document.querySelectorAll('.clip .dur').length`)) > 0, 15000), 'recent clip shown with its duration');
await ov.shot('2-overlay-saved');

// 3) Review Recording: real playback of buffered footage, marker on the timeline, range save.
await click(ov, 'Replay');
check(await waitFor(async () => (await ov.js(`(() => { const v = document.querySelector('.rv video'); return v && v.readyState >= 2 && v.duration > 3; })()`)), 20000), 'review video loaded while recording continues');
check(await ov.js(`document.querySelectorAll('.mk').length`) >= 1, 'marker visible on the timeline');
await ov.js(`(() => { const v = document.querySelector('.rv video'); v.currentTime = Math.max(0, v.duration - 4); return v.play().then(() => true, () => false); })()`);
await sleep(1500);
check(await ov.js(`!document.querySelector('.rv video').paused && document.querySelector('.rv video').currentTime > 0`), 'review plays');
await ov.shot('3-review');
await click(ov, 'Create clip');
check(await ov.js(`document.querySelectorAll('.handle').length === 2`), 'two trim handles shown');
await ov.shot('4-review-selection');
const clips1 = files('Clips').length;
await click(ov, 'Save clip');
check(await waitFor(async () => files('Clips').length > clips1, 20000), 'selected range saved as a clip');
await click(ov, 'Back');

// 4) Session recording.
await sleep(500);
await click(ov, 'Start Recording');
check(await waitFor(async () => (await ov.js(`document.querySelector('.act.rec .big')?.textContent`)) === 'Stop Recording', 5000), 'session started');
await sleep(5000);
await click(ov, 'Stop Recording');
check(await waitFor(async () => files('Sessions').length > 0, 20000), `session file written: ${files('Sessions')[0] ?? 'none'}`);

// 5) Esc hides; recording kept running the whole time.
await ov.js(`window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))`);
check(await waitFor(async () => !overlayVisible(), 5000), 'Esc closes the overlay');
const status1 = await main.js(`document.querySelector('[data-test=status]')?.textContent.trim()`);
check(status1 === 'Replay on', `still recording after using the overlay (status "${status1}")`);
// 6) Marker hotkey without the overlay.
altKey(0x77); // Alt+F8
check(await waitFor(async () => (await main.js(`document.body.innerText`)).includes('Marker added'), 5000), 'Alt+F8 adds a marker without opening anything');
main.close(); ov.close();
console.log(failures ? `${failures} FAILED` : 'ALL PASSED');
process.exit(failures ? 1 : 0);
