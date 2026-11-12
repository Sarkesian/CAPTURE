<script lang="ts" module>
  // Simple 24×24 stroke icons drawn for CAPTURE (2 px stroke, round caps), so the app has one
  // consistent icon language without an icon-font dependency.
  const P: Record<string, string> = {
    home: 'M4 11.5 12 5l8 6.5V19a1 1 0 0 1-1 1h-4v-5h-6v5H5a1 1 0 0 1-1-1z',
    replay: 'M4 12a8 8 0 1 0 2.3-5.6M4 4v4h4 M12 8v4l3 2',
    library: 'M4 5h7v6H4z M13 5h7v6h-7z M4 13h7v6H4z M13 13h7v6h-7z',
    settings: 'M4 7h10 M18 7h2 M16 5v4 M4 17h4 M12 17h8 M10 15v4 M4 12h14 M20 12h0',
    mic: 'M12 4a3 3 0 0 0-3 3v5a3 3 0 0 0 6 0V7a3 3 0 0 0-3-3z M6 11a6 6 0 0 0 12 0 M12 17v3',
    'mic-off': 'M4 4l16 16 M9 9v3a3 3 0 0 0 5 2.2 M15 10V7a3 3 0 0 0-5.8-1 M6 11a6 6 0 0 0 9.6 4.8 M18 11c0 .7-.1 1.4-.3 2 M12 17v3',
    speaker: 'M5 10h3l4-4v12l-4-4H5z M16 9a4 4 0 0 1 0 6 M18.5 6.5a7.5 7.5 0 0 1 0 11',
    'speaker-off': 'M5 10h3l4-4v12l-4-4H5z M16 9l5 6 M21 9l-5 6',
    chat: 'M5 6h14v9H10l-4 3v-3H5z',
    monitor: 'M4 5h16v11H4z M9 20h6 M12 16v4',
    game: 'M7 9h10a4 4 0 0 1 4 4v1a3 3 0 0 1-5.4 1.8L14.5 14h-5l-1.1 1.8A3 3 0 0 1 3 14v-1a4 4 0 0 1 4-4z M8 11v3 M6.5 12.5h3 M16 12h.01 M18 13.5h.01',
    folder: 'M4 7a1 1 0 0 1 1-1h4l2 2h8a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1z',
    marker: 'M7 4h10v16l-5-4-5 4z',
    record: 'M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10z',
    stop: 'M7 7h10v10H7z',
    save: 'M12 4v11 M7.5 10.5 12 15l4.5-4.5 M5 19h14',
    play: 'M8 5.5v13l10.5-6.5z',
    pause: 'M8 5v14 M16 5v14',
    fullscreen: 'M9 4H4v5 M15 4h5v5 M20 15v5h-5 M9 20H4v-5',
    'skip-back': 'M18 6v12l-8.5-6z M6 6v12',
    'skip-fwd': 'M6 6v12l8.5-6z M18 6v12',
    back: 'M15 18l-6-6 6-6',
    rewind: 'M12 7l-6 5 6 5z M19 7l-6 5 6 5z',
    forward: 'M12 7l6 5-6 5z M5 7l6 5-6 5z',
    chevron: 'M6 9l6 6 6-6',
    more: 'M6 12h.01 M12 12h.01 M18 12h.01',
    check: 'M5 12.5l4.5 4.5L19 7',
    x: 'M6 6l12 12 M18 6 6 18',
    alert: 'M12 4 3 19h18z M12 10v4 M12 16.5h.01',
    scissors: 'M7 7a2.5 2.5 0 1 0 0 .01 M7 17a2.5 2.5 0 1 0 0 .01 M9 8.5 19 18 M9 15.5 19 6',
    camera: 'M4 8h3l2-2h6l2 2h3v11H4z M12 10.5a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
    refresh: 'M19 12a7 7 0 1 1-2-4.9 M19 4v4h-4',
    search: 'M11 5a6 6 0 1 0 0 12 6 6 0 0 0 0-12z M20 20l-4.3-4.3',
    copy: 'M9 9h10v10H9z M5 15V5h10',
    pencil: 'M15 5l4 4L9 19H5v-4z',
    trash: 'M5 7h14 M10 7V5h4v2 M7 7l1 12h8l1-12',
    overlay: 'M4 5h16v14H4z M8 9h8 M8 13h5',
    info: 'M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16z M12 11v5 M12 8h.01',
    live: 'M12 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4z M7.8 7.8a6 6 0 0 0 0 8.4 M16.2 7.8a6 6 0 0 1 0 8.4',
    audio: 'M4 10v4 M8 7v10 M12 4v16 M16 8v8 M20 11v2',
  };
  export type IconName = keyof typeof P;
</script>

<script lang="ts">
  let { name, size = 18, label = '' }: { name: string; size?: number; label?: string } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill={name === 'play' || name === 'record' || name === 'stop' ? 'currentColor' : 'none'}
  stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
  role={label ? 'img' : undefined} aria-label={label || undefined} aria-hidden={label ? undefined : 'true'} class="icon">
  <path d={P[name] ?? ''} />
</svg>

<style>
  .icon { flex: none; display: block; }
</style>
