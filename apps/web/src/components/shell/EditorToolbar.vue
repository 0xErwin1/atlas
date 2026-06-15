<script setup lang="ts">
withDefaults(
  defineProps<{
    breadcrumbs?: string[];
    dirty?: boolean;
  }>(),
  {
    breadcrumbs: () => [],
    dirty: false,
  },
);
</script>

<template>
  <div
    class="flex items-center gap-2"
    style="
      height: var(--h-toolbar);
      padding: 0 12px;
      background-color: var(--c-panel);
      border-bottom: 1px solid var(--c-border);
      flex-shrink: 0;
    "
  >
    <div class="flex items-center gap-1 flex-1 min-w-0 overflow-hidden">
      <template
        v-for="(crumb, i) in breadcrumbs"
        :key="crumb"
      >
        <span
          :style="`
            font-family: var(--font-mono);
            font-size: var(--fs-xs);
            color: ${i === breadcrumbs.length - 1 ? 'var(--c-foreground)' : 'var(--c-border)'};
            font-weight: ${i === breadcrumbs.length - 1 ? 'var(--fw-semibold)' : 'var(--fw-normal)'};
            white-space: nowrap;
          `"
        >
          {{ crumb }}
        </span>
        <span
          v-if="i < breadcrumbs.length - 1"
          style="color: var(--c-border); font-size: var(--fs-xs);"
          aria-hidden="true"
        >
          /
        </span>
      </template>
    </div>

    <div class="flex items-center gap-1">
      <slot />
    </div>
  </div>
</template>
