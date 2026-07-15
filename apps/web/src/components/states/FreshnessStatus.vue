<script setup lang="ts">
import type { ResourceStatus } from '@/stores/resourceStatus';

defineProps<{
  status: ResourceStatus;
}>();

defineEmits<{
  retry: [];
}>();

const labels: Partial<Record<ResourceStatus, string>> = {
  refreshing: 'Updating…',
  reconnecting: 'Reconnecting…',
  offline: 'Offline — showing saved data',
  'error-with-data': 'Showing saved data — update failed',
  'error-empty': 'Unable to load data',
};
</script>

<template>
  <div v-if="labels[status]" role="status" class="flex items-center" style="gap: 8px; color: var(--c-muted); font-size: var(--fs-sm);">
    <span>{{ labels[status] }}</span>
    <button
      v-if="status === 'offline' || status === 'error-with-data' || status === 'error-empty'"
      type="button"
      style="border: 0; background: transparent; color: var(--c-primary); cursor: pointer; font: inherit; padding: 0;"
      @click="$emit('retry')"
    >
      Retry
    </button>
  </div>
</template>
