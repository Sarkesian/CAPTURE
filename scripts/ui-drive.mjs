// Drives the real CAPTURE window over the Chrome DevTools Protocol (WebView2 remote debugging)
// and presses the real global hotkey with Windows SendInput. Used for UI acceptance runs.
//
//   1. start CAPTURE.exe with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333
//   2. node scripts/ui-drive.mjs <screenshotDir>
import { execFileSync } from 'node:child_process';
import { writeFileSync, mkdirSync } from 'node:fs';

const shots = process.argv[2] ?? 'test-output/ui';
mkdirSync(shots, { recursive: true });
const sleep = ms => new Promise(r => setTimeout(r, ms));

async function connect() {
  for (let i = 0; i < 60; i++) {
    try {
      const list = await (await fetch('http://127.0.0.1:9333/json')).json();
      const page = list.find(t => t.type === 'page' && !t.url.includes('#overlay'));
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
ws.addEventListener('message', ev => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
});
const cdp = (method, params = {}) => new Promise(r => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async expr => {
  const r = await cdp('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (r.result?.exceptionDetails) throw new Error(r.result.exceptionDetails.text + ' ' + expr);
  return r.result?.result?.value;
};
const shot = async name => {
  const r = await cdp('Page.captureScreenshot', { format: 'png' });
  writeFileSync(`${shots}/${name}.png`, Buffer.from(r.result.data, 'base64'));
  console.log('screenshot', `${shots}/${name}.png`);
};
const text = () => js('document.body.innerText');
const status = () => js(`document.querySelector('.pill')?.textContent?.trim() ?? ''`);
const clickText = (t, scope = 'button') => js(`(() => { const b = [...document.querySelectorAll('${scope}')].find(e => e.textContent.trim() === ${JSON.stringify(t)}); if (!b) return false; b.click(); return true; })()`);
const clickSel = sel => js(`(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e) return false; e.click(); return true; })()`);
async function waitFor(pred, ms, what) {
  const until = Date.now() + ms;
  while (Date.now() < until) { if (await pred()) return true; await sleep(250); }
  throw new Error('timed out waiting for ' + what + '\n---\n' + (await text()));
}
function pressGlobalHotkey() {
  // Alt+F10 via SendInput (VK_MENU=0x12, VK_F10=0x79): a real system-wide key press.
  execFileSync('powershell', ['-NoProfile', '-Command', `
Add-Type -Namespace W -Name K -MemberDefinition '[DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, System.UIntPtr e);'
[W.K]::keybd_event(0x12,0,0,[UIntPtr]::Zero); [W.K]::keybd_event(0x79,0,0,[UIntPtr]::Zero)
Start-Sleep -Milliseconds 60
[W.K]::keybd_event(0x79,0,2,[UIntPtr]::Zero); [W.K]::keybd_event(0x12,0,2,[UIntPtr]::Zero)`]);
}

await cdp('Page.enable');
await waitFor(async () => ['Ready', 'Needs attention', 'Buffering'].includes(await status()), 20000, 'initial status');
console.log('initial status:', await status());
await shot('1-capture-ready');

// Turn instant replay on with the big switch.
await clickSel('button[role="switch"][aria-label="Instant replay"]');
await waitFor(async () => (await status()) === 'Buffering', 20000, 'Buffering');
console.log('status after toggle:', await status(), '|', await js(`document.querySelector('.detail').textContent`));
await sleep(12000);
await shot('2-capture-buffering');

// 1) Real global hotkey.
const before = await js(`document.querySelector('.last .ellipsis')?.textContent ?? ''`);
pressGlobalHotkey();
await waitFor(async () => (await js(`document.querySelector('.last .ellipsis')?.textContent ?? ''`)) !== before, 20000, 'hotkey save');
const hk = await js(`document.querySelector('.last').innerText`);
console.log('HOTKEY SAVE ->', hk.replace(/\n/g, ' | '));

// 2) Save clip button.
const prev = await js(`document.querySelector('.last .ellipsis').textContent`);
await sleep(1200);
await clickText('Save clip');
await waitFor(async () => (await js(`document.querySelector('.last .ellipsis').textContent`)) !== prev, 20000, 'button save');
console.log('BUTTON SAVE ->', (await js(`document.querySelector('.last').innerText`)).replace(/\n/g, ' | '));
await shot('3-capture-saved');

// 3) Copy path really reaches the clipboard.
await clickText('Copy path');
await sleep(500);
const clip = execFileSync('powershell', ['-NoProfile', '-Command', 'Get-Clipboard']).toString().trim();
console.log('clipboard:', clip);

// 4) Library lists the clips.
await clickText('Library', 'button[role="tab"]');
await sleep(800);
console.log('library rows:', await js(`document.querySelectorAll('li').length`));
await shot('4-library');

// 5) Settings + advanced details.
await clickText('Settings', 'button[role="tab"]');
await sleep(300);
await clickSel('button[aria-expanded]');
await sleep(300);
console.log('hotkey line:', await js(`document.querySelector('p.ok, p.warn')?.textContent?.trim()`));
await shot('5-settings');
await clickText('Capture', 'button[role="tab"]');
ws.close();
