<script lang="ts">
  import HistoryPanel from './HistoryPanel.svelte';
  let { onclose, compact = false }: { onclose: () => void; compact?: boolean } = $props();
</script>
<HistoryPanel {onclose} {compact} />
