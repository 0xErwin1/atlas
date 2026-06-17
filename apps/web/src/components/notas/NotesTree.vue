<script setup lang="ts">
import { computed } from 'vue';
import Row from '@/components/ui/Row.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { buildNotesTree, type DocInput, type FolderInput } from '@/lib/notesTree';
import NoteTreeRow from './NoteTreeRow.vue';

const props = defineProps<{
  projectName: string;
  folders: FolderInput[];
  docs: DocInput[];
  activeSlug: string | null;
}>();

const emit = defineEmits<{
  'select-doc': [slug: string];
}>();

const tree = computed(() => buildNotesTree(props.folders, props.docs));

const isEmpty = computed(() => tree.value.folders.length === 0 && tree.value.docs.length === 0);
</script>

<template>
  <div>
    <SectionLabel>{{ projectName }}</SectionLabel>

    <p
      v-if="isEmpty"
      style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No documents yet.
    </p>

    <template v-else>
      <NoteTreeRow
        v-for="folder in tree.folders"
        :key="folder.id"
        :folder="folder"
        :depth="0"
        :active-slug="activeSlug"
        @select-doc="emit('select-doc', $event)"
      />

      <Row
        v-for="doc in tree.docs"
        :key="doc.id"
        :label="doc.title"
        icon="file"
        :active="activeSlug !== null && doc.slug === activeSlug"
        :disabled="doc.slug === null"
        @click="doc.slug !== null && emit('select-doc', doc.slug)"
      />
    </template>
  </div>
</template>
