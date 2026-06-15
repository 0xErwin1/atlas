<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
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
    <div
      style="
        padding: 6px 8px 2px;
        font-family: var(--font-mono);
        font-size: var(--fs-xs);
        color: var(--c-muted);
        text-transform: uppercase;
        letter-spacing: 0.04em;
      "
    >
      {{ projectName }}
    </div>

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

      <button
        v-for="doc in tree.docs"
        :key="doc.id"
        type="button"
        class="flex items-center gap-1 w-full text-left"
        :disabled="doc.slug === null"
        :aria-current="activeSlug !== null && doc.slug === activeSlug"
        :style="`
          height: 24px;
          padding-left: 8px;
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
