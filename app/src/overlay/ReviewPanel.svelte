<script lang="ts">
  import HistoryPanel from './HistoryPanel.svelte';
  let { onclose, compact = false, onlibrary }: { onclose: () => void; compact?: boolean; onlibrary?: () => void } = $props();
</script>
<HistoryPanel {onclose} {compact} {onlibrary} />
