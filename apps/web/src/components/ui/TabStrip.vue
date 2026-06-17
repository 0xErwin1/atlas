<script setup lang="ts">
import Icon from '@/components/ui/Icon.vue';

export interface Tab {
  id: string;
  name: string;
  icon: string;
  active?: boolean;
  dirty?: boolean;
}

withDefaults(
  defineProps<{
    tabs?: Tab[];
    closable?: boolean;
  }>(),
  {
    tabs: () => [],
    closable: false,
  },
);

defineEmits<{
  select: [id: string];
  close: [id: string];
}>();
</script>

<template>
  <div
    class="flex items-end"
    style="
      height: 32px;
      flex: 0 0 32px;
      background-color: var(--c-panel);
      border-bottom: 1px solid var(--c-border);
      padding: 0 4px;
      gap: 2px;
    "
  >
    <div
      v-for="tab in tabs"
      :key="tab.id"
      class="atl-tab inline-flex items-center"
      :title="tab.name"
      role="tab"
      :aria-selected="tab.active === true"
      :style="`
        gap: 8px;
        height: 28px;
        padding: 0 8px 0 10px;
        border-radius: 2px 2px 0 0;
        background: ${tab.active ? 'var(--c-background)' : 'transparent'};
        color: ${tab.active ? 'var(--c-foreground)' : 'var(--c-muted)'};
        font-weight: ${tab.active ? 'var(--fw-semibold)' : 'var(--fw-medium)'};
        font-size: var(--fs-sm);
        cursor: pointer;
        box-shadow: ${tab.active ? 'inset 0 -2px 0 var(--c-primary)' : 'none'};
        max-width: 200px;
        min-width: 110px;
      `"
      @click="$emit('select', tab.id)"
    >
      <Icon :name="tab.icon" :size="12" style="flex-shrink: 0;" />
      <span class="flex-1 truncate">{{ tab.name }}</span>
      <span
        v-if="tab.dirty"
        style="width: 6px; height: 6px; border-radius: 9999px; background: var(--c-primary); flex-shrink: 0;"
        aria-hidden="true"
      />
      <span
        v-if="closable"
        class="atl-x inline-flex items-center justify-center"
        style="width: 14px; height: 14px; border-radius: 2px; color: var(--c-muted); font-size: 13px; flex-shrink: 0;"
        role="button"
        :aria-label="`Close ${tab.name}`"
        @click.stop="$emit('close', tab.id)"
      >
        ×
      </span>
    </div>

    <div class="flex items-center" style="margin-left: auto; gap: 4px; padding-right: 4px; align-self: center;">
      <slot name="right" />
    </div>
  </div>
</template>
