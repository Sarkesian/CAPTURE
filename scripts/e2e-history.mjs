// Disposable prerecorded fixtures only: never starts capture or touches the installed app.
import {spawn,spawnSync} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
const repo=path.resolve(import.meta.dirname,'..');
const ffmpeg=path.join(repo,'third_party/ffmpeg/bin/ffmpeg.exe');
const ffprobe=path.join(repo,'third_party/ffmpeg/bin/ffprobe.exe');
const root=path.join(repo,'test-output',`indexed-e2e-${Date.now()}`);
fs.mkdirSync(root,{recursive:true});
const buffer=path.join(root,'Background Recording');fs.mkdirSync(buffer);
const run=(exe,args)=>{const r=spawnSync(exe,args,{encoding:'utf8',maxBuffer:16*1024*1024,windowsHide:true});assert.equal(r.status,0,r.stderr);return r.stdout;};
const probe=p=>JSON.parse(run(ffprobe,['-v','error','-show_format','-show_streams','-of','json',p]));
for(const [i,game] of ['Fixture A','Fixture B'].entries()){
  const dir=path.join(buffer,`2026-11-09_10-00-0${i}_${i+1}`);fs.mkdirSync(dir);
  fs.writeFileSync(path.join(dir,'session.json'),JSON.stringify({capture_session:1,game,audio_tracks:['Preview Mix'],video:{encoder:'h264_nvenc',width:320,height:180,fps:30},segment_container:'mpegts'}));
  run(ffmpeg,['-hide_banner','-loglevel','error','-f','lavfi','-i','testsrc2=size=320x180:rate=30','-f','lavfi','-i','sine=frequency=440:sample_rate=48000','-t','14','-c:v','h264_nvenc','-preset','p1','-g','30','-bf','0','-c:a','aac','-b:a','192k','-output_ts_offset',String(i*14400),'-f','segment','-segment_time','2','-segment_format','mpegts',path.join(dir,'seg_%06d.ts')]);
  fs.writeFileSync(path.join(dir,'markers.json'),JSON.stringify([{id:1,t:i*14400+6,at:'10:00:06',label:`Moment ${game}`,footage:'buffered',note:null,saved_in:[]}]));
  if(i===1){
    const shotDir=path.join(root,game,'Screenshots');fs.mkdirSync(shotDir,{recursive:true});const png=path.join(shotDir,'fixture.png');fs.writeFileSync(png,Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jN1sAAAAASUVORK5CYII=','base64'));
    fs.writeFileSync(path.join(dir,'events.jsonl'),JSON.stringify({id:'screenshot:fixture',kind:'screenshot',t:14406,end:null,label:'Fixture screenshot',path:png,marker:null})+'\n');
  }
}
const messages=[];let serial=0;let stderr='';
const launch=()=>{const p=spawn(path.join(repo,'target/debug/capture-worker.exe'),[],{env:{...process.env,PATH:path.dirname(ffmpeg)+';'+process.env.PATH,CAPTURE_IN_GAME_OVERLAY:'0'},stdio:['pipe','pipe','pipe'],windowsHide:true});p.stderr.on('data',d=>stderr+=d);readline.createInterface({input:p.stdout}).on('line',s=>messages.push(JSON.parse(s)));return p;};
let worker=launch();
const send=(cmd,args)=>{const id=++serial;worker.stdin.write(JSON.stringify({v:1,id,cmd,...(args===undefined?{}:{args})})+'\n');return id;};
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function wait(pred,timeout=20000){const end=Date.now()+timeout;while(Date.now()<end){const m=messages.find(pred);if(m)return m;await sleep(20);}throw Error('timeout\n'+stderr);}
const metrics={root};
try{
  await wait(m=>m.event==='hello');
  send('configure',{capture_root:root,replay_secs:120,history_secs:14400,target:{kind:'none'},quality:'high',fps:60,capture_cursor:false,audio:{game_audio:false},save_hotkey:'',hotkeys:{marker:'',overlay:'',session:'',screenshot:'',mic_mute:''},overlay:false,buffer_cap_bytes:20e9,min_free_bytes:1e9});
  let history;
  for(let n=0;n<100;n++){const id=send('history');history=(await wait(m=>m.re===id&&m.event==='history_ready')).data;if(history.sessions.length===2 && !history.reason?.includes('Recovering'))break;await sleep(100);}
  assert.equal(history.sessions.length,2);assert.ok(history.retained_secs>27);
  const s=history.sessions.find(s=>s.game==='Fixture B');assert.ok(s.intervals[0][0]>14000);assert.ok(s.events.some(e=>e.label==='Moment Fixture B'));assert.ok(s.events.some(e=>e.kind==='screenshot'&&e.t===14406));
  const start=s.intervals[0][0]+4,end=start+4;
  let t=performance.now();const rid=send('history_review',{session:s.id,at:start});const review=await wait(m=>m.re===rid&&(m.event==='review_ready'||m.event==='review_failed'));assert.equal(review.event,'review_ready',JSON.stringify(review));metrics.seek_ms=performance.now()-t;metrics.preview_bytes=fs.statSync(review.data.path).size;assert.ok(probe(review.data.path).format.duration<33);
  const paths=[];
  for(let n=0;n<2;n++){
    t=performance.now();const id=send('history_save',{session:s.id,start,end});const accepted=await wait(m=>m.re===id&&m.event==='save_started');metrics[`accept_${n}_ms`]=performance.now()-t;
    const fin=await wait(m=>m.re===id&&(m.event==='save_finished'||m.event==='save_failed'));assert.equal(fin.event,'save_finished',JSON.stringify(fin));metrics[`save_${n}_ms`]=performance.now()-t;assert.ok(fs.existsSync(fin.data.path));assert.ok(!fs.existsSync(fin.data.path+'.partial'));paths.push(fin.data.path);
    run(ffmpeg,['-v','error','-i',fin.data.path,'-f','null','-']);
    const after=messages.filter(m=>m.re===id);assert.equal(after.filter(m=>m.event==='capture_triggered').length,1);assert.equal(accepted.data.requested_secs,4);
  }
  const hashes=p=>run(ffmpeg,['-v','error','-i',p,'-map','0:v','-c:v','copy','-bsf:v','h264_mp4toannexb','-f','framemd5','-']).split('\n').filter(l=>l&&!l.startsWith('#')).map(l=>l.split(',').at(-1).trim());
  assert.deepEqual(hashes(paths[0]),hashes(paths[1]));metrics.identical_packet_hashes=hashes(paths[0]).length;
  const sourceDir=path.join(buffer,s.id);
  // MP4 length prefixes / SPS insertion change packet hashes at keyframes. Compare original
  // encoded VCL NAL payloads, independently normalizing only Annex-B framing.
  const sliceHashes=p=>{
    const r=spawnSync(ffmpeg,['-v','error','-i',p,'-map','0:v','-c:v','copy','-bsf:v','h264_mp4toannexb','-f','h264','pipe:1'],{maxBuffer:16*1024*1024,windowsHide:true});assert.equal(r.status,0,String(r.stderr));
    const b=r.stdout,starts=[];for(let i=0;i<b.length-3;i++){if(b[i]===0&&b[i+1]===0){const n=b[i+2]===1?3:b[i+2]===0&&b[i+3]===1?4:0;if(n){starts.push([i,i+n]);i+=n-1;}}}
    return starts.map(([,a],i)=>b.subarray(a,starts[i+1]?.[0]??b.length)).filter(n=>[1,5].includes(n[0]&31)).map(n=>crypto.createHash('sha256').update(n).digest('hex'));
  };
  const originalHashes=fs.readdirSync(sourceDir).filter(n=>n.endsWith('.ts')).sort().flatMap(n=>sliceHashes(path.join(sourceDir,n)));
  const savedHashes=sliceHashes(paths[0]),origin=originalHashes.indexOf(savedHashes[0]);assert.ok(origin>=0,'Saved keyframe slice is an original encoded slice');
  assert.deepEqual(savedHashes,originalHashes.slice(origin,origin+savedHashes.length));metrics.original_video_slices_verified=savedHashes.length;
  const damaged=path.join(sourceDir,'seg_000003.ts'),backup=damaged+'.fixture-backup';fs.copyFileSync(damaged,backup);fs.writeFileSync(damaged,'deliberately corrupt disposable fixture');
  const bad=send('history_save',{session:s.id,start,end});const failure=await wait(m=>m.re===bad&&(m.event==='save_failed'||m.event==='save_finished'));assert.equal(failure.event,'save_failed');assert.ok(fs.existsSync(backup));assert.ok(fs.existsSync(path.join(sourceDir,'seg_000002.ts')));fs.copyFileSync(backup,damaged);
  const retry=send('history_save',{session:s.id,start,end});assert.equal((await wait(m=>m.re===retry&&(m.event==='save_failed'||m.event==='save_finished'))).event,'save_finished');metrics.failed_save_retry_passed=true;
  const secondReview=send('history_review',{session:s.id,at:start});const second=await wait(m=>m.re===secondReview&&m.event==='review_ready');assert.equal(second.data.path,review.data.path,'Playback windows reused');
  const stateId=send('history');const state=(await wait(m=>m.re===stateId&&m.event==='history_ready')).data;assert.ok(state.preview_bytes>0&&state.preview_bytes<256*1024*1024);
  send('release_review',{path:review.data.path});await sleep(100);assert.ok(fs.existsSync(review.data.path),'Other playback owner still holds the file');
  send('release_review',{path:second.data.path});await sleep(350);assert.ok(!fs.existsSync(review.data.path));
  send('clear_temp_cache');await sleep(150);assert.ok(paths.every(p=>fs.existsSync(p)));
  send('shutdown');await new Promise(r=>worker.on('exit',r));
  messages.length=0;worker=launch();await wait(m=>m.event==='hello');
  send('configure',{capture_root:root,replay_secs:120,history_secs:14400,target:{kind:'none'},quality:'high',fps:60,capture_cursor:false,audio:{game_audio:false},save_hotkey:'',hotkeys:{marker:'',overlay:'',session:'',screenshot:'',mic_mute:''},overlay:false,buffer_cap_bytes:20e9,min_free_bytes:1e9});
  const restarted=send('history');const persisted=(await wait(m=>m.re===restarted&&m.event==='history_ready')).data;assert.equal(persisted.sessions.length,2);const restored=persisted.sessions.find(x=>x.id===s.id);assert.ok(restored.events.some(e=>e.kind==='screenshot'&&e.t===14406));assert.ok(restored.events.some(e=>e.kind==='marker'));assert.ok(paths.every(p=>fs.existsSync(p)));metrics.restart_persistence_passed=true;
  send('shutdown');await new Promise(r=>worker.on('exit',r));
  metrics.passed=true;fs.writeFileSync(path.join(root,'results.json'),JSON.stringify(metrics,null,2));console.log(JSON.stringify(metrics,null,2));
}finally{if(worker.exitCode===null){worker.stdin.end();await new Promise(r=>{worker.once('exit',r);setTimeout(()=>{worker.kill();r();},5000).unref();});}}
