// Phase 0 spike B: drive a portable libobs (OBS 33) over obs-websocket v5 to build
// display capture + two per-application audio captures routed to separate tracks,
// run the replay buffer, save it, and report the saved file path and OBS stats.
// Usage: node obs_spike.mjs <bufferSeconds>
const secs = Number(process.argv[2] ?? 12);
const ws = new WebSocket('ws://127.0.0.1:4455');
let nextId = 1;
const pending = new Map();
const sleep = ms => new Promise(r => setTimeout(r, ms));

function req(requestType, requestData = {}) {
  const requestId = String(nextId++);
  ws.send(JSON.stringify({ op: 6, d: { requestType, requestId, requestData } }));
  return new Promise((resolve, reject) => pending.set(requestId, { resolve, reject, requestType }));
}

ws.onmessage = ev => {
  const m = JSON.parse(ev.data);
  if (m.op === 0) ws.send(JSON.stringify({ op: 1, d: { rpcVersion: 1 } }));
  if (m.op === 2) main().catch(e => { console.error('FAIL', e.message); process.exit(1); });
  if (m.op === 7) {
    const p = pending.get(m.d.requestId);
    pending.delete(m.d.requestId);
    if (m.d.requestStatus.result) p.resolve(m.d.responseData ?? {});
    else p.reject(new Error(`${p.requestType}: ${m.d.requestStatus.code} ${m.d.requestStatus.comment ?? ''}`));
  }
};
ws.onerror = e => { console.error('ws error', e.message); process.exit(1); };

// OBS window-match strings are "title:class:exe" with ':' escaped as '#3A'.
const win = (title, cls, exe) => [title, cls, exe].map(s => s.replaceAll(':', '#3A')).join(':');

async function main() {
  const v = await req('GetVersion');
  console.log('obs', v.obsVersion, 'websocket', v.obsWebSocketVersion, 'platform', v.platformDescription);
  const kinds = (await req('GetInputKindList')).inputKinds;
  console.log('relevant kinds:', kinds.filter(k => /game_capture|monitor_capture|window_capture|wasapi/.test(k)).join(', '));

  // Remove any auto-created global desktop/mic sources so nothing is double-captured.
  // Global channel sources (Desktop Audio, Mic/Aux) cannot be removed via RemoveInput, so
  // they are muted and unrouted from every track instead.
  const none = Object.fromEntries([1, 2, 3, 4, 5, 6].map(n => [String(n), false]));
  for (const i of (await req('GetInputList')).inputs) {
    console.log('pre-existing input (muting + unrouting):', i.inputName, i.inputKind);
    await req('SetInputMute', { inputName: i.inputName, inputMuted: true });
    await req('SetInputAudioTracks', { inputName: i.inputName, inputAudioTracks: none });
  }

  await req('CreateInput', { sceneName: 'Scene', inputName: 'Display', inputKind: 'monitor_capture', inputSettings: { method: 2, capture_cursor: true } });
  const track = on => Object.fromEntries([1, 2, 3, 4, 5, 6].map(n => [String(n), on.includes(n)]));
  const apps = [['Game', 'GameTone', [1, 2]], ['VoiceChat', 'ChatTone', [1, 4]]];
  for (const [name, title, tracks] of apps) {
    await req('CreateInput', {
      sceneName: 'Scene', inputName: name, inputKind: 'wasapi_process_output_capture',
      inputSettings: { window: win(title, 'SDL_app', 'ffplay.exe'), priority: 1 },
    });
    await req('SetInputAudioTracks', { inputName: name, inputAudioTracks: track(tracks) });
  }
  console.log('inputs now:', (await req('GetInputList')).inputs.map(i => `${i.inputName}(${i.inputKind})`).join(', '));

  await req('StartReplayBuffer');
  console.log(`replay buffer started; waiting ${secs}s`);
  await sleep(secs * 1000);
  const stats = await req('GetStats');
  await req('SaveReplayBuffer');
  await sleep(4000);
  const saved = await req('GetLastReplayBufferReplay');
  await req('StopReplayBuffer');
  console.log('saved', saved.savedReplayPath);
  console.log('stats', JSON.stringify({ cpu: stats.cpuUsage, memMB: stats.memoryUsage, renderSkipped: stats.renderSkippedFrames, outputSkipped: stats.outputSkippedFrames, activeFps: stats.activeFps }));
  ws.close();
  process.exit(0);
}
