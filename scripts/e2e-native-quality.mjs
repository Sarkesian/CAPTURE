// Real WGC/DX11 fixture: native-resolution epochs, MP4, screenshot, timing and manual saves.
// Uses only a disposable test window and test-output; does not change display settings.
import {spawn,execFileSync} from 'node:child_process';
import fs from 'node:fs';import path from 'node:path';import readline from 'node:readline';import assert from 'node:assert/strict';
const repo=path.resolve(import.meta.dirname,'..'),root=path.join(repo,'test-output',`native-quality-${Date.now()}`);
fs.mkdirSync(root,{recursive:true});const bin=path.join(repo,'third_party/ffmpeg/bin');
const env={...process.env,PATH:bin+';'+process.env.PATH,CAPTURE_IN_GAME_OVERLAY:'0'};
const hdr=process.argv.includes('--hdr'),compatible=process.argv.includes('--compatible'),quality=process.argv.includes('--high')?'high':'maximum';
const game=spawn(path.join(repo,'target/debug/hook-testapp.exe'),['--resolution-test','--flip',...(hdr?['--hdr']:[]),'--secs','180','--title',`Native-quality-${process.pid}`],{windowsHide:true,stdio:['pipe','pipe','pipe']});
const worker=spawn(path.join(repo,'target/debug/capture-worker.exe'),[],{env,windowsHide:true,stdio:['pipe','pipe','pipe']});
const events=[];let log='',id=0;const results={root,hdr_source:hdr,compatible,quality,phases:[]};
game.stderr.on('data',d=>log+=d);worker.stderr.on('data',d=>log+=d);
readline.createInterface({input:game.stdout}).on('line',l=>log+=l+'\n');
readline.createInterface({input:worker.stdout}).on('line',l=>events.push(JSON.parse(l)));
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const send=(cmd,args)=>{const r=++id;worker.stdin.write(JSON.stringify({v:1,id:r,cmd,...(args===undefined?{}:{args})})+'\n');return r;};
async function wait(pred,timeout=20000){const end=Date.now()+timeout;while(Date.now()<end){const m=events.find(pred);if(m)return m;await sleep(20);}throw Error('Timeout\n'+log);}
const run=(exe,args)=>execFileSync(exe,args,{encoding:'utf8',windowsHide:true,maxBuffer:64e6});
const probe=p=>JSON.parse(run(path.join(bin,'ffprobe.exe'),['-v','error','-count_frames','-show_streams','-of','json',p])).streams[0];
async function save(cmd,args){const start=performance.now(),r=send(cmd,args);const accepted=await wait(m=>m.re===r&&['save_started','save_failed'].includes(m.event));assert.equal(accepted.event,'save_started',JSON.stringify(accepted));const m=await wait(m=>m.data?.save_id===accepted.data.save_id&&['save_finished','save_failed'].includes(m.event),30000);assert.equal(m.event,'save_finished',JSON.stringify(m));return {...m.data,latency_ms:performance.now()-start};}
try {
  await wait(m=>m.event==='hello');await sleep(1000);game.stdin.write('window 640 360\n');await sleep(500);
  const r=send('list_targets');const targets=(await wait(m=>m.re===r&&m.event==='targets')).data;const w=targets.windows.find(w=>w.pid===game.pid);assert.ok(w);
  results.displays=targets.displays;
  send('configure',{capture_root:root,replay_secs:3,history_secs:3600,target:{kind:'window',hwnd:w.hwnd,pid:w.pid,exe:w.exe,title:w.title},quality,recording_output:compatible?'compatible':'original_hdr',fps:60,capture_cursor:false,audio:{game_audio:false},save_hotkey:'',hotkeys:{marker:'',overlay:'',session:'',screenshot:'',mic_mute:''},overlay:false,buffer_cap_bytes:10e9,min_free_bytes:1e9});
  send('set_replay',{enabled:true});
  await wait(m=>m.event==='state'&&m.data.encoder);
  send('start_session');
  for(const [width,height] of [[1920,1080],[2560,1440],[3840,2160]]) {
    const since=events.length;game.stdin.write(`${width===3840?'borderless':'window'} ${width} ${height}\n`);
    await wait(m=>events.indexOf(m)>=since&&m.event==='state'&&m.data.encoder?.width===width&&m.data.encoder?.height===height,20000);
    await sleep(6500);
    const state=events.filter(m=>m.event==='state').at(-1).data;
    const clip=await save('save_replay');const stream=probe(clip.path);
    assert.equal(stream.width,width);assert.equal(stream.height,height);assert.equal(stream.r_frame_rate,'60/1');
    run(path.join(bin,'ffmpeg.exe'),['-v','error','-i',clip.path,'-f','null','-']);
    const packets=JSON.parse(run(path.join(bin,'ffprobe.exe'),['-v','error','-select_streams','v','-show_packets','-show_entries','packet=pts_time','-of','json',clip.path])).packets;
    const delta=packets.slice(1).map((p,i)=>Number(p.pts_time)-Number(packets[i].pts_time));
    results.phases.push({width,height,encoder:state.encoder.name,encoded:state.frames_encoded,dropped:state.frames_dropped,repeated:state.frames_repeated,without_source:state.frames_without_source,source_unselected:state.source_frames_unselected,capture_fps:state.capture_fps,clip_frames:stream.nb_read_frames,clip_seconds:stream.duration,min_delta:Math.min(...delta),max_delta:Math.max(...delta),save_ms:clip.latency_ms,path:clip.path});
    if(hdr&&!compatible) {
      assert.equal(stream.pix_fmt,'yuv420p10le');assert.equal(stream.color_transfer,'smpte2084');
      const raw=execFileSync(path.join(bin,'ffmpeg.exe'),['-v','error','-i',clip.path,'-frames:v','1','-pix_fmt','yuv420p10le','-f','rawvideo','pipe:1'],{windowsHide:true,maxBuffer:64e6});
      const pq=n=>{const x=(n/10000)**(2610/16384);return ((3424/4096+2413/128*x)/(1+2392/128*x))**(2523/32);};
      const measured=[[.05,.05,1000],[.25,.25,.8]].map(([x,y,nits])=>{const got=raw.readUInt16LE((Math.floor(y*height)*width+Math.floor(x*width))*2),expected=64+876*pq(nits);assert.ok(Math.abs(got-expected)<8,`WGC HDR luminance ${nits} nits: ${got} vs ${expected}`);return {nits,got,expected};});
      results.phases.at(-1).hdr_luminance=measured;
    }
    console.log(JSON.stringify(results.phases.at(-1)));
  }
  const shot=send('screenshot');assert.equal((await wait(m=>m.re===shot&&['screenshot_saved','error'].includes(m.event))).event,'screenshot_saved');
  const manual=await save('stop_session');assert.ok(fs.existsSync(manual.path));
  const h=send('history');results.history=(await wait(m=>m.re===h&&m.event==='history_ready')).data.sessions.map(s=>({id:s.id,intervals:s.intervals}));assert.ok(results.history.length>=4,'Resolution epochs retain earlier footage');
  results.manual_saves=events.filter(m=>m.event==='save_finished').map(m=>m.data.path);
  results.passed=true;
} finally {
  send('shutdown');await Promise.race([new Promise(r=>worker.once('exit',r)),sleep(5000)]);worker.kill();game.kill();
  fs.writeFileSync(path.join(root,'results.json'),JSON.stringify(results,null,2));fs.writeFileSync(path.join(root,'worker.log'),log);console.log('Results: '+root);
  fs.writeFileSync(path.join(root,'events.json'),JSON.stringify(events,null,2));
}
