// Screenshot every screen of the running app (WebView2 DevTools on 9333) for UX review.
//   node scripts/ui-shots.mjs <outDir> [width height]
// Uses CDP to set the viewport size, so "small window" shots are reproducible.
import { writeFileSync, mkdirSync } from 'node:fs';
const [out = 'test-output/shots', w = '1000', h = '760'] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
const sleep = ms => new Promise(r => setTimeout(r, ms));
const vw = +w, vh = +h;
async function conn(match) {
  const pg = (await (await fetch('http://127.0.0.1:9333/json')).json()).find(t => t.type === 'page' && match(t.url));
  const ws = new WebSocket(pg.webSocketDebuggerUrl); await new Promise(r => ws.addEventListener('open', r, { once: true }));
  let id = 0; const p = new Map(); ws.addEventListener('message', e => { const m = JSON.parse(e.data); if (p.has(m.id)) { p.get(m.id)(m); p.delete(m.id); } });
  const cdp = (method, params = {}) => new Promise(r => { const i = ++id; p.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
  const js = async e => (await cdp('Runtime.evaluate', { expression: e, returnByValue: true, awaitPromise: true })).result?.result?.value;
  const shot = async (n, full = false) => {
    // The app scrolls inside <main>, so a "full" shot grows the viewport to the content height.
    const sh = full ? await js(`(() => { const m = document.querySelector('main'); return m ? m.scrollHeight + (innerHeight - m.clientHeight) : 0 })()`) : 0;
    if (sh) { await cdp('Emulation.setDeviceMetricsOverride', { width: vw, height: Math.min(sh, 4000), deviceScaleFactor: 1, mobile: false }); await sleep(400); }
    writeFileSync(`${out}/${n}.png`, Buffer.from((await cdp('Page.captureScreenshot', { format: 'png' })).result.data, 'base64'));
    if (sh) { await cdp('Emulation.setDeviceMetricsOverride', { width: vw, height: vh, deviceScaleFactor: 1, mobile: false }); await sleep(200); }
  };
  return { cdp, js, shot, close: () => ws.close() };
}
const main = await conn(u => !u.includes('#overlay'));
await main.cdp('Emulation.setDeviceMetricsOverride', { width: +w, height: +h, deviceScaleFactor: 1, mobile: false });
await sleep(800);
// Navigate via any nav element whose text matches (works for tabs or sidebars).
const go = t => main.js(`(() => { const el = [...document.querySelectorAll('[role=tab],nav a,nav button,[data-nav]')].find(b => b.textContent.trim() === '${t}'); if (el) { el.click(); return true } return false })()`);
for (const [name, label] of [['home', null], ['replay', 'Replay'], ['library', 'Library'], ['settings', 'Settings']]) {
  if (label && !(await go(label))) continue;
  await main.js(`document.querySelector('main') && (document.querySelector('main').scrollTop = 0)`);
  await sleep(1500);
  await main.shot(`${name}-${w}x${h}`);
  await main.shot(`${name}-${w}x${h}-full`, true);
}
await go('Home') || await go('Capture');
// Light theme (follows Windows; emulated here because the test machine runs dark mode).
await main.cdp('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: 'light' }] });
await sleep(800);
await main.shot(`home-light-${w}x${h}`);
await go('Settings'); await sleep(800);
await main.shot(`settings-light-${w}x${h}`);
await go('Home');
await main.cdp('Emulation.setEmulatedMedia', { features: [] });
await main.cdp('Emulation.clearDeviceMetricsOverride');
main.close();
try {
  const first = await conn(u => !u.includes('#overlay'));
  await first.js(`window.__TAURI_INTERNALS__.invoke('toggle_overlay')`); first.close();
  await sleep(2000);
  const ov = await conn(u => u.includes('#overlay'));
  await sleep(1200);
  // Start from the overlay's home view even if it was left on another view.
  await ov.js(`[...document.querySelectorAll('header button')].find(b => b.innerText.trim() === 'Back')?.click()`);
  await sleep(600);
  await ov.shot('overlay-home');
  await ov.js(`[...document.querySelectorAll('button')].find(b => /^Replay\b/.test(b.innerText.trim()))?.click()`);
  await sleep(4000);
  await ov.shot('overlay-review');
  await ov.js(`window.__TAURI_INTERNALS__.invoke('hide_overlay')`);
  ov.close();
} catch (e) { console.log('overlay shots skipped:', e.message); }
console.log('shots written to', out);
