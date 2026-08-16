<script setup lang="ts">
/**
 * Says out loud when live updates are not arriving.
 *
 * The broker retries a dropped stream and eventually gives up, and until this
 * existed both of those were silent: the app kept rendering the last data it
 * had and looked exactly as live as before. It shows nothing while the stream
 * is healthy — a permanent "connected" light is noise nobody reads.
 */
import { computed, ref } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { useLiveUpdates } from '@/composables/useLiveUpdates';
import type { LiveConnectionState } from '@/lib/workspaceLiveUpdates';

const props = defineProps<{ ws: string }>();

const state = ref<LiveConnectionState>('connected');

useLiveUpdates(
  computed(() => props.ws),
  {
    onEvent: () => undefined,
    onResync: () => undefined,
    onConnectionState: (next) => {
      state.value = next;
    },
  },
);

const label = computed(() =>
  state.value === 'reconnecting'
    ? 'Reconnecting to live updates…'
    : 'Live updates are offline — this view may be out of date',
);
</script>

<template>
  <div
    v-if="state !== 'connected'"
    class="atl-live-status"
    :class="{ offline: state === 'offline' }"
    role="status"
    :title="label"
    :aria-label="label"
  >
    <Icon :name="state === 'offline' ? 'cloud-off' : 'refresh-cw'" :size="14" />
  </div>
</template>

<style scoped>
.atl-live-status {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 32px;
  color: var(--c-muted);
}

.atl-live-status.offline {
  color: var(--c-danger);
}
</style>
