<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, watch } from 'vue';
import { useRouter } from 'vue-router';
import CommandPalette, { type PaletteSelection } from '@/components/search/CommandPalette.vue';
import type { LocalAction } from '@/composables/useSearch';
import type { SearchHitDto } from '@/stores/search';
import { useSearchStore } from '@/stores/search';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const router = useRouter();
const workspace = useWorkspaceStore();
const searchStore = useSearchStore();
const ui = useUiStore();

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

// Opening the palette (from anywhere — Cmd/Ctrl+K or a toolbar button) starts
// from a clean search slate.
watch(
  () => ui.paletteOpen,
  (open) => {
    if (open) searchStore.clear();
  },
);

const localActions: LocalAction[] = [
  { id: 'goto-notes', label: 'Go to Notes', kind: 'navigate' },
  { id: 'goto-tasks', label: 'Go to Tasks', kind: 'navigate' },
  { id: 'goto-search', label: 'Go to Search', kind: 'navigate' },
];

function runAction(action: LocalAction): void {
  switch (action.id) {
    case 'goto-notes':
      void router.push({ name: 'notes' });
      break;
    case 'goto-tasks':
      void router.push({ name: 'tasks' });
      break;
    case 'goto-search':
      void router.push({ name: 'search' });
      break;
  }
}

function jumpToHit(hit: SearchHitDto): void {
  if (hit.kind === 'task' && hit.readable_id) {
    void router.push({ name: 'task-detail', params: { readableId: hit.readable_id } });
    return;
  }
  void router.push({ name: 'notes', params: { slug: hit.id } });
}

function onSelect(selection: PaletteSelection): void {
  ui.closePalette();
  if (selection.type === 'action') {
    runAction(selection.action);
  } else {
    jumpToHit(selection.hit);
  }
}

function onGlobalKeydown(event: KeyboardEvent): void {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
    event.preventDefault();
    ui.togglePalette();
  }
}

onMounted(() => window.addEventListener('keydown', onGlobalKeydown));
onBeforeUnmount(() => window.removeEventListener('keydown', onGlobalKeydown));
</script>

<template>
  <RouterView />
  <CommandPalette
    :ws="ws"
    :open="ui.paletteOpen"
    :actions="localActions"
    @select="onSelect"
    @close="ui.closePalette()"
  />
</template>
