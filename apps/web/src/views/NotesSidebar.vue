<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NotesSpace from '@/components/notas/NotesSpace.vue';
import SidebarViews from '@/components/notas/SidebarViews.vue';
import ProjectCreateDialog from '@/components/projects/ProjectCreateDialog.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
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

// Whole-sidebar loading gate: the tree stays behind a single loader until every
// space's initial catalog has settled, instead of each space popping in on its
// own. The spaces stay mounted (so they load) while the gate is closed; later
// background revalidations never reopen it because a settled slug is sticky
// until the project set itself changes (e.g. a workspace switch).
const settledSlugs = ref<Set<string>>(new Set());

function onSpaceSettled(slug: string): void {
  if (settledSlugs.value.has(slug)) return;
  const next = new Set(settledSlugs.value);
  next.add(slug);
  settledSlugs.value = next;
}

const projectSlugs = computed(() => workspace.projects.map((project) => project.slug));

const allSpacesReady = computed(
  () => projectSlugs.value.length > 0 && projectSlugs.value.every((slug) => settledSlugs.value.has(slug)),
);

watch(
  () => projectSlugs.value.join('\0'),
  (nextSlugs) => {
    const next = new Set(nextSlugs === '' ? [] : nextSlugs.split('\0'));
    settledSlugs.value = new Set([...settledSlugs.value].filter((slug) => next.has(slug)));
  },
);

// The footer "New page or board" acts on the first accessible project. Each
// space header also offers per-space creation for a precise context.
const footerSpace = computed(() => spaceRefs.value[0] ?? null);

const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();
const createProjectOpen = ref(false);

const footerMenuItems = computed<MenuItem[]>(() => [
  { label: 'New project', icon: 'folder-plus', action: () => (createProjectOpen.value = true) },
]);

function openFooterMenu(event: MouseEvent): void {
  openAt(event);
}

function openNewPage(): void {
  footerSpace.value?.startNewPage();
}

function openBackgroundMenu(event: MouseEvent): void {
  const target = event.target as HTMLElement;
  if (target.closest('input,textarea,button,a,[contenteditable="true"]') !== null) return;

  event.preventDefault();
  openAt(event);
}

defineExpose({ openNewPage });
</script>

<template>
  <div class="notes-sidebar" @contextmenu="openBackgroundMenu">
    <template v-if="workspace.projects.length > 0">
      <LoadingState v-if="!allSpacesReady" label="Loading…" />

      <!-- Spaces stay mounted while the gate is closed so their initial catalogs
           load; the tree is only revealed once every space has settled. -->
      <div v-show="allSpacesReady" class="notes-sidebar-body">
        <div class="notes-sidebar-scroll" role="region" aria-label="Sidebar content">
          <SectionLabel>Spaces</SectionLabel>
          <NotesSpace
            v-for="(project, index) in workspace.projects"
            :key="project.slug"
            :ref="(el) => (spaceRefs[index] = el as InstanceType<typeof NotesSpace> | null)"
            :project="project"
            :active-slug="activeSlug"
            :active-board-id="activeBoardId"
            @initial-settled="onSpaceSettled(project.slug)"
          />
        </div>

        <footer class="notes-sidebar-actions" aria-label="Sidebar actions">
          <SidebarViews :active-view-id="activeViewId" />
          <button
            type="button"
            class="notes-sidebar-footer"
            title="New project"
            aria-label="New project"
            @click="openFooterMenu"
          >
            <Icon name="plus" :size="14" />
            <span>New project</span>
          </button>
        </footer>
      </div>

    </template>

    <ErrorState
      v-else-if="workspace.projectsError !== null"
      title="Couldn’t load projects"
      :hint="workspace.projectsError"
      @retry="loadProjects"
    />
    <EmptyState v-else icon="folder" title="No projects yet." />
    <ContextMenu
      :open="menuOpen"
      :x="menuX"
      :y="menuY"
      :items="footerMenuItems"
      @close="closeMenu"
    />
    <ProjectCreateDialog
      :open="createProjectOpen"
      @created="createProjectOpen = false"
      @cancel="createProjectOpen = false"
    />
  </div>
</template>

<style scoped>
.notes-sidebar {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.notes-sidebar-body {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.notes-sidebar-scroll {
  flex: 1;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  padding-bottom: 4px;
}

.notes-sidebar-actions {
  flex-shrink: 0;
  border-top: 1px solid var(--c-border);
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
