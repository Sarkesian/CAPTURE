// Small, genuine encoded windows at widely separated media timestamps. Gaps are intentional.
// This fixture is sparse six-hour history, not a claim of six hours of live capture.
import fs from 'node:fs';import path from 'node:path';import {spawnSync} from 'node:child_process';import assert from 'node:assert/strict';
const repo=path.resolve(import.meta.dirname,'..'),root=path.join(repo,'test-output','native-replay-media');
assert.ok(!fs.existsSync(root),'Fixture already exists; use the existing disposable media or a fresh worktree');
fs.mkdirSync(root,{recursive:true});const buffer=path.join(root,'Background Recording');fs.mkdirSync(buffer);
const ff=path.join(repo,'third_party/ffmpeg/bin/ffmpeg.exe');
const run=args=>{const r=spawnSync(ff,['-v','error',...args],{windowsHide:true,encoding:'utf8'});assert.equal(r.status,0,r.stderr);};
for(const [i,duration] of [120,1800,7200,14400,21600].entries()){
 const game=i<2?'Hollow Knight':'The Witcher 3',id=`2026-11-10_12-00-0${i}_${i+1}`,dir=path.join(buffer,id);fs.mkdirSync(dir);
 fs.writeFileSync(path.join(dir,'session.json'),JSON.stringify({capture_session:1,game,started_unix_ms:1794337200000+i*600000,audio_tracks:['Preview Mix'],video:{encoder:'h264_nvenc',width:640,height:360,fps:30},segment_container:'mpegts'}));
 const offsets=i===4?[0,7200,14390,duration-14]:[0,duration-14];
 for(const [j,offset] of offsets.entries())run(['-f','lavfi','-i',`testsrc2=size=640x360:rate=30`,'-f','lavfi','-i','sine=frequency=440:sample_rate=48000','-t','14','-c:v','h264_nvenc','-preset','p1','-g','30','-bf','0','-c:a','aac','-b:a','192k','-output_ts_offset',String(offset),'-f','segment','-segment_time','2','-segment_format','mpegts','-segment_start_number',String(j*7),path.join(dir,'seg_%06d.ts')]);
 const shotDir=path.join(root,game,'Screenshots');fs.mkdirSync(shotDir,{recursive:true});const shot=path.join(shotDir,`fixture-${i}.png`);
 run(['-f','lavfi','-i','testsrc2=size=640x360:rate=1','-frames:v','1',shot]);
 const events=[{id:'expired',kind:'marker',t:.2,end:null,label:'Expired opening moment',path:null,marker:null}];
 for(const offset of offsets){events.push({id:'marker:'+offset,kind:'marker',t:offset+6,end:null,label:offset>20000?'Final victory':`Highlight at ${offset+6}s`,path:null,marker:null});events.push({id:'shot:'+offset,kind:'screenshot',t:offset+7,end:null,label:'Screenshot near highlight',path:shot,marker:null});}
 if(i===4)for(let n=0;n<100;n++)events.push({id:'dense:'+n,kind:n%3===0?'screenshot':'marker',t:7206+n*.01,end:null,label:`Battle moment ${n+1}`,path:n%3===0?shot:null,marker:null});
 fs.writeFileSync(path.join(dir,'events.jsonl'),events.map(e=>JSON.stringify(e)).join('\n')+'\n');
}
console.log(root);
