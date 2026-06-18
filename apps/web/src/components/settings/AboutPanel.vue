<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { wrappedClient } from '@/api/wrapper';

const version = ref<string | null>(null);
const build = ref<string | null>(null);
const loading = ref(true);

onMounted(async () => {
  try {
    const { data } = await wrappedClient.GET('/v1/meta', {});
    if (data) {
      version.value = data.version;
      build.value = data.build ?? null;
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
    <div class="atl-panel-head">
      <div class="atl-panel-title">About</div>
      <div class="atl-panel-sub">Server build information</div>
    </div>

    <div class="atl-about">
      <div class="atl-about-row">
        <div class="atl-about-k">Server version</div>
        <div class="atl-about-v">{{ loading ? '…' : (version ?? '—') }}</div>
      </div>
      <div class="atl-about-row">
        <div class="atl-about-k">Build</div>
        <div class="atl-about-v">{{ loading ? '…' : (build ?? '—') }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-panel-head {
  margin-bottom: 16px;
}

.atl-panel-title {
  font-size: 15px;
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
}

.atl-panel-sub {
  font-size: 12px;
  color: var(--c-muted);
  margin-top: 3px;
}

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
