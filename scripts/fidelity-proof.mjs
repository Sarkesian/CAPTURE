// Focused synthetic hardware proof; does not capture the user's screen or game.
import fs from 'node:fs';import path from 'node:path';import {spawnSync} from 'node:child_process';import assert from 'node:assert/strict';import crypto from 'node:crypto';
const repo=path.resolve(import.meta.dirname,'..'),out=path.join(repo,'test-output',`fidelity-${Date.now()}`);fs.mkdirSync(out,{recursive:true});
const ff=path.join(repo,'third_party/ffmpeg/bin/ffmpeg.exe');
function run(args){const t=performance.now();const r=spawnSync(ff,['-v','error',...args],{encoding:'utf8',windowsHide:true});assert.equal(r.status,0,r.stderr);return performance.now()-t;}
const input=path.join(out,'source.yuv'),clip=path.join(out,'lossless-444.mp4'),decoded=path.join(out,'decoded.yuv');
run(['-f','lavfi','-i','testsrc2=size=256x144:rate=30','-t','2','-pix_fmt','yuv444p','-f','rawvideo',input]);
const encode_ms=run(['-f','rawvideo','-pixel_format','yuv444p','-video_size','256x144','-framerate','30','-i',input,'-c:v','h264_nvenc','-preset','p3','-tune','lossless','-rc','constqp','-qp','0','-profile:v','high444p','-bf','0',clip]);
run(['-i',clip,'-pix_fmt','yuv444p','-f','rawvideo',decoded]);
const source=fs.readFileSync(input),roundtrip=fs.readFileSync(decoded);assert.deepEqual(roundtrip,source);
const rgb=path.join(out,'source.bgra'),rgbclip=path.join(out,'rgb-converted.mp4'),rgbdecoded=path.join(out,'decoded.bgra');
run(['-f','lavfi','-i','testsrc=size=256x144:rate=30','-t','2','-pix_fmt','bgra','-f','rawvideo',rgb]);
run(['-f','rawvideo','-pixel_format','bgra','-video_size','256x144','-framerate','30','-i',rgb,'-c:v','h264_nvenc','-preset','p3','-tune','lossless','-rc','constqp','-qp','0','-profile:v','high444p','-rgb_mode','yuv444','-bf','0',rgbclip]);
run(['-i',rgbclip,'-pix_fmt','bgra','-f','rawvideo',rgbdecoded]);
const a=fs.readFileSync(rgb),b=fs.readFileSync(rgbdecoded);let different=0,max_error=0;for(let i=0;i<a.length;i++){const d=Math.abs(a[i]-b[i]);if(d)different++;max_error=Math.max(max_error,d);}
const result={out,frames:60,resolution:'256x144',fps:30,encode_ms,encoded_bytes:fs.statSync(clip).size,yuv444_bit_exact:true,yuv_sha256:crypto.createHash('sha256').update(source).digest('hex'),rgb_bit_exact:different===0,rgb_different_bytes:different,rgb_max_channel_error:max_error,scope:'Synthetic system-memory NVENC proof only; not an end-to-end D3D11/WGC fidelity or gameplay-performance proof'};
fs.writeFileSync(path.join(out,'results.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result,null,2));
