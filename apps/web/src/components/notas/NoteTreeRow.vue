<script setup lang="ts">
import { ref } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import type { TreeFolder } from '@/lib/notesTree';

const props = defineProps<{
  folder: TreeFolder;
  depth: number;
  activeSlug: string | null;
}>();

const emit = defineEmits<{
  'select-doc': [slug: string];
}>();

const expanded = ref(true);

function indent(depth: number): string {
  return `${8 + depth * 14}px`;
}
</script>

<template>
  <div>
    <button
      type="button"
      class="flex items-center gap-1 w-full text-left"
      :style="`
        height: 24px;
        padding-left: ${indent(depth)};
        padding-right: 8px;
        border: none;
        cursor: pointer;
        background: transparent;
        color: var(--c-foreground);
        font-size: var(--fs-sm);
      `"
      @click="expanded = !expanded"
    >
      <Icon :name="expanded ? 'chevron-down' : 'chevron-right'" :size="12" />
      <Icon name="folder" :size="13" />
      <span class="truncate">{{ folder.name }}</span>
    </button>

    <template v-if="expanded">
      <NoteTreeRow
        v-for="child in folder.folders"
        :key="child.id"
        :folder="child"
        :depth="depth + 1"
        :active-slug="activeSlug"
        @select-doc="emit('select-doc', $event)"
      />

      <button
        v-for="doc in folder.docs"
        :key="doc.id"
        type="button"
        class="flex items-center gap-1 w-full text-left"
        :disabled="doc.slug === null"
        :aria-current="activeSlug !== null && doc.slug === activeSlug"
        :style="`
          height: 24px;
          padding-left: ${indent(depth + 1)};
          padding-right: 8px;
          border: none;
          cursor: ${doc.slug === null ? 'default' : 'pointer'};
          background: ${activeSlug !== null && doc.slug === activeSlug ? 'var(--c-list-active)' : 'transparent'};
          color: ${activeSlug !== null && doc.slug === activeSlug ? 'var(--c-primary)' : 'var(--c-foreground)'};
          font-size: var(--fs-sm);
          font-weight: ${activeSlug !== null && doc.slug === activeSlug ? 'var(--fw-semibold)' : 'var(--fw-normal)'};
        `"
        @click="doc.slug !== null && emit('select-doc', doc.slug)"
      >
        <Icon name="file" :size="13" />
        <span class="truncate">{{ doc.title }}</span>
      </button>
    </template>
  </div>
</template>
