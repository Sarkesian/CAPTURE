// Isolated headless Edge + mocked IPC. Does not connect to the installed CAPTURE app.
import fs from 'node:fs';import path from 'node:path';import {spawn} from 'node:child_process';import assert from 'node:assert/strict';
import {createServer} from '../app/node_modules/vite/dist/node/index.js';
import {svelte} from '../app/node_modules/@sveltejs/vite-plugin-svelte/src/index.js';
const repo=path.resolve(import.meta.dirname,'..'),appRoot=path.join(repo,'app');
const folder=`test-output/history-ui-${Date.now()}`,dir=path.join(appRoot,folder);fs.mkdirSync(dir,{recursive:true});
const mediaRoot=fs.readdirSync(path.join(repo,'test-output')).filter(n=>n.startsWith('indexed-e2e-')&&fs.existsSync(path.join(repo,'test-output',n,'results.json'))).sort().at(-1);assert.ok(mediaRoot,'Run e2e-history first');
const result=JSON.parse(fs.readFileSync(path.join(repo,'test-output',mediaRoot,'results.json'),'utf8'));
const gameDir=path.join(result.root,'Fixture B','Clips');const clip=fs.readdirSync(gameDir).find(n=>n.endsWith('.mp4'));fs.copyFileSync(path.join(gameDir,clip),path.join(dir,'clip.mp4'));
fs.writeFileSync(path.join(dir,'shot.svg'),'<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90"><rect width="160" height="90" fill="#589"/><text x="15" y="45" fill="white">Fixture screenshot</text></svg>');
fs.writeFileSync(path.join(dir,'index.html'),'<html><body><div id="app" style="height:100vh;padding:20px;display:flex;flex-direction:column;box-sizing:border-box"></div><script type="module" src="./main.ts"></script></body></html>');
fs.writeFileSync(path.join(dir,'main.ts'),`
import {mount} from 'svelte';import Panel from '../../src/overlay/HistoryPanel.svelte';import {app,init,historyReview,releaseReview} from '../../src/lib/store.svelte';import '../../src/app.css';
const callbacks=new Map();const listeners=new Map();let callback=0,serial=0;const calls=[];window.fixtureCalls=calls;window.fixtureApp=app;window.fixtureReview=historyReview;window.fixtureRelease=releaseReview;
const settings={version:3,capture_root:'fixture',replay_secs:120,history_secs:14400,markers:{protect_before_secs:30,protect_after_secs:15},per_game:[]};
const sessions=[{id:'a',game:'Fixture A',created_ms:Date.now()-200000,ended:true,intervals:[[0,60],[80,120]],events:[{id:'m1',kind:'marker',t:6,end:null,label:'First moment',path:null,marker:{id:1,t:6}}]},{id:'b',game:'Fixture B',created_ms:Date.now(),ended:true,intervals:[[0,14400]],events:[{id:'m2',kind:'marker',t:14380,end:null,label:'Late moment',path:null,marker:{id:2,t:14380}},{id:'s1',kind:'screenshot',t:14382,end:null,label:'Screenshot',path:'/${folder}/shot.svg',marker:null}]}];
const emit=(payload)=>{const handler=listeners.get('engine');if(handler)callbacks.get(handler)?.({event:'engine',payload});};
window.__TAURI_INTERNALS__={transformCallback(fn){callbacks.set(++callback,fn);return callback;},unregisterCallback(id){callbacks.delete(id);},convertFileSrc(p){return p;},async invoke(cmd,args){
 if(cmd==='bootstrap')return {settings,data_dir:'fixture',last_state:null,engine_down:null,app:null,path_issues:[],notices:[]};
 if(cmd==='plugin:event|listen'){listeners.set(args.event,args.handler);return ++serial;}
 if(cmd==='list_clips'||cmd==='check_paths')return [];
 if(cmd==='engine_command'){
  const id=++serial,c=args.command;calls.push(c);
  const reply=(event,data)=>setTimeout(()=>emit({v:1,re:id,event,data}),10);
  if(c.cmd==='history')reply('history_ready',{sessions,bytes:1e8,preview_bytes:0,retained_secs:14500,target_secs:14400,reason:null});
  if(c.cmd==='history_review'){const start=Math.floor(c.args.at/3)*3;reply('review_ready',{session:c.args.session,path:'/${folder}/clip.mp4',start,end:start+4,game:c.args.session==='a'?'Fixture A':'Fixture B',past:true,markers:[]});}
  if(c.cmd==='history_save'){reply('capture_triggered',{action:'replay',overlay_notice:false});setTimeout(()=>emit({v:1,re:id,event:'save_finished',data:{path:'permanent.mp4'}}),40);}
  return id;
 }
 return null;
}};
window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
await init();mount(Panel,{target:document.getElementById('app'),props:{onclose(){}}});window.fixtureReady=true;
`);
const server=await createServer({root:appRoot,configFile:false,plugins:[svelte()],server:{host:'127.0.0.1',port:4148,strictPort:true}});await server.listen();
const edge=spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',['--headless=new','--no-first-run','--disable-gpu','--remote-debugging-port=9448',`--user-data-dir=${path.join(repo,'test-output',path.basename(dir),'edge-profile')}`,`http://127.0.0.1:4148/${folder}/index.html`],{windowsHide:true,stdio:'ignore'});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));let ws;
try{
 let pages;for(let n=0;n<100;n++){try{pages=await(await fetch('http://127.0.0.1:9448/json')).json();if(pages.length)break;}catch{}await sleep(100);}
 const page=pages.find(p=>p.type==='page');ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));let id=0;const pending=new Map(),errors=[];
 ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){pending.get(m.id)?.(m);pending.delete(m.id);}if(m.method==='Runtime.exceptionThrown')errors.push(m.params.exceptionDetails);});
 const cdp=(method,params={})=>new Promise(r=>{pending.set(++id,r);ws.send(JSON.stringify({id,method,params}));});
 const js=async expression=>(await cdp('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true})).result?.result?.value;
 await cdp('Runtime.enable');await cdp('Emulation.setDeviceMetricsOverride',{width:1100,height:800,deviceScaleFactor:1,mobile:false});
 for(let n=0;n<100&&!await js('window.fixtureReady && document.querySelector("video")');n++)await sleep(100);
 assert.equal(await js('!!document.querySelector("video")'),true,'Replay rendered');
 assert.equal(await js(`document.querySelectorAll('select[aria-label="Game and recording"] option').length`),2);
 assert.equal(await js('document.body.innerText.includes("Late moment")'),true);
 await js(`document.querySelector('select[aria-label="Timeline zoom"]').value="16";document.querySelector('select[aria-label="Timeline zoom"]').dispatchEvent(new Event("change",{bubbles:true}))`);
 await js('[...document.querySelectorAll("button")].find(b=>b.textContent.trim()==="Create clip").click()');await sleep(100);
 assert.equal(await js(`!!document.querySelector('input[aria-label="Clip start"]')`),true);
 await js('[...document.querySelectorAll("button")].find(b=>b.textContent.trim()==="Save clip").click()');await sleep(150);
 assert.equal(await js('window.fixtureCalls.some(c=>c.cmd==="history_save" && c.args.session==="b")'),true);
 assert.equal(await js('window.fixtureApp.toasts.filter(t=>t.text==="Saving Clip").length'),1);
 assert.equal(await js('window.fixtureApp.toasts.some(t=>t.text==="Clip Saved")'),false);
 assert.equal(await js('window.fixtureApp.libraryVersion'),1);
 await js(`document.querySelector('select[aria-label="Game and recording"]').value="a";document.querySelector('select[aria-label="Game and recording"]').dispatchEvent(new Event("change",{bubbles:true}))`);await sleep(100);
 assert.equal(await js('document.querySelectorAll(".available").length'),2,'Gap shown as two intervals');
 const screenshot=await cdp('Page.captureScreenshot',{format:'png'});fs.writeFileSync(path.join(dir,'replay.png'),Buffer.from(screenshot.result.data,'base64'));
 await cdp('Emulation.setDeviceMetricsOverride',{width:760,height:600,deviceScaleFactor:1,mobile:false});await sleep(100);
 assert.equal(await js('document.documentElement.scrollWidth<=innerWidth'),true,'Compact layout fits without horizontal overflow');
 const compact=await cdp('Page.captureScreenshot',{format:'png'});fs.writeFileSync(path.join(dir,'replay-compact.png'),Buffer.from(compact.result.data,'base64'));
 await js('window.fixtureReview("a",90);window.fixtureRelease()');await sleep(100);assert.equal(await js('window.fixtureApp.review===null'),true,'Late reply after closing is released');
 assert.equal(errors.length,0,JSON.stringify(errors));
 fs.writeFileSync(path.join(dir,'results.json'),JSON.stringify({passed:true,checks:11,screenshot:path.join(dir,'replay.png')},null,2));console.log(JSON.stringify({passed:true,checks:11,output:dir},null,2));
 await cdp('Browser.close');
}finally{ws?.close();edge.kill();await server.close();}
