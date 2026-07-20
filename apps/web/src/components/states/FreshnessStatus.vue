<script setup lang="ts">
import { computed } from 'vue';
import type { ResourceStatus } from '@/stores/resourceStatus';

// `suppressRefreshing` hides the routine background-revalidation row ("Updating…")
// so it never shifts layout in dense contexts like the sidebar tree; genuine
// stale/offline/error signals still surface.
const props = withDefaults(
  defineProps<{
    status: ResourceStatus;
    suppressRefreshing?: boolean;
  }>(),
  { suppressRefreshing: false },
);

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

const label = computed(() =>
  props.suppressRefreshing && props.status === 'refreshing' ? undefined : labels[props.status],
);
</script>

<template>
  <div v-if="label" role="status" class="flex items-center" style="gap: 8px; color: var(--c-muted); font-size: var(--fs-sm);">
    <span>{{ label }}</span>
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
