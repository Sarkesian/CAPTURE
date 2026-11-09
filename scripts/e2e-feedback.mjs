// Real worker acceptance: input latency, asynchronous screenshots, replay/session saves,
// rapid inputs, failure propagation and clean shutdown. No UI automation.
import { spawn } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import readline from 'node:readline';
import path from 'node:path';
const [worker, root] = process.argv.slice(2);
mkdirSync(root, { recursive: true });
const sound = process.argv.includes('--sound');
const markers = { sound, sound_volume: .001 };
const proc = spawn(worker, [], { stdio: ['pipe','pipe','pipe'], env: { ...process.env, ...(sound ? { CAPTURE_TRACE: path.join(root,'timing.csv') } : {}) } });
const events = [], results = [], sent = new Map();
let id=0, failed=0, diagnostics='';
proc.stderr.on('data', b => diagnostics += b);
readline.createInterface({input:proc.stdout}).on('line', l=>events.push({...JSON.parse(l), at:performance.now()}));
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const check=(ok,label)=>{results.push({ok,label});console.log(`${ok?'PASS':'FAIL'} ${label}`);if(!ok)failed++};
const wait=async pred=>{const end=Date.now()+30000;while(Date.now()<end){const v=events.find(pred);if(v)return v;await sleep(5)}throw Error('Timed out')};
const send=(cmd,args)=>{const re=++id;sent.set(re,performance.now());proc.stdin.write(JSON.stringify({v:1,id:re,cmd,...(args===undefined?{}:{args})})+'\n');return re};
const ack=async(re,action)=>{
  const event=await wait(m=>m.re===re&&m.event==='capture_triggered');
  const latency=event.at-sent.get(re);
  check(event.data.action===action&&latency<100,`${action} input acknowledged in ${latency.toFixed(1)} ms`);
  return event;
};
const completion=async re=>{
  const started=await wait(m=>m.re===re&&m.event==='save_started');
  return wait(m=>/save_finished|save_failed/.test(m.event)&&m.data.save_id===started.data.save_id);
};
try{
  await wait(m=>m.event==='hello');
  send('configure',{capture_root:root,replay_secs:5,target:{kind:'display',index:0},quality:'balanced',fps:30,capture_cursor:false,audio:{game_audio:false},markers,save_hotkey:'',hotkeys:{marker:'',overlay:'',session:'',screenshot:''},overlay:false,buffer_cap_bytes:4e9,min_free_bytes:1e9});
  send('set_replay',{enabled:true});
  await wait(m=>m.event==='state'&&m.data.status==='buffering');
  await sleep(3200);
  const saves=[];
  for(let i=0;i<4;i++){
    const shot=send('screenshot');const shotAck=await ack(shot,'screenshot');
    const state=send('get_state');
    const response=await wait(m=>m.re===state&&m.event==='state');
    check(response.at-sent.get(state)<100,`command loop responsive while PNG processes (${(response.at-sent.get(state)).toFixed(1)} ms)`);
    const replay=send('save_replay');await ack(replay,'replay');saves.push(replay);
    const done=await wait(m=>m.re===shot&&/screenshot_saved|error/.test(m.event));
    check(done.event==='screenshot_saved'&&done.at>shotAck.at,`screenshot ${i+1} saved asynchronously after acknowledgment (${(done.at-shotAck.at).toFixed(1)} ms)`);
    await sleep(100);
  }
  const start=send('start_session');await ack(start,'recording_start');await sleep(1500);
  const stop=send('stop_session');await ack(stop,'recording_stop');saves.push(stop);
  // Two simultaneous PNG tasks maximum; extra input must remain responsive and report failure.
  const burst=[send('screenshot'),send('screenshot'),send('screenshot'),send('screenshot')];
  for(const re of burst) await wait(m=>m.re===re&&/screenshot_saved|error/.test(m.event));
  check(burst.some(re=>events.some(m=>m.re===re&&m.event==='error')), 'rapid screenshot burst is bounded and excess work reports an error');
  for(const re of saves) check((await completion(re)).event==='save_finished',`capture ${re} finished in background`);
  check(events.filter(m=>m.event==='capture_triggered').every(m=>!events.some(x=>x.re===m.re&&/screenshot_saved/.test(x.event)&&x.at<m.at)), 'no success completion precedes input acknowledgment');
  check(saves.every(re=>events.filter(m=>m.re===re&&m.event==='capture_triggered').length===1),'one acknowledgment per replay/session action');
  // Force an actual PNG write error while leaving the replay buffer writable.
  mkdirSync(path.join(root,'Desktop'),{recursive:true});
  // All successful screenshots have finished. Rename is deliberately unnecessary: use a new
  // root with a file blocking its per-game directory and a separate buffer folder.
  const bad=path.join(root,'failure');mkdirSync(bad,{recursive:true});writeFileSync(path.join(bad,'Desktop'),'blocked');
  send('configure',{capture_root:bad,buffer_root:path.join(root,'failure-buffer'),replay_secs:5,target:{kind:'display',index:0},quality:'balanced',fps:30,capture_cursor:false,audio:{game_audio:false},markers,save_hotkey:'',hotkeys:{marker:'',overlay:'',session:'',screenshot:''},overlay:false,buffer_cap_bytes:4e9,min_free_bytes:1e9});
  await sleep(1000);
  const badShot=send('screenshot');await ack(badShot,'screenshot');
  const error=await wait(m=>m.re===badShot&&m.event==='error');
  check(error.data.message.includes('Screenshot not saved'),'filesystem failure is clearly reported after input acceptance');
  check(!events.some(m=>m.re===badShot&&m.event==='screenshot_saved'),'failed screenshot never reports saved');
}catch(e){check(false,e.stack)}finally{
  send('shutdown');await Promise.race([new Promise(r=>proc.once('exit',r)),sleep(10000)]);proc.kill();
  if(sound){
    const trace=readFileSync(path.join(root,'timing.csv'),'utf8').trim().split(/\r?\n/).slice(1).map(l=>l.split(','));
    const starts=trace.filter(r=>r[0]==='sound_start');
    const latency=starts.map(r=>+r[3]/10000);
    check(starts.length>=3&&!diagnostics.includes('feedback audio:'),`persistent WASAPI player rendered ${starts.length} cues without errors`);
    check(latency.every(ms=>ms<50),`sound submission latency ${latency.map(ms=>ms.toFixed(1)).join(', ')} ms`);
    let active=false,overlap=false;
    for(const r of trace){if(r[0]==='sound_start'){if(active)overlap=true;active=true}else if(r[0]==='sound_end')active=false}
    check(!overlap,'rapid actions never overlap audio cues');
  }
  writeFileSync(path.join(root,'feedback-results.json'),JSON.stringify({results,events},null,2));
  writeFileSync(path.join(root,'worker.log'),diagnostics);
}
process.exitCode=failed?1:0;
