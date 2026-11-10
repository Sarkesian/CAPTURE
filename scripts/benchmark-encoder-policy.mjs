// Bounded, reproducible encoder comparison. Offline metrics are not gameplay claims.
import {spawnSync} from 'node:child_process';import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';
const repo=path.resolve(import.meta.dirname,'..'),bin=path.join(repo,'third_party/ffmpeg/bin'),root=path.join(repo,'test-output',`encoder-policy-${Date.now()}`);fs.mkdirSync(root,{recursive:true});
const run=(exe,args)=>{const t=performance.now();const p=spawnSync(path.join(bin,exe),args,{windowsHide:true,encoding:'utf8',maxBuffer:32e6});assert.equal(p.status,0,p.stderr);return {seconds:(performance.now()-t)/1000,stdout:p.stdout,stderr:p.stderr};};
const results=[];
for(const [quality,preset,cq,bpp] of [['high-original',4,20,.12],['high-fixed',5,21,.12],['maximum-original',4,17,.20],['maximum-fixed',6,18,.20]]){
 const output=path.join(root,quality+'.mp4'),maxrate=Math.round(3840*2160*60*bpp);
 const a=run('ffmpeg.exe',['-hide_banner','-loglevel','info','-f','lavfi','-i','testsrc2=size=3840x2160:rate=60','-frames:v','180','-an','-c:v','h264_nvenc','-preset',`p${preset}`,'-tune','hq','-rc','vbr','-cq',String(cq),'-b:v','0','-maxrate',String(maxrate),'-bufsize',String(maxrate),'-profile:v','high','-spatial-aq','1','-g','60','-bf','0','-color_primaries','bt709','-color_trc','bt709','-colorspace','bt709',output]);
 const s=run('ffmpeg.exe',['-hide_banner','-i',output,'-f','lavfi','-i','testsrc2=size=3840x2160:rate=60','-filter_complex','[0:v]setpts=PTS-STARTPTS[a];[1:v]trim=end_frame=180,setpts=PTS-STARTPTS[b];[a][b]ssim','-an','-f','null','-']);
 const p=run('ffmpeg.exe',['-hide_banner','-i',output,'-f','lavfi','-i','testsrc2=size=3840x2160:rate=60','-filter_complex','[0:v]setpts=PTS-STARTPTS[a];[1:v]trim=end_frame=180,setpts=PTS-STARTPTS[b];[a][b]psnr','-an','-f','null','-']);
 const info=JSON.parse(run('ffprobe.exe',['-v','error','-count_frames','-show_streams','-of','json',output]).stdout).streams[0];
 const result={quality,preset,cq,frames:info.nb_read_frames,width:info.width,height:info.height,avg_fps:info.avg_frame_rate,encode_seconds:a.seconds,end_to_end_fps:180/a.seconds,bytes:fs.statSync(output).size,ssim:Number(s.stderr.match(/All:([\d.]+)/)?.[1]),psnr_db:Number(p.stderr.match(/average:([\d.]+)/)?.[1]),path:output};
 fs.writeFileSync(path.join(root,quality+'.log'),a.stderr+'\n'+s.stderr+'\n'+p.stderr);results.push(result);console.log(JSON.stringify(result));
}
fs.writeFileSync(path.join(root,'results.json'),JSON.stringify({workload:'180 deterministic testsrc2 YUV420 frames, 4K60, CPU-generated input; wall time includes process startup and upload',results},null,2));console.log('Results: '+root);
