// Decode the existing MP4 through WebView2. Only compressed preview pixels remain in
// memory: no thumbnail files, retained video elements, or recording-sized canvases.
const LIMIT = 48;
const BYTE_LIMIT = 8 * 1024 * 1024;
const cache = new Map<string, { image: string; bytes: number }>();
const jobs = new Map<string, { promise: Promise<string>; signal?: AbortSignal }>();
const queue: Array<() => void> = [];
let active = 0, bytes = 0;

function changed(video: HTMLVideoElement, event: string, signal: AbortSignal, action: () => void): Promise<void> {
  return new Promise((resolve, reject) => {
    const finish = (error?: Error) => {
      video.removeEventListener(event, ready);
      video.removeEventListener('error', failed);
      signal.removeEventListener('abort', aborted);
      error ? reject(error) : resolve();
    };
    const ready = () => finish();
    const failed = () => finish(new Error('This video could not be decoded'));
    const aborted = () => finish(new Error('Preview timed out'));
    if (signal.aborted) { reject(new Error('Preview cancelled')); return; }
    video.addEventListener(event, ready, { once: true });
    video.addEventListener('error', failed, { once: true });
    signal.addEventListener('abort', aborted, { once: true });
    try { action(); } catch (e) { finish(e instanceof Error ? e : new Error(String(e))); }
  });
}

async function decode(url: string, signal?: AbortSignal, at?: number): Promise<string> {
  const video = document.createElement('video');
  const canvas = document.createElement('canvas');
  const sample = document.createElement('canvas');
  const abort = new AbortController();
  const cancelled = () => abort.abort();
  signal?.addEventListener('abort', cancelled, { once: true });
  if (signal?.aborted) abort.abort();
  const timeout = setTimeout(() => abort.abort(), 10000);
  video.muted = true;
  video.playsInline = true;
  video.preload = 'metadata';
  video.crossOrigin = 'anonymous';
  try {
    await changed(video, 'loadedmetadata', abort.signal, () => { video.src = url; video.load(); });
    if (!video.videoWidth || !video.videoHeight) throw new Error('Video has no picture');
    const scale = Math.min(1, 320/video.videoWidth, 180/video.videoHeight);
    canvas.width = Math.max(1, Math.round(video.videoWidth * scale));
    canvas.height = Math.max(1, Math.round(video.videoHeight * scale));
    sample.width = 32; sample.height = 18;
    const ctx = canvas.getContext('2d')!;
    const test = sample.getContext('2d', { willReadFrequently: true })!;
    const duration = video.duration;
    if (!Number.isFinite(duration) || duration <= 0) throw new Error('Video duration is unavailable');
    const positions = at === undefined ? [.1, .25, .5, .75, .9].map(f => duration * f) : [at];
    for (let i = 0; i < positions.length; i++) positions[i] = Math.max(0.001, Math.min(positions[i], duration - .001));
    for (const time of [...new Set(positions)]) {
      if (abort.signal.aborted) throw new Error('Preview cancelled');
      if (Math.abs(time-video.currentTime) > .001) {
        // `seeked` can precede the new decoded surface reaching the compositor.
        // Register before seeking so canvas never samples the previous (often black) frame.
        const presented = new Promise<void>((resolve, reject) => {
          let frame = 0;
          const cancelled = () => { video.cancelVideoFrameCallback(frame); reject(new Error('Preview cancelled')); };
          abort.signal.addEventListener('abort', cancelled, { once: true });
          const ready = (_now: number, metadata: VideoFrameCallbackMetadata) => {
            if (Math.abs(metadata.mediaTime-time) > .15) { frame = video.requestVideoFrameCallback(ready); return; }
            abort.signal.removeEventListener('abort', cancelled); resolve();
          };
          frame = video.requestVideoFrameCallback(ready);
        });
        await Promise.all([presented, changed(video, 'seeked', abort.signal, () => { video.currentTime = time; })]);
      }
      test.drawImage(video, 0, 0, 32, 18);
      const data = test.getImageData(0, 0, 32, 18).data;
      let visible = 0;
      for (let i=0; i<data.length; i+=4) if (Math.max(data[i], data[i+1], data[i+2]) > 24) visible++;
      if (at !== undefined || visible > 32*18*.025) {
        ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
        return canvas.toDataURL('image/webp', .78);
      }
    }
    throw new Error('No visible preview frame found');
  } finally {
    clearTimeout(timeout);
    signal?.removeEventListener('abort', cancelled);
    abort.abort();
    video.pause();
    video.removeAttribute('src');
    video.load();
    canvas.width = canvas.height = sample.width = sample.height = 0;
  }
}

function pump() {
  while (active < 2 && queue.length) { active++; queue.shift()!(); }
}

export function videoPreview(key: string, url: string, signal?: AbortSignal, at?: number): Promise<string> {
  const hit = cache.get(key);
  if (hit) {
    cache.delete(key); cache.set(key, hit);
    return Promise.resolve(hit.image);
  }
  const running = jobs.get(key);
  if (running && !running.signal?.aborted) return running.promise;
  const promise = new Promise<string>((resolve, reject) => {
    queue.push(() => {
      if (signal?.aborted) { active--; reject(new Error('Preview cancelled')); pump(); return; }
      decode(url, signal, at).then(image => {
        const cost = image.length * 2;
        cache.set(key, { image, bytes: cost }); bytes += cost;
        while (cache.size > LIMIT || bytes > BYTE_LIMIT) {
          const oldest = cache.keys().next().value!;
          bytes -= cache.get(oldest)!.bytes; cache.delete(oldest);
        }
        resolve(image);
      }, reject).finally(() => { active--; pump(); });
    });
  });
  jobs.set(key, { promise, signal });
  // Handle rejection here as well: ignored/cancelled requests never create unhandled rejections.
  const finished = () => { if (jobs.get(key)?.promise === promise) jobs.delete(key); };
  promise.then(finished, finished);
  pump();
  return promise;
}
