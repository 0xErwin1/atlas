<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import Icon from '@/components/ui/Icon.vue';

const props = withDefaults(
  defineProps<{
    /** Height (px) the content is clamped to while collapsed. */
    collapsedHeight?: number;
  }>(),
  { collapsedHeight: 220 },
);

const expanded = ref(false);
const overflowing = ref(false);
const content = ref<HTMLElement | null>(null);

let observer: ResizeObserver | null = null;

function measure(): void {
  const el = content.value;
  if (el === null) return;
  overflowing.value = el.scrollHeight > props.collapsedHeight + 4;
}

onMounted(() => {
  measure();
  if (typeof ResizeObserver !== 'undefined' && content.value !== null) {
    observer = new ResizeObserver(() => measure());
    observer.observe(content.value);
  }
});

onBeforeUnmount(() => {
  observer?.disconnect();
  observer = null;
});

function toggle(): void {
  expanded.value = !expanded.value;
}

defineExpose({ measure });
</script>

<template>
  <div class="atl-collapsible" :data-expanded="expanded">
    <div
      ref="content"
      class="atl-collapsible-content"
      :class="{ clamped: !expanded && overflowing }"
      :style="!expanded && overflowing ? { maxHeight: `${collapsedHeight}px` } : undefined"
    >
      <slot />
    </div>

    <button v-if="overflowing" type="button" class="atl-collapsible-toggle" @click="toggle">
      <Icon :name="expanded ? 'chevron-up' : 'chevron-down'" :size="14" />
      {{ expanded ? 'Show less' : 'Show more' }}
    </button>
  </div>
</template>

<style scoped>
.atl-collapsible-content.clamped {
  overflow: hidden;
  -webkit-mask-image: linear-gradient(to bottom, #000 62%, transparent);
  mask-image: linear-gradient(to bottom, #000 62%, transparent);
}

.atl-collapsible-toggle {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-top: 6px;
  padding: 2px 0;
  background: transparent;
  border: none;
  cursor: pointer;
  color: var(--c-primary);
  font-size: var(--fs-sm);
}

.atl-collapsible-toggle:hover {
  text-decoration: underline;
}
</style>
