<script setup lang="ts">
import { ref } from 'vue';
import Row from '@/components/ui/Row.vue';
import type { TreeFolder } from '@/lib/notesTree';

defineProps<{
  folder: TreeFolder;
  depth: number;
  activeSlug: string | null;
}>();

const emit = defineEmits<{
  'select-doc': [slug: string];
}>();

const expanded = ref(true);
</script>

<template>
  <div>
    <Row
      :label="folder.name"
      :icon="expanded ? 'folder-open' : 'folder'"
      :depth="depth"
      chevron
      :open="expanded"
      @click="expanded = !expanded"
    />

    <template v-if="expanded">
      <NoteTreeRow
        v-for="child in folder.folders"
        :key="child.id"
        :folder="child"
        :depth="depth + 1"
        :active-slug="activeSlug"
        @select-doc="emit('select-doc', $event)"
      />

      <Row
        v-for="doc in folder.docs"
        :key="doc.id"
        :label="doc.title"
        icon="file"
        :depth="depth + 1"
        :active="activeSlug !== null && doc.slug === activeSlug"
        :disabled="doc.slug === null"
        @click="doc.slug !== null && emit('select-doc', doc.slug)"
      />
    </template>
  </div>
</template>
