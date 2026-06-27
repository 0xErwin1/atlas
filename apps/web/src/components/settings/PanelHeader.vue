<script setup lang="ts">
defineProps<{ title: string; subtitle?: string }>();
</script>

<template>
  <div class="atl-panel-head" :class="{ 'atl-panel-head--actions': $slots.actions }">
    <div class="atl-panel-head-text">
      <div class="atl-panel-title">{{ title }}</div>
      <div v-if="subtitle || $slots.subtitle" class="atl-panel-sub">
        <slot name="subtitle">{{ subtitle }}</slot>
      </div>
    </div>
    <div v-if="$slots.actions" class="atl-panel-head-actions">
      <slot name="actions" />
    </div>
  </div>
</template>

<style scoped>
.atl-panel-head {
  margin-bottom: 16px;
}

.atl-panel-head--actions {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.atl-panel-head-text {
  min-width: 0;
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

.atl-panel-head-actions {
  flex: 0 0 auto;
}
</style>
