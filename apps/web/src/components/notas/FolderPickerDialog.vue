<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import { buildNotesTree, type FolderInput, type TreeFolder } from '@/lib/notesTree';

/**
 * Modal that picks a destination folder (or the project root) for a move/copy.
 * The parent owns visibility via `open` and reacts to `confirm` (with the chosen
 * folder id, or `null` for the project root) or `cancel`.
 */
const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    folders: FolderInput[];
    confirmLabel?: string;
  }>(),
  { confirmLabel: 'Move here' },
);

const emit = defineEmits<{
  confirm: [folderId: string | null];
  cancel: [];
}>();

// null = project root.
const selected = ref<string | null>(null);

watch(
  () => props.open,
  (open) => {
    if (open) selected.value = null;
  },
);

interface Option {
  id: string | null;
  name: string;
  depth: number;
}

const options = computed<Option[]>(() => {
  const out: Option[] = [{ id: null, name: 'Project root', depth: 0 }];
  const tree = buildNotesTree(props.folders, []);

  function walk(folders: TreeFolder[], depth: number): void {
    for (const folder of folders) {
      out.push({ id: folder.id, name: folder.name, depth });
      walk(folder.folders, depth + 1);
    }
  }

  walk(tree.folders, 1);
  return out;
});

function onKeydown(event: KeyboardEvent): void {
  if (props.open && event.key === 'Escape') emit('cancel');
}

onMounted(() => window.addEventListener('keydown', onKeydown));
onUnmounted(() => window.removeEventListener('keydown', onKeydown));
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="fixed inset-0 flex items-center justify-center"
      style="background: rgba(7, 10, 15, 0.66); z-index: 300; padding: 24px;"
      @mousedown.self="emit('cancel')"
    >
      <div
        role="dialog"
        aria-modal="true"
        :style="{
          width: '420px',
          maxWidth: '100%',
          maxHeight: '80vh',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--c-raised)',
          border: '1px solid var(--c-border)',
          borderRadius: 'var(--r-md)',
          boxShadow: 'var(--shadow-lg)',
          overflow: 'hidden',
          fontFamily: 'var(--font-ui)',
        }"
      >
        <div
          style="padding: 14px 16px; border-bottom: 1px solid var(--c-border); font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--c-foreground);"
        >
          {{ title }}
        </div>

        <div style="flex: 1; overflow-y: auto; padding: 6px;">
          <button
            v-for="opt in options"
            :key="opt.id ?? '__root__'"
            type="button"
            class="atl-folder-opt flex items-center"
            :class="{ on: selected === opt.id }"
            :style="{ paddingLeft: `${8 + opt.depth * 16}px` }"
            @click="selected = opt.id"
            @dblclick="emit('confirm', opt.id)"
          >
            <Icon
              :name="opt.id === null ? 'home' : 'folder'"
              :size="14"
              :style="{ color: selected === opt.id ? 'var(--c-primary)' : 'var(--c-muted)', flex: '0 0 auto' }"
            />
            <span style="flex: 1; text-align: left;">{{ opt.name }}</span>
            <Icon v-if="selected === opt.id" name="check" :size="14" :style="{ color: 'var(--c-primary)' }" />
          </button>
        </div>

        <div
          class="flex justify-end"
          style="gap: 8px; padding: 12px 16px; border-top: 1px solid var(--c-border);"
        >
          <Btn variant="secondary" @click="emit('cancel')">Cancel</Btn>
          <Btn variant="primary" @click="emit('confirm', selected)">{{ confirmLabel }}</Btn>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.atl-folder-opt {
  gap: 8px;
  width: 100%;
  height: 30px;
  padding-right: 8px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-sm);
  color: var(--c-foreground);
}

.atl-folder-opt:hover {
  background: var(--c-background);
}

.atl-folder-opt.on {
  background: var(--c-selection);
}
</style>
