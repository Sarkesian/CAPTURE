// Real WebView2 pointer input against a disposable extracted review build. Never targets the installed app.
import fs from 'node:fs';import path from 'node:path';import {spawn} from 'node:child_process';import assert from 'node:assert/strict';
const dir=path.resolve(process.argv[2]),exe=path.join(dir,'CAPTURE.exe'),media=path.join(dir,'ReviewCaptures');
const settings=JSON.parse(fs.readFileSync(path.join(dir,'Data/Settings/settings.json')));
assert.equal(settings.capture_root,media);assert.equal(settings.replay_enabled,false);
const port=9462,env={...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`};
const sleep=ms=>new Promise(r=>setTimeout(r,ms));let checks=0;
const check=(v,label)=>{assert.ok(v,label);checks++;console.log('PASS '+label);};
const wait=async(fn,label,ms=12000)=>{const until=Date.now()+ms;while(Date.now()<until){const v=await fn();if(v)return v;await sleep(60);}throw Error('Timed out: '+label);};
let child,ws;
async function start(){
 child=spawn(exe,['--library'],{cwd:dir,env,windowsHide:true,stdio:'ignore'});
 const url=await wait(async()=>{try{return (await(await fetch(`http://127.0.0.1:${port}/json`)).json()).find(p=>p.type==='page'&&p.url.endsWith('#library'))?.webSocketDebuggerUrl;}catch{}},'isolated WebView2');
 ws=new WebSocket(url);await new Promise((r,j)=>{ws.addEventListener('open',r,{once:true});ws.addEventListener('error',j,{once:true});});let id=0;const pending=new Map();
 ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id&&pending.has(m.id)){pending.get(m.id)(m);pending.delete(m.id);}});
 const cdp=(method,params={})=>new Promise(r=>{pending.set(++id,r);ws.send(JSON.stringify({id,method,params}));});
 const js=async expression=>{const r=await cdp('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(r.result?.exceptionDetails)throw Error(JSON.stringify(r.result.exceptionDetails));return r.result?.result?.value;};
 const rect=sel=>js(`(()=>{const el=document.querySelector(${JSON.stringify(sel)});if(!el)return null;el.scrollIntoView({block:'nearest'});const b=el.getBoundingClientRect();return {x:b.x,y:b.y,w:b.width,h:b.height};})()`);
 const mouse=(type,x,y,buttons)=>cdp('Input.dispatchMouseEvent',{type,x,y,button:type==='mouseMoved'?'none':'left',buttons,clickCount:type==='mouseMoved'?0:1});
 const click=async sel=>{const b=await rect(sel);assert.ok(b,sel);const x=b.x+b.w/2,y=b.y+b.h/2;await mouse('mousePressed',x,y,1);await mouse('mouseReleased',x,y,0);await sleep(100);};
 const shot=async name=>{const r=await cdp('Page.captureScreenshot',{format:'png'});assert.ok(r.result?.data);fs.writeFileSync(path.join(dir,name+'.png'),Buffer.from(r.result.data,'base64'));};
 await cdp('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:'dark'}]});await cdp('Emulation.setDeviceMetricsOverride',{width:1280,height:860,deviceScaleFactor:1,mobile:false});
 await wait(()=>js('document.querySelectorAll(".recording-card").length===5 && [...document.querySelectorAll(".recording-card .duration")].every(e=>!/[a-z]ired|Loading/i.test(e.textContent))'),'five indexed recordings',45000);
 return {js,click,rect,mouse,shot};
}
async function stop(){ws?.close();if(child && child.exitCode==null){spawn(exe,['--quit'],{cwd:dir,windowsHide:true,stdio:'ignore'});await wait(()=>child.exitCode!=null,'review build exits');}child=null;}
try{
 let ui=await start();
 check(await ui.js('document.querySelector("h1").textContent==="Library" && !document.querySelector("#nav-replay")'),'native launch opens unified Library');
 await wait(()=>ui.js('document.querySelector(".recording-card img")?.naturalWidth>0'),'game card artwork');
 const art=await ui.js('decodeURIComponent(document.querySelector(".recording-card img").src)');console.log('Cover source: '+(art.toLowerCase().includes('librarycache')?'local Steam artwork':'capture screenshot fallback'));
 await ui.shot('library');const card=await ui.rect('.recording-card');await ui.mouse('mouseMoved',card.x+card.w/2,card.y+card.h/2,0);await sleep(180);await ui.shot('library-hover');
 await ui.click('.recording-card');await wait(()=>ui.js('document.querySelector(".stage video")?.readyState>=2'),'native history preview');
 check(await ui.js('document.querySelectorAll("[role=slider]").length===1 && Number(document.querySelector(".detail").getAttribute("aria-valuemax"))>=21600'),'six-hour recording opens with one timeline');await ui.shot('recording-viewer');
 await ui.click('.moment[aria-label*="102 moments"]');await ui.click('.cluster-event:nth-of-type(2)');
 await wait(()=>ui.js('document.querySelector(".time").textContent.includes("2:00:06") && document.querySelector(".stage video")?.readyState>=2 && !document.querySelector(".busy")'),'marker playback');check(true,'native marker seek at 2:00:06');
 await ui.click('.transport-end .primary');
 const end=await ui.rect('.handle.end'),x=end.x+end.w/2,y=end.y+end.h/2;
 await ui.mouse('mousePressed',x,y,1);await ui.mouse('mouseMoved',x-100,y,1);await ui.mouse('mouseReleased',x-100,y,0);await sleep(100);
 check(await ui.js('document.querySelector(".transport-end .primary").textContent.trim()==="Save Clip" && !document.querySelector(".transport-end .primary").disabled'),'native trim changes length and offers Save Clip');await ui.shot('clip-selection');
 const started=performance.now();await ui.click('.transport-end .primary');
 const clips=path.join(media,'The Witcher 3','Clips');const clip=await wait(()=>fs.readdirSync(clips,{throwIfNoEntry:false})?.find(p=>p.endsWith('.mp4')),'MP4 in configured save folder');
 check(fs.statSync(path.join(clips,clip)).size>100000,'real MP4 written into configured capture folder');console.log(`File visible after ${Math.round(performance.now()-started)} ms`);
 await ui.click('.back');const play=`.tile[aria-label=${JSON.stringify('Play '+clip)}]`;await wait(()=>ui.js(`!!document.querySelector(${JSON.stringify(play)})`),'Library updates');await ui.click(play);
 await wait(()=>ui.js('document.querySelector(".saved-stage video")?.currentTime>.3'),'MP4 playback');check(true,'saved MP4 plays in Library viewer');
 await stop();ui=await start();check(await ui.js(`!!document.querySelector(${JSON.stringify(play)})`)||await wait(()=>ui.js(`!!document.querySelector(${JSON.stringify(play)})`),'saved clip after restart'),'saved clip persists after restart');
 await ui.click('.recording-card');await wait(()=>ui.js('document.querySelector(".stage video")?.readyState>=2'),'reopened history');check(await ui.js(`!!document.querySelector(${JSON.stringify('.moment[aria-label*="102 moments"]')})`),'indexed markers and screenshots persist after restart');
 console.log(JSON.stringify({passed:true,checks,output:dir},null,2));
}finally{await stop();}
