// Waits for actual Ctrl+Alt+F12 screenshot and Ctrl+Alt+F10 replay hotkeys in a
// dedicated DX11 test window. Use Computer Use or press the keys yourself.
import {spawn} from 'node:child_process';
import {mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import readline from 'node:readline';
import path from 'node:path';
const [worker,root]=process.argv.slice(2);mkdirSync(root,{recursive:true});
const game=spawn(path.resolve(import.meta.dirname,'../target/release/hook-testapp.exe'),['--secs','120','--title','Capture feedback latency test'],{stdio:['ignore','pipe','inherit']});
let ready=false;readline.createInterface({input:game.stdout}).on('line',l=>{if(l.startsWith('ready'))ready=true});
const proc=spawn(worker,[],{stdio:['pipe','pipe','pipe'],env:{...process.env,CAPTURE_TRACE:path.join(root,'timing.csv'),CAPTURE_IN_GAME_OVERLAY:'0'}});
const events=[];let id=0,diagnostics='';proc.stderr.on('data',b=>diagnostics+=b);
readline.createInterface({input:proc.stdout}).on('line',l=>events.push(JSON.parse(l)));
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const wait=async pred=>{const end=Date.now()+100000;while(Date.now()<end){const x=pred();if(x)return x;await sleep(10)}throw Error('Timed out waiting for physical hotkeys')};
const send=(cmd,args)=>{const re=++id;proc.stdin.write(JSON.stringify({v:1,id:re,cmd,...(args===undefined?{}:{args})})+'\n');return re};
let error;
try{
 await wait(()=>ready);await wait(()=>events.find(m=>m.event==='hello'));const re=send('list_targets');
 const targets=await wait(()=>events.find(m=>m.re===re&&m.event==='targets'));const w=targets.data.windows.find(w=>w.pid===game.pid);
 send('configure',{capture_root:root,replay_secs:5,target:{kind:'window',hwnd:w.hwnd,pid:w.pid,exe:w.exe,title:w.title},quality:'balanced',fps:30,capture_cursor:false,audio:{game_audio:false},markers:{sound:false},save_hotkey:'Ctrl+Alt+F10',hotkeys:{marker:'',overlay:'',session:'',screenshot:'Ctrl+Alt+F12'},overlay:true,notices:{recording_started:false,saved:true,problems:true,marker:false},buffer_cap_bytes:4e9,min_free_bytes:1e9});
 send('set_replay',{enabled:true});await wait(()=>events.find(m=>m.event==='state'&&m.data.status==='buffering'));await sleep(3000);
 writeFileSync(path.join(root,'ready.json'),JSON.stringify({game:game.pid,worker:proc.pid}));console.log('READY: Ctrl+Alt+F12, then Ctrl+Alt+F10 in the test window');
 await wait(()=>events.find(m=>m.event==='capture_triggered'&&m.data.action==='screenshot'&&m.re===undefined));
 await wait(()=>events.find(m=>m.event==='capture_triggered'&&m.data.action==='replay'&&m.re===undefined));
 await wait(()=>events.find(m=>m.event==='save_finished'));await wait(()=>events.find(m=>m.event==='screenshot_saved'));
}catch(e){error=e.stack}finally{
 send('shutdown');await Promise.race([new Promise(r=>proc.once('exit',r)),sleep(10000)]);proc.kill();game.kill();
 const rows=readFileSync(path.join(root,'timing.csv'),'utf8').trim().split(/\r?\n/).slice(1).map(l=>l.split(','));
 const hotkeys=rows.filter(r=>r[0]==='hotkey');
 const timings=hotkeys.map(h=>{const feedback=rows.find(r=>r[0]==='feedback'&&+r[1]>=+h[1]);const paint=rows.find(r=>r[0]==='notice_paint'&&+r[1]>=+h[1]);return{action:+h[2],ack_ms:feedback?(+feedback[1]-h[1])/10000:null,paint_ms:paint?(+paint[1]-h[1])/10000:null}});
 const ok=!error&&timings.length>=2&&timings.every(t=>t.ack_ms!==null&&t.ack_ms<100&&t.paint_ms!==null&&t.paint_ms<100);
 const report={ok,error,timings,events};writeFileSync(path.join(root,'hotkey-results.json'),JSON.stringify(report,null,2));writeFileSync(path.join(root,'worker.log'),diagnostics);
 console.log(JSON.stringify({ok,error,timings},null,2));process.exitCode=ok?0:1;
}
