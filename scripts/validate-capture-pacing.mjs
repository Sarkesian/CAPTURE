import {spawn,execFileSync} from 'node:child_process';import fs from 'node:fs';import path from 'node:path';import readline from 'node:readline';import assert from 'node:assert/strict';
const repo=path.resolve(import.meta.dirname,'..'),root=path.join(repo,'test-output',`pacing-${Date.now()}`);fs.mkdirSync(root,{recursive:true});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));const bin=path.join(repo,'third_party/ffmpeg/bin');const results=[];
for(const legacy of [true,false]){
 const dir=path.join(root,legacy?'default':'uncapped');fs.mkdirSync(dir);
 const trace=path.join(dir,'trace.csv'),events=[],presents=[];let id=0,log='';
 const game=spawn(path.join(repo,'target/debug/hook-testapp.exe'),['--resolution-test','--flip','--hdr','--rate','60','--secs','45','--title',`Pacing-${legacy}-${process.pid}`],{windowsHide:true,stdio:['pipe','pipe','pipe']});
 const worker=spawn(path.join(repo,'target/debug/capture-worker.exe'),[],{windowsHide:true,env:{...process.env,PATH:bin+';'+process.env.PATH,CAPTURE_IN_GAME_OVERLAY:'0',CAPTURE_TRACE:trace,CAPTURE_WGC_DEFAULT_PACING:legacy?'1':'0'},stdio:['pipe','pipe','pipe']});
 readline.createInterface({input:worker.stdout}).on('line',l=>events.push(JSON.parse(l)));worker.stderr.on('data',d=>log+=d);game.stderr.on('data',d=>log+=d);readline.createInterface({input:game.stdout}).on('line',l=>{presents.push(l);});
 const send=(cmd,args)=>{const r=++id;worker.stdin.write(JSON.stringify({v:1,id:r,cmd,...(args===undefined?{}:{args})})+'\n');return r;};
 async function wait(pred,ms=20000){const end=Date.now()+ms;while(Date.now()<end){const m=events.find(pred);if(m)return m;await sleep(10);}throw Error('timeout '+log);}
 try{
  await wait(m=>m.event==='hello');await sleep(500);game.stdin.write('window 1920 1080\n');await sleep(400);
  const t=send('list_targets');const w=(await wait(m=>m.re===t&&m.event==='targets')).data.windows.find(w=>w.pid===game.pid);assert.ok(w);
  send('configure',{capture_root:dir,replay_secs:8,target:{kind:'window',hwnd:w.hwnd,pid:w.pid,exe:w.exe,title:w.title},quality:'high',recording_output:'original_hdr',fps:60,capture_cursor:false,audio:{game_audio:false},save_hotkey:'',hotkeys:{marker:'',overlay:'',session:'',screenshot:'',mic_mute:''},overlay:false,buffer_cap_bytes:5e9,min_free_bytes:1e9});send('set_replay',{enabled:true});await wait(m=>m.event==='state'&&m.data.status==='buffering');await sleep(10000);
  const latest=events.filter(m=>m.event==='state').at(-1).data;const r=send('save_replay');const accept=await wait(m=>m.re===r&&m.event==='save_started');const done=await wait(m=>m.event==='save_finished'&&m.data.save_id===accept.data.save_id);
  send('shutdown');await new Promise(r=>worker.once('exit',r));
  const lines=fs.readFileSync(trace,'utf8').trim().split('\n').slice(1).map(l=>{const [kind,...n]=l.split(',');return {kind,n:n.map(Number)};});const source=lines.filter(r=>r.kind==='src').map(r=>r.n[1]);const intervals=source.slice(1).map((v,i)=>(v-source[i])/10000).sort((a,b)=>a-b);
  const raw=execFileSync(path.join(bin,'ffmpeg.exe'),['-v','error','-i',done.data.path,'-vf','crop=128:16:0:1064','-pix_fmt','yuv420p10le','-f','rawvideo','pipe:1'],{maxBuffer:16e6,windowsHide:true});const ids=[];
  for(let f=0;f<raw.length/6144;f++){let code=0;for(let b=0;b<16;b++)if(raw.readUInt16LE(f*6144+(8*128+b*8+4)*2)>200)code|=1<<b;ids.push(code);}
  const repeats=ids.slice(1).filter((v,i)=>v===ids[i]).length;const skipped=ids.slice(1).reduce((s,v,i)=>s+Math.max(0,v-ids[i]-1),0);
  const result={legacy,source_fps:(source.length-1)/((source.at(-1)-source[0])/1e7),arrival_ms:{p50:intervals[Math.floor(intervals.length*.5)],p95:intervals[Math.floor(intervals.length*.95)]},encoded:latest.frames_encoded,missed_ticks:latest.frames_dropped,repeated_generations:latest.frames_repeated,unselected_generations:latest.source_frames_unselected,decoded_frames:ids.length,duplicate_picture_ids:repeats,skipped_picture_ids:skipped,first_id:ids[0],last_id:ids.at(-1),path:done.data.path,presents,log};results.push(result);fs.writeFileSync(path.join(dir,'frame-ids.json'),JSON.stringify(ids));console.log(JSON.stringify(result));
 }finally{if(worker.exitCode===null){send('shutdown');await Promise.race([new Promise(r=>worker.once('exit',r)),sleep(1000)]);worker.kill();}game.kill();fs.writeFileSync(path.join(dir,'worker.log'),log);}
}
fs.writeFileSync(path.join(root,'results.json'),JSON.stringify(results,null,2));console.log('Results: '+root);
