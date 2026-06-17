<script setup lang="ts">
withDefaults(
  defineProps<{
    parts?: string[];
  }>(),
  {
    parts: () => [],
  },
);
</script>

<template>
  <div
    class="flex items-center min-w-0"
    style="gap: 7px; font-size: var(--fs-sm); color: var(--c-muted);"
  >
    <template
      v-for="(part, i) in parts"
      :key="`${i}-${part}`"
    >
      <span
        class="truncate"
        :style="`
          color: ${i === parts.length - 1 ? 'var(--c-foreground)' : 'var(--c-muted)'};
          font-weight: ${i === parts.length - 1 ? 'var(--fw-semibold)' : 'var(--fw-normal)'};
        `"
      >
        {{ part }}
      </span>
      <span
        v-if="i < parts.length - 1"
        style="color: var(--c-border);"
        aria-hidden="true"
      >
        /
      </span>
    </template>
  </div>
</template>
