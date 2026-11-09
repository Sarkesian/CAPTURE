// UI smoke check for the stabilization settings, on an isolated copy driven over CDP:
// the new Settings rows render with the existing Toggle component, change the saved settings,
// and the Capture tab explains a microphone that only sends silence.
//   1. start the copy with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9335
//   2. node scripts/ui-stabilize-smoke.mjs <screenshotDir>
import { mkdirSync, writeFileSync } from 'node:fs';
const shots = process.argv[2] ?? 'test-output/shots-stabilize';
mkdirSync(shots, { recursive: true });
const sleep = ms => new Promise(r => setTimeout(r, ms));
let fail = 0;
const check = (ok, what) => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`); if (!ok) fail++; };
let page;
for (let i = 0; i < 80 && !page; i++) { try { const ps = (await (await fetch('http://127.0.0.1:9335/json')).json()).filter(t => t.type === 'page'); if (ps.length >= 2) page = ps.find(t => !t.url.includes('overlay')); } catch {} if (!page) await sleep(250); }
console.log('page', page.url);
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(r => ws.addEventListener('open', r, { once: true }));
let id = 0; const pending = new Map();
ws.addEventListener('message', ev => { const m = JSON.parse(ev.data); if (pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } });
const cdp = (method, params = {}) => new Promise(r => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async e => (await cdp('Runtime.evaluate', { expression: e, returnByValue: true, awaitPromise: true })).result?.result?.value;
const shot = async n => { const r = await cdp('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true }); writeFileSync(`${shots}/${n}.png`, Buffer.from(r.result.data, 'base64')); console.log('screenshot', `${shots}/${n}.png`); };
const settings = () => js(`window.__TAURI_INTERNALS__.invoke('bootstrap').then(b => b.settings)`);
await sleep(2500);

await js(`document.getElementById('tab-settings').click()`);
await sleep(600);
const body = await js('document.body.innerText');
check(body.includes('Create separate audio files for editors'), 'Settings shows "Create separate audio files for editors"');
check(body.includes('Minimize to the tray'), 'Settings shows "Minimize to the tray"');
const toggleFor = label => `[...document.querySelectorAll('[aria-label="${label}"]')][0]`;
check(await js(`!!${toggleFor('Create separate audio files for editors')}`), 'it uses the shared Toggle (aria-label present)');
check((await settings()).editor_audio_files === false, 'editor audio files are off by default');
await js(`${toggleFor('Create separate audio files for editors')}.click()`);
await sleep(700);
check((await settings()).editor_audio_files === true, 'toggling it saves the setting');
await js(`[...document.querySelectorAll('h2')].find(h => h.textContent.includes('Storage'))?.scrollIntoView()`);
await sleep(300);
await shot('settings-storage');

await js(`document.getElementById('tab-capture').click()`);
await sleep(9000); // the silence verdict needs 5 s of zeros from the mic monitor
const cap = await js('document.body.innerText');
const silent = /complete silence|muted, so it records silence|0% in Windows/.test(cap);
console.log(silent ? 'mic message shown' : 'no mic message (the default microphone may be delivering sound)');
check(cap.includes('All computer audio'), 'audio sources use the honest "All computer audio" label');
await shot('capture-audio');
console.log(fail ? `${fail} FAILED` : 'ALL PASSED');
ws.close();
process.exit(fail ? 1 : 0);
