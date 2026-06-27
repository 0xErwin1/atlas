<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { wrappedClient } from '@/api/wrapper';
import PanelHeader from '@/components/settings/PanelHeader.vue';

const version = ref<string | null>(null);
const build = ref<string | null>(null);
const url = ref<string | null>(null);
const loading = ref(true);

onMounted(async () => {
  try {
    const { data } = await wrappedClient.GET('/v1/meta', {});
    if (data) {
      version.value = data.version;
      build.value = data.build ?? null;
      url.value = data.url ?? null;
    }
  } catch {
    // leave fields null; the panel renders an em dash
  } finally {
    loading.value = false;
  }
});
</script>

<template>
  <div>
    <PanelHeader title="About" subtitle="Server build information" />

    <div class="atl-about">
      <div class="atl-about-row">
        <div class="atl-about-k">Server version</div>
        <div class="atl-about-v">{{ loading ? '…' : (version ?? '—') }}</div>
      </div>
      <div v-if="url" class="atl-about-row">
        <div class="atl-about-k">URL</div>
        <div class="atl-about-v">{{ url }}</div>
      </div>
      <div class="atl-about-row">
        <div class="atl-about-k">Build</div>
        <div class="atl-about-v">{{ loading ? '…' : (build ?? '—') }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-about {
  border-bottom: 1px solid var(--c-border);
}

.atl-about-row {
  display: flex;
  align-items: center;
  height: 34px;
  border-top: 1px solid var(--c-border);
}

.atl-about-k {
  flex: 0 0 130px;
  font-size: 12.5px;
  color: var(--c-muted);
}

.atl-about-v {
  flex: 1;
  font-size: 12.5px;
  font-family: var(--font-mono);
  color: var(--c-foreground);
}
</style>
