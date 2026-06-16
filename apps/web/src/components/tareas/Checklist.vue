<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import type { ChecklistItemDto } from '@/stores/taskDetail';

const props = defineProps<{
  items: ChecklistItemDto[];
}>();

const emit = defineEmits<{
  toggle: [itemId: string];
  promote: [itemId: string];
}>();

const doneCount = computed(() => props.items.filter((i) => i.checked).length);
</script>

<template>
  <section>
    <div
      style="
        font-size: var(--fs-xs);
        font-weight: var(--fw-semibold);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        color: var(--c-muted);
        margin-bottom: 6px;
      "
    >
      Sub-tasks · {{ doneCount }} / {{ items.length }}
    </div>

    <div
      v-for="item in items"
      :key="item.id"
      class="group flex items-center"
      style="gap: 8px; padding: 4px 0; font-size: var(--fs-base);"
      :data-checklist-item="item.id"
    >
      <button
        type="button"
        :aria-pressed="item.checked"
        :aria-label="item.checked ? 'Uncheck item' : 'Check item'"
        class="flex items-center justify-center shrink-0 cursor-pointer"
        :style="{
          width: '15px',
          height: '15px',
          borderRadius: 'var(--r-sm)',
          border: item.checked ? 'none' : '1px solid var(--c-muted)',
          backgroundColor: item.checked ? 'var(--c-success)' : 'transparent',
          color: 'var(--c-background)',
          padding: 0,
        }"
        @click="emit('toggle', item.id)"
      >
        <Icon v-if="item.checked" name="check" :size="12" />
      </button>

      <span
        class="flex-1 min-w-0"
        :style="{
          color: item.checked ? 'var(--c-muted)' : 'var(--c-foreground)',
          textDecoration: item.checked ? 'line-through' : 'none',
        }"
      >
        {{ item.title }}
      </span>

      <a
        v-if="item.promoted_readable_id"
        class="shrink-0"
        style="font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-info);"
      >
        {{ item.promoted_readable_id }}
      </a>
      <button
        v-else
        type="button"
        title="Promote to task"
        aria-label="Promote to task"
        class="shrink-0 cursor-pointer opacity-0 group-hover:opacity-100 flex items-center justify-center"
        style="
          width: 22px;
          height: 22px;
          border: 1px solid var(--c-border);
          border-radius: var(--r-sm);
          background: var(--c-secondary);
          color: var(--c-muted);
        "
        @click="emit('promote', item.id)"
      >
        <Icon name="arrow-up-right" :size="13" />
      </button>
    </div>

    <p
      v-if="items.length === 0"
      style="font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No sub-tasks.
    </p>
  </section>
</template>
