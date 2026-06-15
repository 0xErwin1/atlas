<script setup lang="ts">
withDefaults(
  defineProps<{
    title?: string;
    collapsible?: boolean;
  }>(),
  {
    title: 'Explorer',
    collapsible: true,
  },
);
</script>

<template>
  <aside
    style="
      width: 256px;
      min-width: 248px;
      max-width: 264px;
      background-color: var(--c-panel);
      border-right: 1px solid var(--c-border);
      flex-shrink: 0;
      height: 100%;
      display: flex;
      flex-direction: column;
      overflow: hidden;
    "
  >
    <div
      class="flex items-center justify-between"
      style="
        height: 36px;
        padding: 0 10px;
        border-bottom: 1px solid var(--c-border);
        flex-shrink: 0;
      "
    >
      <span
        style="
          font-family: var(--font-mono);
          font-size: var(--fs-base);
          font-weight: var(--fw-bold);
          color: var(--c-foreground);
        "
      >
        {{ title }}
      </span>

      <div class="flex items-center gap-1">
        <slot name="header-actions" />
      </div>
    </div>

    <div
      class="flex-1 overflow-y-auto overflow-x-hidden"
      style="padding: 4px 0;"
    >
      <slot />
    </div>
  </aside>
</template>
