# CAPTURE 0.6.5 clip editor

The recording viewer puts the video above a slim timeline and one compact control row. Playback and elapsed time sit in the center, with Clip at the lower right. Timeline gaps and moment markers stay visible when the session contains them. Hovering shows a timecode and, within the loaded preview window, a thumbnail.

Clip highlights a range around the playhead on the same timeline. Drag either handle to adjust the range. The duration updates as you drag. Use comma and period to step the playhead by source frame, I and O to set the boundaries, or focus a handle and use the arrow keys. Save Clip writes an MP4 to the configured capture folder. Save and Cancel both return to watch mode in the viewer.

## Changed components

- `HistoryPanel.svelte` and `ReplayTimeline.svelte` implement the compact viewer, playback and trim controls, pointer capture, and contextual Moments and zoom.
- `store.svelte.ts`, `timeline.ts` and `videoPreview.ts` coordinate asynchronous seeks, frame timing, and on-demand thumbnails.
- The history worker reads timestamps from nearby source segments. The aligned MP4 writer checks the requested start and output duration before publishing the file. The viewer uses the existing capture folder and Library integration.

## Verification

- 82 Rust workspace tests passed.
- 41 browser checks passed, including playback with 4K60 and portrait fixtures, resizing, 150% scaling, pointer release, and late seek replies.
- Eight native Windows checks passed. WebView2 opened a six-hour sample, sought to its 2:00:06 marker, dragged the Out handle, saved a playable MP4, and found the clip and source moments after restart.
- FFprobe read 361 video frames and a duration of 12.033333 seconds from the native save. The worker also verified that the file duration matched the selected interval within four milliseconds before publishing.

## Remaining limits

Stream-copy export uses a source keyframe for In. CAPTURE aligns the visible boundary before enabling Save Clip. Out can end at a video frame boundary. A clip must stay within a contiguous recorded section. Thumbnail previews use the current decoded playback window. Variable-frame-rate export still needs an end-to-end check.

The before, watch, and trim screenshots are from native Windows builds. The bright patterns in the preview come from generated test media.
