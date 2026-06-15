<script setup lang="ts">
export interface Tab {
  id: string;
  label: string;
  dirty?: boolean;
}

const props = withDefaults(
  defineProps<{
    tabs?: Tab[];
    activeId?: string;
  }>(),
  {
    tabs: () => [],
    activeId: undefined,
  },
);

const emit = defineEmits<{
  select: [id: string];
  close: [id: string];
}>();
</script>

<template>
  <div
    class="flex items-center overflow-x-auto"
    style="
      height: var(--h-tab);
      background-color: var(--c-background);
      border-bottom: 1px solid var(--c-border);
      flex-shrink: 0;
    "
  >
    <div
      v-for="tab in tabs"
      :key="tab.id"
      class="flex items-center gap-1 relative"
      :style="`
        height: var(--h-tab);
        padding: 0 10px;
        cursor: pointer;
        flex-shrink: 0;
        border-right: 1px solid var(--c-border);
        background-color: ${props.activeId === tab.id ? 'var(--c-background)' : 'var(--c-panel)'};
        color: ${props.activeId === tab.id ? 'var(--c-foreground)' : 'var(--c-muted)'};
      `"
      @click="emit('select', tab.id)"
    >
      <span
        v-if="props.activeId === tab.id"
        style="
          position: absolute;
          top: 0;
          left: 0;
          right: 0;
          height: 2px;
          background-color: var(--c-primary);
        "
        aria-hidden="true"
      />

      <span style="font-family: var(--font-mono); font-size: var(--fs-xs); font-weight: var(--fw-medium);">
        {{ tab.label }}
      </span>

      <span
        v-if="tab.dirty"
        style="
          width: 5px;
          height: 5px;
          border-radius: 50%;
          background-color: var(--c-primary);
          flex-shrink: 0;
        "
        aria-hidden="true"
      />

      <button
        type="button"
        :aria-label="`Close ${tab.label}`"
        style="
          width: 14px;
          height: 14px;
          border: none;
          cursor: pointer;
          border-radius: 2px;
          background: transparent;
          color: var(--c-muted);
          display: flex;
          align-items: center;
          justify-content: center;
          padding: 0;
          flex-shrink: 0;
        "
        @click.stop="emit('close', tab.id)"
      >
        ×
      </button>
    </div>

    <slot />
  </div>
</template>
