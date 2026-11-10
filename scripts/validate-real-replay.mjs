// Launch a separately identified compiled Capture Point against disposable history.
// This is the actual packaged WebView2/Replay, with no mocked IPC or replacement viewer.
import fs from 'node:fs';import path from 'node:path';import {spawn} from 'node:child_process';
import assert from 'node:assert/strict';
const repo=path.resolve(import.meta.dirname,'..');
const root=path.join(repo,'test-output',`real-replay-${Date.now()}`);fs.mkdirSync(root,{recursive:true});
const payload='G:/Documents/Codex/2026-11-09/files-pasted-by-the-user-capture-2/outputs/CAPTURE-HDR-Preview';
const appDir=path.join(root,'App');fs.cpSync(payload,appDir,{recursive:true});
fs.copyFileSync(path.join(repo,'target/release/CAPTURE.exe'),path.join(appDir,'CAPTURE.exe'));
fs.copyFileSync(path.join(repo,'target/debug/capture-worker.exe'),path.join(appDir,'Runtime/capture-worker.exe'));
const records=path.join(root,'Recordings');const background=path.join(records,'Background Recording');fs.mkdirSync(background,{recursive:true});
const inputRoots=[
 path.join(repo,'test-output/native-quality-1794292317771/Background Recording'),
 path.join(repo,'test-output/indexed-e2e-1794291549469/Background Recording'),
];
for(const source of inputRoots)for(const e of fs.readdirSync(source,{withFileTypes:true}))if(e.isDirectory())fs.cpSync(path.join(source,e.name),path.join(background,e.name),{recursive:true});
fs.mkdirSync(path.join(appDir,'Data/Settings'),{recursive:true});
fs.writeFileSync(path.join(appDir,'Data/Settings/settings.json'),JSON.stringify({version:4,capture_root:records,target:{kind:'none'},replay_enabled:false,first_run_done:true,save_hotkey:'',notify_on_save:false,hotkeys:{marker:'',overlay:'',session:'',screenshot:'',mic_mute:''},recording_output:'original_hdr'}));
const port=9453;
const child=spawn(path.join(appDir,'CAPTURE.exe'),[],{cwd:appDir,windowsHide:true,env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`}});
let ws,serial=0;const pending=new Map(),media=[],errors=[];
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const result={root,appDir,media,errors,tests:[]};
try {
 let pages;
 for(let i=0;i<100;i++){try{pages=await(await fetch(`http://127.0.0.1:${port}/json`)).json();if(pages.some(p=>p.type==='page'))break;}catch{}await sleep(100);}
 assert.ok(pages,'Validation WebView2 did not start');const page=pages.find(p=>p.type==='page'&&!p.url.includes('overlay'));
 ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){pending.get(m.id)?.(m);pending.delete(m.id);}else if(m.method?.startsWith('Media.'))media.push(m);else if(m.method==='Runtime.exceptionThrown')errors.push(m.params);});
 const cdp=(method,params={})=>new Promise(r=>{pending.set(++serial,r);ws.send(JSON.stringify({id:serial,method,params}));});
 const js=async expression=>{const r=await cdp('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(r.result?.exceptionDetails)throw Error(JSON.stringify(r.result.exceptionDetails));return r.result?.result?.value;};
 await cdp('Runtime.enable');await cdp('Media.enable');result.browser=await cdp('Browser.getVersion');result.system=await cdp('SystemInfo.getInfo');
 for(let i=0;i<100&&!await js('!!window.__TAURI_INTERNALS__ && !!document.querySelector("button")');i++)await sleep(100);
 result.bootstrap=await js('window.__TAURI_INTERNALS__.invoke("bootstrap")');
 assert.equal(path.resolve(result.bootstrap.settings.capture_root),path.resolve(records));
 assert.ok(await js('(()=>{const b=[...document.querySelectorAll("button")].find(b=>b.textContent.trim()==="Replay");if(b){b.click();return true;}return document.body.innerText.includes("All gameplay");})()'));
 for(let i=0;i<100&&!await js('!!document.querySelector("video")');i++)await sleep(100);
 result.initial=await js('({body:document.body.innerText,selects:[...document.querySelectorAll("select")].map(s=>({label:s.getAttribute("aria-label"),options:[...s.options].map(o=>({value:o.value,text:o.text}))}))})');
 const choices=await js(`Array.from(document.querySelector('select[aria-label="Game and recording"]').options,o=>({value:o.value,text:o.text}))`);
 for(const choice of choices){
   await js(`(()=>{const s=document.querySelector('select[aria-label="Game and recording"]');s.value=${JSON.stringify(choice.value)};s.dispatchEvent(new Event('change',{bubbles:true}));})()`);
   await sleep(700);
   for(let i=0;i<50&&!await js('document.querySelector("video")?.readyState>=2 || document.body.innerText.includes("cannot decode")');i++)await sleep(100);
   const info=await js('(()=>{const v=document.querySelector("video");return {src:v?.currentSrc,width:v?.videoWidth,height:v?.videoHeight,duration:v?.duration,ready:v?.readyState,error:v?.error?.message,body:document.body.innerText};})()');
   const test={choice,...info};
   if(info.width>0){
     await js('document.querySelector("video").muted=true;document.querySelector("video").play()');await sleep(700);
     test.playing=await js('(()=>{const v=document.querySelector("video");return {t:v.currentTime,quality:v.getVideoPlaybackQuality()};})()');
     await js('(()=>{const v=document.querySelector("video");v.pause();v.currentTime=Math.min(2,v.duration/2);})()');await sleep(300);
     test.seek=await js('(()=>{const v=document.querySelector("video");return {t:v.currentTime,width:v.videoWidth,ready:v.readyState};})()');
   }
   result.tests.push(test);console.log(JSON.stringify({choice:choice.text,width:test.width,height:test.height,playing:test.playing,seek:test.seek,error:test.error}));
 }
 result.capabilities=await js('(()=>{const v=document.createElement("video");return {hevc:v.canPlayType(\'video/mp4; codecs="hvc1.2.4.L153.B0"\'),avc:v.canPlayType(\'video/mp4; codecs="avc1.640033"\')};})()');
 const shot=await cdp('Page.captureScreenshot',{format:'png'});fs.writeFileSync(path.join(root,'replay.png'),Buffer.from(shot.result.data,'base64'));
 result.completed=true;await js('window.__TAURI_INTERNALS__.invoke("quit_app")');
} finally {
 ws?.close();await Promise.race([new Promise(r=>child.once('exit',r)),sleep(3000)]);if(child.exitCode===null)child.kill();
 fs.writeFileSync(path.join(root,'results.json'),JSON.stringify(result,null,2));console.log('Results: '+root);
}
