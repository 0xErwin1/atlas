<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NotesSpace from '@/components/notas/NotesSpace.vue';
import SidebarViews from '@/components/notas/SidebarViews.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { useActiveSidebarNode } from '@/composables/useActiveSidebarNode';
import { useContextMenu } from '@/composables/useContextMenu';
import { docKey } from '@/lib/notesTree';
import { useTreeSelection } from '@/stores/treeSelection';
import { useWorkspaceStore } from '@/stores/workspace';

const workspace = useWorkspaceStore();
const selection = useTreeSelection();

const spaceRefs = ref<Array<InstanceType<typeof NotesSpace> | null>>([]);

const { activeSlug, activeBoardId, activeViewId } = useActiveSidebarNode();

// Keep the tree's persistent selection in step with the open document: the
// selection store outlives this view (Pinia), so without this a doc selected
// before switching apps would stay highlighted on return even with nothing open.
watch(
  activeSlug,
  (slug) => {
    if (slug === null) selection.clear();
    else selection.selectOnly(docKey(slug));
  },
  { immediate: true },
);

async function loadProjects(): Promise<void> {
  const wsSlug = workspace.activeWorkspaceSlug;
  if (wsSlug === null) {
    await workspace.loadProjects('');
    return;
  }
  if (workspace.projects.length === 0) {
    await workspace.loadProjects(wsSlug);
  }
}

onMounted(loadProjects);
watch(() => workspace.activeWorkspaceSlug, loadProjects);

// The footer "New page or board" acts on the first accessible project. Each
// space header also offers per-space creation for a precise context.
const footerSpace = computed(() => spaceRefs.value[0] ?? null);

const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();

const footerMenuItems = computed<MenuItem[]>(() => [
  { label: 'New page', icon: 'file-plus', action: () => footerSpace.value?.startNewPage() },
  { label: 'New board', icon: 'columns-3', action: () => footerSpace.value?.startNewBoard() },
]);

function openFooterMenu(event: MouseEvent): void {
  openAt(event);
}

function openNewPage(): void {
  footerSpace.value?.startNewPage();
}

defineExpose({ openNewPage });
</script>

<template>
  <div class="notes-sidebar">
    <template v-if="workspace.projects.length > 0">
      <SectionLabel>Spaces</SectionLabel>
      <NotesSpace
        v-for="(project, index) in workspace.projects"
        :key="project.slug"
        :ref="(el) => (spaceRefs[index] = el as InstanceType<typeof NotesSpace> | null)"
        :project="project"
        :active-slug="activeSlug"
        :active-board-id="activeBoardId"
      />

      <SidebarViews :active-view-id="activeViewId" />

      <button
        type="button"
        class="notes-sidebar-footer"
        title="New page or board"
        aria-label="New page or board"
        @click="openFooterMenu"
      >
        <Icon name="plus" :size="14" />
        <span>New page or board</span>
      </button>

      <ContextMenu
        :open="menuOpen"
        :x="menuX"
        :y="menuY"
        :items="footerMenuItems"
        @close="closeMenu"
      />
    </template>

    <ErrorState
      v-else-if="workspace.projectsError !== null"
      title="Couldn’t load projects"
      :hint="workspace.projectsError"
      @retry="loadProjects"
    />
    <p
      v-else
      style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No projects yet.
    </p>
  </div>
</template>

<style scoped>
.notes-sidebar {
  display: flex;
  flex-direction: column;
  min-height: 100%;
}

.notes-sidebar-footer {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  height: 30px;
  margin-top: 6px;
  padding: 0 10px;
  border: none;
  border-top: 1px solid var(--c-border);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
  font-size: var(--fs-sm);
  font-weight: var(--fw-medium);
  text-align: left;
}

.notes-sidebar-footer:hover {
  color: var(--c-foreground);
}
</style>
