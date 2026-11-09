// Test the real Svelte preview component and decoder in an isolated headless Edge profile.
// Uses only existing video media plus controlled black/corrupt fixtures. No thumbnails saved.
// node scripts/e2e-previews.mjs <work-folder>
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { createReadStream, mkdirSync, writeFileSync, readFileSync, statSync, unlinkSync } from 'node:fs';
import path from 'node:path';
const root=path.resolve(import.meta.dirname,'..'), work=path.resolve(process.argv[2]);
mkdirSync(work,{recursive:true});
const ffmpeg=path.join(root,'third_party/ffmpeg/bin/ffmpeg.exe');
const mixed=path.join(work,'black-opening.mp4'), black=path.join(work,'black.mp4');
const recordings=process.argv.slice(3);
execFileSync(ffmpeg,['-v','error','-y','-f','lavfi','-i','color=black:s=640x360:r=30:d=8','-f','lavfi','-i','testsrc2=s=640x360:r=30:d=2','-filter_complex','[0:v][1:v]concat=n=2:v=1:a=0','-c:v','h264_nvenc','-preset','p1','-pix_fmt','yuv420p',mixed]);
execFileSync(ffmpeg,['-v','error','-y','-f','lavfi','-i','color=black:s=640x360:r=30:d=2','-c:v','h264_nvenc','-preset','p1','-pix_fmt','yuv420p',black]);
let requests=0, transferred=0;
const server=createServer((req,res)=>{
  res.setHeader('Access-Control-Allow-Origin','*');
  if(req.url==='/stats'){res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({requests}));return}
  if(req.url.startsWith('/corrupt')){res.writeHead(200,{'Content-Type':'video/mp4'});res.end('broken media');return}
  const file=req.url.startsWith('/session.mp4')&&recordings[0]?recordings[0]
    :req.url.startsWith('/replay.mp4')&&recordings[1]?recordings[1]
    :req.url.startsWith('/black.mp4')?black:mixed;
  const size=statSync(file).size;requests++;
  const range=/bytes=(\d+)-(\d*)/.exec(req.headers.range||'');
  const start=range?+range[1]:0,end=range&&range[2]?Math.min(size-1,+range[2]):size-1;
  res.writeHead(range?206:200,{'Content-Type':'video/mp4','Accept-Ranges':'bytes','Content-Length':end-start+1,...(range?{'Content-Range':`bytes ${start}-${end}/${size}`}:{})});
  transferred+=end-start+1;createReadStream(file,{start,end}).pipe(res);
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const media=`http://127.0.0.1:${server.address().port}`;
const pageFile=path.join(root,'app/preview-acceptance.html');
const html=`<!doctype html><meta charset="utf-8"><style>body{background:#121418;color:#eee;font:14px system-ui;margin:24px}#grid{display:grid;grid-template-columns:repeat(5,200px);gap:16px}.card{height:113px;position:relative;background:#252932;border-radius:8px}h1{font-size:22px}.label{margin-top:120px}</style><h1>Video preview acceptance · direct decoding, memory cache</h1><div id="recent"></div><div id="grid"></div><script type="module">
import {mount,unmount} from 'svelte';
import Preview from '/src/components/VideoPreview.svelte';
import RecentClips from '/src/components/RecentClips.svelte';
import {videoPreview} from '/src/lib/videoPreview.ts';
import {app,init} from '/src/lib/store.svelte.ts';
const callbacks=new Map(),listeners=new Map();let callbackId=0;
window.__TAURI_INTERNALS__={convertFileSrc:p=>p,transformCallback:fn=>{callbacks.set(++callbackId,fn);return callbackId},invoke:async(cmd,args)=>{
 if(cmd==='bootstrap')return {settings:{},data_dir:'',last_state:null,engine_down:null,app:null,path_issues:[],notices:[]};
 if(cmd==='plugin:event|listen'){listeners.set(args.event,callbacks.get(args.handler));return 1}return [];
}};
window.results=[];performance.setResourceTimingBufferSize(5000);const check=(ok,label)=>{results.push({ok,label});console.log((ok?'PASS ':'FAIL ')+label)};
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
window.done=(async()=>{
await init();const engine=(event,data)=>listeners.get('engine')({payload:{v:1,event,data}});
for(const action of ['screenshot','replay','recording_start','recording_stop']){
 app.toasts=[];engine('capture_triggered',{action,overlay_notice:false});
 const expected={screenshot:'Screenshot saved',replay:'Clip Saved',recording_start:'Recording started',recording_stop:'Recording saved'}[action];
 check(app.toasts.length===1&&app.toasts[0].text===expected,'one immediate UI confirmation for '+action);
 engine('save_started',{save_id:1,requested_secs:3});engine('save_finished',{save_id:1,path:'test.mp4',duration_secs:3,requested_secs:3});engine('screenshot_saved',{path:'test.png'});
 check(app.toasts.length===1,'processing completion stays silent for '+action);
}
app.toasts=[];engine('capture_triggered',{action:'replay',overlay_notice:true});check(app.toasts.length===0,'native overlay feedback is not duplicated in the UI');
engine('save_failed',{save_id:1,message:'Disk unavailable'});check(app.toasts.length===1&&app.toasts[0].kind==='error','save failure remains clearly visible');
${process.argv.includes('--feedback-only')?'return {results};':''}
let last=performance.now(),maxGap=0;const heartbeat=setInterval(()=>{const now=performance.now();maxGap=Math.max(maxGap,now-last);last=now},16);
const url=${JSON.stringify(media)};
await sleep(0); // let mocked save-completion list refreshes settle before seeding recent clips
app.clips=[0,1].map(i=>({path:url+'/opening.mp4?recent='+i,name:'Recent '+i,game:'Preview test',bytes:123,modified_ms:1,kind:'clip',has_stems:false,has_markers:false}));
for(const clip of app.clips)app.clipInfo[clip.path]={duration:125};
let played=false;mount(RecentClips,{target:document.querySelector('#recent'),props:{count:4,onplay:()=>played=true}});
const recentEnd=performance.now()+10000;while(document.querySelectorAll('#recent img').length<2&&performance.now()<recentEnd)await sleep(50);
check(document.querySelectorAll('#recent img').length===2,'Home Recent clips uses real decoded previews');
check(document.querySelectorAll('#recent .dur').length===2&&[...document.querySelectorAll('#recent .dur')].every(e=>e.textContent==='2m 05s'),'Recent clips retains duration badges');
document.querySelector('#recent .thumb').click();check(played,'Recent clips still opens the selected clip');
const start=performance.now();
const first=await videoPreview('opening',url+'/opening.mp4');
const img=new Image();img.src=first;await img.decode();
check(img.width<=320&&img.height<=180,'preview stays within 320×180');
const c=document.createElement('canvas');c.width=32;c.height=18;const ctx=c.getContext('2d');ctx.drawImage(img,0,0,32,18);const px=ctx.getImageData(0,0,32,18).data;
check([...px].filter((v,i)=>i%4!==3&&v>30).length>100,'skips an 8-second black opening and selects visible content');
${recordings.length>=2?"for(const file of ['session.mp4','replay.mp4']){const image=await videoPreview(file,url+'/'+file);check(image.startsWith('data:image/webp'),'real '+file+' decodes directly into a memory preview');}":''}
const cached=performance.now();check(await videoPreview('opening',url+'/opening.mp4')===first&&performance.now()-cached<10,'cached preview is reused immediately');
for(const [name,file] of [['black','black.mp4'],['corrupt','corrupt.mp4']]){
let error=false;try{await videoPreview(name,url+'/'+file)}catch(e){error=true}check(error,name+' video has an explicit error result');}
const cards=[];
for(let i=0;i<20;i++){const target=document.createElement('div');target.className='card';document.querySelector('#grid').append(target);cards.push(mount(Preview,{target,props:{path:url+'/opening.mp4?card='+i,bytes:i,modified:1}}));}
const end=performance.now()+20000;while(document.querySelectorAll('.card img').length<20&&performance.now()<end)await sleep(50);
check(document.querySelectorAll('.card img').length===20,'20 real Svelte cards show usable images asynchronously');
const cancelled=new AbortController();const pending=videoPreview('cancelled',url+'/opening.mp4?cancel',cancelled.signal);cancelled.abort();
let aborted=false;try{await pending}catch{aborted=true}check(aborted,'cancelled preview frees its decoder slot');
await Promise.all(Array.from({length:55},(_,i)=>videoPreview('lru-'+i,url+'/opening.mp4?lru='+i)));
const oldRequests=(await(await fetch(url+'/stats')).json()).requests;
await videoPreview('opening',url+'/opening.mp4');
check((await(await fetch(url+'/stats')).json()).requests>oldRequests,'old preview is evicted after the memory cache reaches its entry limit');
check(maxGap<250,'library remains responsive; largest heartbeat gap '+maxGap.toFixed(1)+' ms');
clearInterval(heartbeat);window.elapsed=performance.now()-start;
return {results,elapsed:window.elapsed,maxGap};
})().catch(e=>{results.push({ok:false,label:e.stack});return{results}});
</script>`;
writeFileSync(pageFile,html);
const vite=spawn(process.execPath,[path.join(root,'app/node_modules/vite/bin/vite.js'),'--host','127.0.0.1','--port','9346','--strictPort'],{cwd:path.join(root,'app'),stdio:'ignore',windowsHide:true});
const browser=spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',['--headless=new','--no-first-run','--no-default-browser-check','--disable-background-timer-throttling','--remote-debugging-port=9347','--user-data-dir='+path.join(work,'edge-profile'),'about:blank'],{stdio:'ignore',windowsHide:true});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
let ws;
try{
  let page;
  for(let i=0;i<100;i++){try{await fetch('http://127.0.0.1:9346/preview-acceptance.html');page=(await(await fetch('http://127.0.0.1:9347/json')).json()).find(x=>x.type==='page');if(page)break}catch{}await sleep(100)}
  if(!page)throw Error('Headless browser did not start');
  ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
  let id=0;const pending=new Map();ws.addEventListener('message',ev=>{const m=JSON.parse(ev.data);if(pending.has(m.id)){pending.get(m.id)(m);pending.delete(m.id)}});
  const cdp=(method,params={})=>new Promise(r=>{const n=++id;pending.set(n,r);ws.send(JSON.stringify({id:n,method,params}))});
  await cdp('Emulation.setDeviceMetricsOverride',{width:1120,height:800,deviceScaleFactor:1,mobile:false});
  await cdp('Network.enable');await cdp('Network.setCacheDisabled',{cacheDisabled:true});
  await cdp('Page.navigate',{url:'http://127.0.0.1:9346/preview-acceptance.html'});
  await sleep(500);
  const response=await cdp('Runtime.evaluate',{expression:'window.done',awaitPromise:true,returnByValue:true});
  const result=response.result?.result?.value;
  if(!result)throw Error(JSON.stringify(response));
  result.requests=requests;result.transferred=transferred;
  writeFileSync(path.join(work,'preview-results.json'),JSON.stringify(result,null,2));
  console.log(JSON.stringify(result,null,2));
  const screenshot=await cdp('Page.captureScreenshot',{format:'png'});
  writeFileSync(path.join(work,'preview-grid.png'),Buffer.from(screenshot.result.data,'base64'));
  process.exitCode=result.results.every(x=>x.ok)?0:1;
  await cdp('Browser.close');
}catch(e){console.error(e);process.exitCode=1}finally{
  ws?.close();browser.kill();vite.kill();server.close();unlinkSync(pageFile);
}
