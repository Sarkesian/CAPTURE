// Non-UI acceptance of an extracted candidate after Review.ps1 -InitializeOnly.
import fs from 'node:fs';import path from 'node:path';import crypto from 'node:crypto';import {spawn} from 'node:child_process';import readline from 'node:readline';import assert from 'node:assert/strict';
const dir=path.resolve(process.argv[2]),manifest=JSON.parse(fs.readFileSync(path.join(dir,'manifest.json')));
assert.equal(manifest.version,'0.6.5-rc.1');
for(const f of manifest.files){const p=path.join(dir,f.path);assert.equal(fs.statSync(p).size,f.size,f.path);assert.equal(crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex'),f.sha256,f.path);}
const settings=JSON.parse(fs.readFileSync(path.join(dir,'Data/Settings/settings.json'))),root=path.join(dir,'ReviewCaptures');
assert.equal(settings.capture_root,root);assert.equal(settings.replay_enabled,false);assert.equal(settings.target.kind,'none');
const worker=spawn(path.join(dir,'Runtime/capture-worker.exe'),[],{windowsHide:true,stdio:['pipe','pipe','pipe']});
const events=[];let serial=0,log='';worker.stderr.on('data',d=>log+=d);readline.createInterface({input:worker.stdout}).on('line',s=>events.push(JSON.parse(s)));
const send=(cmd,args)=>{const id=++serial;worker.stdin.write(JSON.stringify({v:1,id,cmd,...(args?{args}:{})})+'\n');return id;};
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function wait(test){const deadline=Date.now()+20000;while(Date.now()<deadline){const e=events.find(test);if(e)return e;await sleep(20);}throw Error('Worker timeout: '+log);}
try{
 await wait(e=>e.event==='hello');
 send('configure',{capture_root:root,replay_secs:120,history_secs:21600,target:{kind:'none'},quality:'high',fps:60,capture_cursor:false,audio:{game_audio:false},save_hotkey:'',hotkeys:{marker:'',overlay:'',session:'',screenshot:'',mic_mute:''},overlay:false,buffer_cap_bytes:20e9,min_free_bytes:1e9});
 let history;
 for(let n=0;n<100;n++){const id=send('history');history=(await wait(e=>e.re===id&&e.event==='history_ready')).data;if(history.sessions.length===5&&!history.reason?.includes('Recovering'))break;await sleep(100);}
 assert.equal(history.sessions.length,5);const session=history.sessions.find(s=>s.events.length===109);assert.ok(session);assert.ok(session.intervals.at(-1)[1]>21600);
 assert.ok(session.events.some(e=>e.kind==='marker'&&e.t===7206));
 const screenshots=history.sessions.flatMap(s=>s.events.filter(e=>e.kind==='screenshot'));assert.ok(screenshots.length>30);assert.ok(screenshots.every(e=>fs.existsSync(e.path)&&e.path.startsWith(root)));
 const started=performance.now(),id=send('history_review',{session:session.id,at:7206});const review=await wait(e=>e.re===id&&['review_ready','review_failed'].includes(e.event));assert.equal(review.event,'review_ready');assert.ok(fs.existsSync(review.data.path));const previewMs=performance.now()-started;
 send('release_review',{path:review.data.path});const save=send('history_save',{session:session.id,start:7203,end:7207});const done=await wait(e=>e.re===save&&['save_finished','save_failed'].includes(e.event));assert.equal(done.event,'save_finished');assert.ok(fs.existsSync(done.data.path));assert.ok(done.data.path.startsWith(root));
 const result={passed:true,version:manifest.version,programFilesVerified:manifest.files.length,sessions:history.sessions.length,sixHourEvents:session.events.length,screenshotAssociations:screenshots.length,previewMs,savedClip:done.data.path};
 fs.writeFileSync(path.join(dir,'package-verification.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result,null,2));
}finally{send('shutdown');await Promise.race([new Promise(r=>worker.once('exit',r)),sleep(5000)]);if(worker.exitCode===null)worker.kill();}
