// The real Library / Replay UI in a real browser (headless Edge): real mouse, wheel and keyboard
// input and real video decoding, against a mocked engine (scripts/overhaul-fixture.ts.txt).
// Nothing here touches the installed app or any recording.
//
//   node scripts/ui-overhaul-check.mjs
import fs from 'node:fs';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import assert from 'node:assert/strict';
import { createServer } from '../app/node_modules/vite/dist/node/index.js';
import { svelte } from '../app/node_modules/@sveltejs/vite-plugin-svelte/src/index.js';

const repo = path.resolve(import.meta.dirname, '..'), appRoot = path.join(repo, 'app');
const folder = `test-output/overhaul-ui-${Date.now()}`, dir = path.join(appRoot, folder);
fs.mkdirSync(dir, { recursive: true });
const made = spawnSync(path.join(repo, 'third_party/ffmpeg/bin/ffmpeg.exe'), ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=1920x1080:rate=60', '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000', '-t', '4', '-c:v', 'h264_nvenc', '-g', '30', '-bf', '0', '-c:a', 'aac', '-movflags', '+faststart', path.join(dir, 'clip.mp4')], { windowsHide: true, encoding: 'utf8' });
assert.equal(made.status, 0, made.stderr);
fs.writeFileSync(path.join(dir, 'shot.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360"><rect width="640" height="360" fill="#293c59"/><text x="40" y="180" font-size="32" fill="#8cb9ff">Screenshot</text></svg>');
fs.writeFileSync(path.join(dir, 'index.html'), '<html><title>Overhaul UI check</title><body><div id="app" style="height:100vh"></div><script type="module" src="./main.ts"></script></body></html>');
fs.copyFileSync(path.join(repo, 'scripts/overhaul-fixture.ts.txt'), path.join(dir, 'main.ts'));
const PORT = 4151, DEBUG = 9451;
const server = await createServer({ root: appRoot, configFile: false, plugins: [svelte()], logLevel: 'error', server: { host: '127.0.0.1', port: PORT, strictPort: true } });
await server.listen();
const url = `http://127.0.0.1:${PORT}/${folder}/index.html`;
const edge = spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', ['--headless=new', '--no-first-run', '--autoplay-policy=no-user-gesture-required', `--remote-debugging-port=${DEBUG}`, `--user-data-dir=${path.join(dir, 'edge-profile')}`, url], { windowsHide: true, stdio: 'ignore' });
const sleep = ms => new Promise(r => setTimeout(r, ms));
let ws, checks = 0;
try {
  let pages;
  for (let n = 0; n < 100; n++) { try { pages = await (await fetch(`http://127.0.0.1:${DEBUG}/json`)).json(); if (pages.some(p => p.type === 'page')) break; } catch { /* starting */ } await sleep(100); }
  ws = new WebSocket(pages.find(p => p.type === 'page').webSocketDebuggerUrl);
  await new Promise(r => ws.addEventListener('open', r, { once: true }));
  let id = 0;
  const pending = new Map(), errors = [];
  ws.addEventListener('message', e => { const m = JSON.parse(e.data); if (m.id) { pending.get(m.id)?.(m); pending.delete(m.id); } if (m.method === 'Runtime.exceptionThrown') errors.push(m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text); });
  const cdp = (method, params = {}) => new Promise(r => { pending.set(++id, r); ws.send(JSON.stringify({ id, method, params })); });
  const js = async expression => { const m = await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }); if (m.result?.exceptionDetails) throw Error(JSON.stringify(m.result.exceptionDetails)); return m.result?.result?.value; };
  const check = (ok, label) => { assert.ok(ok, label); checks++; console.log('PASS ' + label); };
  const wait = async (expr, label, ms = 6000) => { const until = Date.now() + ms; while (Date.now() < until) { if (await js(expr)) return; await sleep(30); } throw Error('timed out: ' + label + '\n' + errors.join('\n') + '\n' + await js('JSON.stringify([window.fixtureReady, window.fixtureCalls?.map(c=>c.cmd).slice(-6), window.fixtureApp?.history?.sessions?.length]) + document.body.innerText.slice(0,300)')); };
  // `sel`: a CSS selector, or `js:<expression>` giving the element.
  const element = sel => sel.startsWith('js:') ? sel.slice(3) : `document.querySelector(${JSON.stringify(sel)})`;
  const rect = sel => js(`(()=>{const r=(${element(sel)}).getBoundingClientRect();return {x:r.x,y:r.y,w:r.width,h:r.height}})()`);
  const witcher = `document.querySelectorAll('section[aria-label="Background recordings"] .grid')[1]`;
  const mouse = (type, x, y, buttons = 0, extra = {}) => cdp('Input.dispatchMouseEvent', { type, x, y, button: buttons ? 'left' : 'none', buttons, clickCount: type === 'mouseMoved' ? 0 : 1, ...extra });
  const clickAt = async (x, y) => { await mouse('mousePressed', x, y, 1); await mouse('mouseReleased', x, y, 0, { button: 'left' }); await sleep(90); };
  const click = async (sel, fx = .5, fy = .5) => { const b = await rect(sel); await clickAt(b.x + b.w * fx, b.y + b.h * fy); };
  const key = async (k, code = k) => { await cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: k, code }); await cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code }); await sleep(60); };
  const shot = async name => { const r = await cdp('Page.captureScreenshot', { format: 'png' }); fs.writeFileSync(path.join(dir, name + '.png'), Buffer.from(r.result.data, 'base64')); };
  const size = (width, height) => cdp('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
  const calls = cmd => js(`window.fixtureCalls.filter(c=>c.cmd===${JSON.stringify(cmd)}).map(c=>c.args)`);
  const now = () => js('Number(document.querySelector(".detail").getAttribute("aria-valuenow"))');
  const view = async () => ({ a: await js('Number(document.querySelector(".detail").getAttribute("aria-valuemin"))'), b: await js('Number(document.querySelector(".detail").getAttribute("aria-valuemax"))') });
  // The time (seconds) drawn into the mocked thumbnail currently in the hover card, or null.
  const shownThumb = () => js(`(()=>{const i=document.querySelector('.hover-card .frame img');if(!i)return null;const m=decodeURIComponent(i.src).match(/data-t="([^"]+)"/);return m?Number(m[1]):null})()`);
  const hoverAt = async fraction => { const b = await rect('.detail'); await mouse("mouseMoved", b.x + b.w * fraction, b.y + 26); const v = await view(); return v.a + (v.b - v.a) * fraction; };

  await cdp('Runtime.enable');
  await cdp('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: 'dark' }] });
  await size(1280, 860);

  // The fixture's Witcher 3 history on one line: 30 min, a 2 s flicker, then 3500 s and 3580 s
  // (the game changed resolution in between). Breaks are a fixed width.
  const TOTAL = 1800 + 2 + 3500 + 3580, GAP = .02, END = TOTAL + 3 * GAP;
  const PIECES = [['w-early', 0, 1800], ['flicker', 0, 2], ['w-late', 0, 3500], ['w-late', 3520, 7100]].map(([session, a, b], i, all) => ({ session, a, b, v: all.slice(0, i).reduce((n, p) => n + p[2] - p[1] + GAP, 0) }));
  const realAt = v => { const p = PIECES.find(p => v >= p.v && v < p.v + p.b - p.a); return p ? { session: p.session, t: p.a + v - p.v } : null; };
  const shownThumbAt = () => js(`(()=>{const i=document.querySelector('.hover-card .frame img');if(!i)return null;const m=decodeURIComponent(i.src).match(/data-t="([^:"]+):([^"]+)"/);return m?{session:m[1],t:Number(m[2])}:null})()`);
  const card = name => `[...document.querySelectorAll('.rec')].find(c=>c.textContent.includes(${JSON.stringify(name)}))`;

  // ---- Library: one grid, games side by side ---------------------------------------------------
  await wait('window.fixtureReady && document.querySelectorAll(".rec").length===2', 'game cards', 30000);
  check(await js('document.querySelectorAll("section .grid").length===2 && document.querySelectorAll("section[aria-label=\\"Background recordings\\"] .rec").length===2 && document.querySelectorAll("section[aria-label=\\"Saved media\\"] .saved-card:not(.rec)").length===2'), 'two sections: background recordings (one per game), then saved media');
  check(await js('[...document.querySelectorAll("section .grid")].every(g=>new Set([...g.children].map(c=>Math.round(c.getBoundingClientRect().top))).size===1)'), 'cards sit side by side within each section');
  check(await js('[...document.querySelectorAll(".rec .card-text strong")].map(e=>e.textContent).join("|")') === 'Hollow Knight|The Witcher 3', 'one background-recording card per game, most recently played first');
  check(await js(`${card('The Witcher 3')}.querySelector('.duration').textContent.trim()`) === '2h 28m', 'a game\'s card shows all of its recorded time');
  await wait('document.querySelectorAll(".rec .thumb img").length===2', 'game thumbnails');
  check(await js('new Set([...document.querySelectorAll(".rec .thumb img")].map(i=>i.src)).size===2'), 'each card shows a frame of its own footage');
  check(await js('[...document.querySelectorAll(".saved-card .card-text")].every(e=>getComputedStyle(e).opacity==="0") && [...document.querySelectorAll(".saved-card:not(.rec) .tag")].every(e=>getComputedStyle(e).opacity==="0")'), 'thumbnails are clean at rest: no name, time or type over them');
  check(await js('[...document.querySelectorAll(".rec .tag")].every(e=>getComputedStyle(e).opacity==="1")'), 'background recordings keep their Background tag');
  const tile = await rect('section[aria-label="Saved media"] .saved-card');
  await mouse('mouseMoved', tile.x + tile.w / 2, tile.y + tile.h / 2);
  await sleep(220);
  check(await js('(()=>{const c=document.querySelector("section[aria-label=\\"Saved media\\"] .saved-card");return getComputedStyle(c.querySelector(".card-text")).opacity==="1" && getComputedStyle(c.querySelector(".tag")).opacity==="1"})()') && (await rect('section[aria-label="Saved media"] .saved-card')).h === tile.h, 'hovering shows name, time and type without moving the card');
  await shot('library-hover');
  await mouse('mouseMoved', 5, 5);
  await sleep(200);
  check(await js(`${card('Hollow Knight')}.querySelector('.now').textContent.includes('Recording') && document.body.innerText.includes('Temporary')`), 'what is being recorded is marked, and temporary footage is told apart from saved media');
  await shot('library');

  // ---- Viewer: the game's whole history on one timeline --------------------------------------
  await click(`js:${card('The Witcher 3')}.querySelector('.tile')`);
  await wait('document.querySelector(".stage video")?.readyState>=2', 'recording opens');
  check(await js('document.querySelectorAll("[role=slider]").length===1 && !document.querySelector(".session-nav")'), 'one timeline, nothing to pick first');
  check(await js('document.querySelectorAll(".footage").length===4'), 'every recorded stretch of the game is visible at once, separated by breaks');
  let v = await view();
  check(v.a === 0 && Math.abs(v.b - END) < .01, 'the whole history is shown first ' + JSON.stringify(v) + ' ' + END);
  check(await js('document.querySelectorAll(".session-label").length>=2'), 'sessions are labelled with their date');
  check(await js('document.querySelectorAll(".saved-clip").length===1 && document.querySelectorAll(".moment").length===3'), 'saved clips, markers and screenshots sit on the timeline');
  check(await js('document.querySelector(".time").textContent.includes("/ 2:28:02")'), 'the total is the footage there is (breaks do not count)');
  check(Math.abs(await now() - (END - 25)) < 1, 'it opens on the most recent gameplay');

  // ---- Hover previews -----------------------------------------------------------------------
  let at = await hoverAt(.3);
  await wait('!!document.querySelector(".hover-card .frame img")', 'hover thumbnail');
  let thumb = await shownThumbAt(), expected = realAt(at);
  check(thumb.session === expected.session && thumb.t <= expected.t && expected.t - thumb.t <= 1.05, `the hover picture is the frame under the cursor (${thumb.session} ${thumb.t} for ${expected.t.toFixed(1)})`);
  check(await js('/\\d:\\d\\d:\\d\\d/.test(document.querySelector(".hover-card .when b").textContent) && /\\d{1,2}:\\d\\d/.test(document.querySelector(".hover-card .when small").textContent)'), 'the tooltip shows the footage time and the time of day');
  await shot('hover');
  // Fast movement across sessions: the old picture never stays up for a different place.
  const stale = [];
  for (const fraction of [.82, .12, .64, .1, .9]) {
    at = await hoverAt(fraction);
    const shown = await shownThumbAt(), want = realAt(at);
    if (shown && (shown.session !== want.session || want.t - shown.t > 45 || shown.t - want.t > .2)) stale.push([want, shown]);
  }
  check(stale.length === 0, 'no outdated frame during fast cursor movement ' + JSON.stringify(stale));
  expected = realAt(at);
  await wait(`(()=>{const i=document.querySelector('.hover-card .frame img');if(!i)return false;const m=decodeURIComponent(i.src).match(/data-t="([^:"]+):([^"]+)"/);return m[1]===${JSON.stringify(expected.session)}&&Math.abs(Number(m[2])-${expected.t})<=1.05})()`, 'final hover picture');
  check((await calls('history_thumb')).every(a => a.viewer === 'main-timeline' || a.viewer === 'library'), 'thumbnail requests name their view so the engine can drop outdated ones');
  const inBreak = PIECES[3].v - GAP / 2;
  await hoverAt(inBreak / END);
  await sleep(150);
  check(await js('!document.querySelector(".hover-card .frame") && document.querySelector(".hover-card").textContent.includes("Not recorded")'), 'hovering a break says nothing was recorded there');
  await hoverAt(.9995);
  await sleep(60);
  const edge = await js('document.querySelector(".hover-card .when b").textContent');
  check(edge.startsWith('2:27:') || edge.startsWith('2:28:0'), `cursor, playhead and total share one clock (${edge} at the end of 2:28:02)`);

  // ---- Seeking ------------------------------------------------------------------------------
  await click('.detail', inBreak / END, .45);
  await wait('!window.fixtureApp.reviewLoading', 'seek settles');
  let position = await now();
  check(Math.abs(position - PIECES[3].v) < 1.1 || Math.abs(position - (PIECES[2].v + 3500)) < 1.1, `a click in a break lands on the nearest recorded moment`);
  check(await js('!document.querySelector(".stage-message")'), 'no error for clicking a break');
  await click('.detail', (PIECES[2].v + 3497) / END, .45);
  await wait('!window.fixtureApp.reviewLoading', 'seek settles');
  await click('[aria-label="Forward 10 seconds"]');
  await wait('!window.fixtureApp.reviewLoading', 'skip settles');
  check(await now() >= PIECES[3].v - .01 && await now() <= PIECES[3].v + 30, 'skipping forward crosses the break into the next recorded stretch');
  await click('.detail', .15, .45);
  await wait('!window.fixtureApp.reviewLoading && document.querySelector(".stage video")?.readyState>=2', 'seek to 15%');
  check(Math.abs(await now() - END * .15) < 16 && (await calls('history_review')).at(-1).session === 'w-early', 'a click seeks to that moment of that session');
  const before = (await calls('history_review')).length;
  const strip = await rect('.detail');
  await mouse('mousePressed', strip.x + strip.w * .15, strip.y + 26, 1);
  for (let n = 1; n <= 30; n++) await mouse('mouseMoved', strip.x + strip.w * (.15 + n * .65 / 30), strip.y + 26, 1);
  await mouse('mouseReleased', strip.x + strip.w * .8, strip.y + 26, 0, { button: 'left' });
  await wait('!window.fixtureApp.reviewLoading && document.querySelector(".stage video")?.readyState>=2', 'scrub settles');
  check(Math.abs(await now() - END * .8) < 16 && !(await js('window.fixtureApp.reviewError')), 'rapid scrubbing ends exactly where the mouse was released, without an error');
  check((await calls('history_review')).length - before <= 12 && (await calls('history_review')).every(a => a.viewer === 'main'), 'scrubbing does not flood the engine with window requests');
  position = await now();
  await key(' ', 'Space');
  await wait('!document.querySelector(".stage video").paused', 'playback starts');
  await sleep(400);
  check(await now() > position, 'the playhead follows playback');
  await key(' ', 'Space');

  // ---- Continuous playback across windows (each window is its own file) ----------------------
  await key(' ', 'Space');
  await wait('!document.querySelector(".stage video").paused', 'playback starts again');
  const from = await now(), reviewsBefore = (await calls('history_review')).length;
  const seen = await js(`new Promise(done=>{const r={held:false,loading:false,paused:false,waiting:false,max:0};const t0=performance.now();let last=performance.now(),lastT=-1;
    const tick=()=>{const v=document.querySelector('.stage video:not(.waiting)');
      if(document.querySelector('.still.show'))r.held=true;if(window.fixtureApp.reviewLoading)r.loading=true;if(document.querySelector('.stage video.waiting'))r.waiting=true;
      if(document.querySelector('.play [aria-label]')||document.querySelector('button.play[aria-label=Play]'))r.paused=true;
      const t=v?v.currentTime+':'+v.src:lastT;if(t!==lastT){r.max=Math.max(r.max,performance.now()-last);last=performance.now();lastT=t;}
      if(performance.now()-t0>9000)done(r);else requestAnimationFrame(tick);};tick();})`);
  const moved = await now() - from;
  check(seen.waiting && (await calls('history_review')).slice(reviewsBefore).some(c => c.prefetch), 'the next window is loaded behind the picture before it is needed');
  check(!seen.held && !seen.loading && !seen.paused, 'crossing into the next window never shows a held frame, a loading notice or a paused button ' + JSON.stringify(seen));
  check(moved > 7.5 && seen.max < 250, `playback keeps moving across windows (${moved.toFixed(1)} s in 9 s, longest stall ${Math.round(seen.max)} ms)`);
  await key(' ', 'Space');

  // ---- Zoom ---------------------------------------------------------------------------------
  const b = await rect('.detail');
  for (let n = 0; n < 30; n++) { await cdp('Input.dispatchMouseEvent', { type: 'mouseWheel', x: b.x + b.w * .5, y: b.y + 26, deltaX: 0, deltaY: -100 }); await sleep(25); }
  await sleep(500);
  v = await view();
  check(v.b - v.a < 120 && (await now()) > v.a && (await now()) < v.b, 'the wheel zooms in on the playhead');
  check(await js('document.querySelectorAll(".tick i").length>=3'), 'zoomed in, the timeline is labelled with footage times');
  await hoverAt(.37);
  await wait('!!document.querySelector(".hover-card .frame img")', 'zoomed thumbnail');
  await sleep(200);
  check((await calls('history_thumb')).at(-1).exact === true, 'zoomed in, the hover picture is the exact frame');
  await key('0', 'Digit0');
  await sleep(600);
  v = await view();
  check(v.a === 0 && Math.abs(v.b - END) < .01, 'one key returns to the whole history');

  // ---- No black frame while another moment opens ---------------------------------------------
  await click('.detail', .15, .45);
  await wait('!window.fixtureApp.reviewLoading && document.querySelector(".stage video")?.readyState>=2', 'settled before far seek');
  const far = await rect('.detail');
  await mouse('mousePressed', far.x + far.w * .7, far.y + 40, 1);
  await mouse('mouseReleased', far.x + far.w * .7, far.y + 40, 0, { button: 'left' });
  check(await js('!!document.querySelector(".still.show")'), 'the last picture stays up while a moment elsewhere opens');
  await wait('!window.fixtureApp.reviewLoading && document.querySelector(".stage video")?.readyState>=2', 'far seek settles');
  await wait('!document.querySelector(".still.show")', 'the held picture is released once the new one is drawn');

  // ---- Fullscreen: controls over the picture, leaving while it plays ---------------------------
  await key('f', 'KeyF');
  await wait('document.querySelector(".history").classList.contains("full")', 'fullscreen');
  const dockTop = (await rect('.viewer-dock')).y, stageBox = await rect('.stage');
  check(stageBox.h > dockTop && await js('getComputedStyle(document.querySelector(".viewer-dock")).opacity==="1"'), 'in fullscreen the controls lie over the picture');
  await shot('fullscreen');
  await mouse('mouseMoved', 400, 300);
  await key(' ', 'Space');
  await wait('document.querySelector(".history").classList.contains("idle")', 'controls leave during playback', 5000);
  await sleep(300);
  check(await js('getComputedStyle(document.querySelector(".viewer-dock")).opacity==="0"'), 'they are gone while it plays untouched');
  await mouse('mouseMoved', 400, 300);
  await mouse('mouseMoved', 420, 320);
  await sleep(300);
  check(await js('!document.querySelector(".history").classList.contains("idle") && getComputedStyle(document.querySelector(".viewer-dock")).opacity==="1"'), 'moving the mouse brings them back');
  await key(' ', 'Space');
  await key('f', 'KeyF');
  await wait('!document.querySelector(".history").classList.contains("full")', 'leaves fullscreen');

  // ---- Clip ---------------------------------------------------------------------------------
  await click('.detail', .15, .45);
  await wait('!window.fixtureApp.reviewLoading', 'seek before clip');
  // Right-click: what can be done at that moment.
  const menuAt = await rect('.detail');
  await mouse('mousePressed', menuAt.x + menuAt.w * .4, menuAt.y + 48, 2, { button: 'right' });
  await mouse('mouseReleased', menuAt.x + menuAt.w * .4, menuAt.y + 48, 0, { button: 'right' });
  await wait('!!document.querySelector(".context")', 'context menu');
  check(await js('document.querySelector(".context").textContent.includes("Clip around here")'), 'a right-click on the timeline offers to clip that moment');
  await shot('context');
  await key('Escape');
  check(await js('!document.querySelector(".context")'), 'Escape closes the menu');
  await click('.transport-end .clip');
  await wait('document.querySelectorAll(".handle").length===2 && !document.querySelector(".clipbar .primary").disabled', 'clip range ready');
  const selected = parseFloat(await js('document.querySelector(".clipbar .clip-length").textContent'));
  check(await js('!!document.querySelector(".clip-range")') && selected >= 44 && selected <= 47, `Clip selects a range around the playhead and shows its length (${selected}s)`);
  const handle = await rect('.handle.end');
  await mouse('mousePressed', handle.x + handle.w * .15, handle.y + handle.h / 2, 1);
  for (let n = 1; n <= 6; n++) await mouse('mouseMoved', handle.x + n * 5, handle.y + handle.h / 2, 1);
  await mouse('mouseReleased', handle.x + 30, handle.y + handle.h / 2, 0, { button: 'left' });
  await wait('!document.querySelector(".clipbar .primary").disabled', 'trim aligned');
  check(parseFloat(await js('document.querySelector(".clipbar .clip-length").textContent')) > selected + 1, 'dragging a handle changes the range, and release ends the drag');
  await shot('clip');
  // Snapping: a handle dragged to within a few pixels of the playhead lands exactly on it.
  const head = await rect('.playhead'), startHandle = await rect('.handle.start');
  await mouse('mousePressed', startHandle.x + startHandle.w - 4, startHandle.y + 18, 1);
  for (let n = 1; n <= 8; n++) await mouse('mouseMoved', startHandle.x + startHandle.w - 4 + (head.x - 3 - startHandle.x - startHandle.w) * n / 8, startHandle.y + 18, 1);
  check(await js('!!document.querySelector(".snap-line") && document.querySelector(".hover-card").textContent.includes("Snapped to the playhead")'), 'a handle near the playhead snaps to it and says so');
  await shot('snap');
  await mouse('mouseReleased', head.x - 3, startHandle.y + 18, 0, { button: 'left' });
  await wait('!document.querySelector(".clipbar .primary").disabled', 'trim aligned after snap');
  check(await js('!document.querySelector(".snap-line") && !document.querySelector(".hover-card")'), 'nothing stays attached to the cursor after release');
  await click('.clipbar .primary');
  await wait('!document.querySelector(".handle")', 'saved, back to watching');
  const saved = (await calls('history_save')).at(-1);
  check(saved.session === 'w-early' && saved.aligned === true && saved.end - saved.start > 5 && saved.end <= 1800, 'Save Clip saves exactly the selected range of the session it lies in');
  check(await js('document.body.innerText.includes("Clip saved")'), 'saving is confirmed');
  await click('.back');
  await wait('document.querySelectorAll(".rec").length===2', 'back in the Library');

  // ---- Deleting temporary footage -----------------------------------------------------------
  await js(`${card('The Witcher 3')}.querySelector('.more').click()`);
  await sleep(80);
  await js('[...document.querySelectorAll(".menu button")].find(b=>b.textContent.includes("Delete footage")).click()');
  await sleep(80);
  check(await js('document.querySelector(".menu-note")?.textContent.includes("kept")'), 'deleting footage asks first and says saved media is kept');
  await js('document.querySelector(".menu .danger").click()');
  await wait('document.querySelectorAll(".rec").length===1', 'game removed');
  check((await calls('history_delete')).map(a => a.session).sort().join() === 'flicker,w-early,w-late' && await js('document.querySelectorAll(".saved-card:not(.rec)").length===2'), 'only that game\'s temporary footage is deleted; saved media stays');

  // ---- Small window and the in-game overlay panel -------------------------------------------
  await size(760, 600);
  await click('.rec .tile');
  await wait('document.querySelector(".stage video")?.readyState>=2', 'opens at 760x600');
  check(await js('document.documentElement.scrollWidth<=760 && document.querySelector(".transport").getBoundingClientRect().right<=760 && document.querySelector(".stage").getBoundingClientRect().height>=170'), 'the viewer fits a 760×600 window');
  check(await js('!!document.querySelector(".live")'), 'a game being recorded is marked in its viewer');
  await shot('small');
  await size(900, 620);
  await cdp('Page.navigate', { url: url + '#compact' });
  await sleep(300);
  await js('window.fixtureReady=false');
  await cdp('Page.reload');
  await wait('window.fixtureReady && document.querySelector(".stage video")?.readyState>=2', 'overlay panel opens');
  check(await js('document.querySelector("select[aria-label=Game]").value==="Hollow Knight" && !!document.querySelector(".live")'), 'the overlay opens the game being recorded');
  const length = await js('document.querySelector(".time").textContent');
  const started = await now();
  await sleep(5600);
  check(await js('document.querySelector(".time").textContent') !== length, 'the timeline keeps up as more footage is recorded');
  check(Math.abs(await now() - started) < .01, 'the playhead stays put while the recording grows');
  check(await js('document.querySelectorAll("[role=slider]").length===1 && !!document.querySelector(".transport-end .primary")'), 'the overlay has the same timeline and Clip action');
  check((await calls('history_review')).every(a => a.viewer === 'main'), 'requests are attributed to their view');
  await shot('compact');
  check(errors.length === 0, 'no script errors ' + errors.join(' | '));
  console.log(`ALL ${checks} UI CHECKS PASSED · screenshots in ${dir}`);
} finally {
  try { ws?.close(); } catch { /* closed */ }
  edge.kill();
  await server.close();
}
