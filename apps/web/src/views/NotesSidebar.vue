<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
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
import { type LiveUpdateEvent, useLiveUpdates } from '@/composables/useLiveUpdates';
import { EVENT_TYPE } from '@/lib/eventTypes';
import { boardKey, docKey } from '@/lib/notesTree';
import { useTreeSelection } from '@/stores/treeSelection';
import { useWorkspaceStore } from '@/stores/workspace';

const workspace = useWorkspaceStore();
const selection = useTreeSelection();

const spaceRefs = ref<Array<InstanceType<typeof NotesSpace> | null>>([]);
const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const { activeSlug, activeBoardId, activeViewId } = useActiveSidebarNode();

// Keep the tree's persistent selection in step with whatever is open, document
// or board: the selection store outlives this view (Pinia), and the selected and
// active row styles are both a filled background, so a stale selection reads as
// a second open row. Only navigation moves the active node, so a multi-selection
// made inside the tree (shift/ctrl-click, which never navigates) survives.
watch(
  [activeSlug, activeBoardId],
  ([slug, boardId]) => {
    if (slug !== null) selection.selectOnly(docKey(slug));
    else if (boardId !== null) selection.selectOnly(boardKey(boardId));
    else selection.clear();
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

async function reloadProjects(): Promise<void> {
  const wsSlug = workspace.activeWorkspaceSlug;
  if (wsSlug === null) return;

  await workspace.loadProjects(wsSlug, { preserveOnError: true });
}

function onLiveEvent(event: LiveUpdateEvent): void {
  if (event.type === EVENT_TYPE.PROJECT_CREATED) void reloadProjects();
}

useLiveUpdates(ws, { onEvent: onLiveEvent, onResync: () => void reloadProjects() });

async function refresh(): Promise<void> {
  await reloadProjects();
  await nextTick();

  const reloads = spaceRefs.value.flatMap((space) => (space === null ? [] : [space.reload()]));
  await Promise.all(reloads);
}

// Whole-sidebar loading gate: the tree stays behind a single loader until every
// space's initial catalog has settled, instead of each space popping in on its
// own. The spaces stay mounted (so they load) while the gate is closed; later
// background revalidations never reopen it because a settled space is sticky
// until the project set itself changes (e.g. a workspace switch).
const settledSpaceKeys = ref<Set<string>>(new Set());

function onSpaceSettled(spaceKey: string): void {
  if (settledSpaceKeys.value.has(spaceKey)) return;
  const next = new Set(settledSpaceKeys.value);
  next.add(spaceKey);
  settledSpaceKeys.value = next;
}

function spaceKey(project: (typeof workspace.projects)[number]): string {
  return `${project.workspace_id}:${project.slug}`;
}

const projectSpaceKeys = computed(() => workspace.projects.map(spaceKey));

const allSpacesReady = computed(
  () =>
    projectSpaceKeys.value.length > 0 &&
    projectSpaceKeys.value.every((key) => settledSpaceKeys.value.has(key)),
);

watch(
  () => projectSpaceKeys.value.join('\0'),
  (nextSpaceKeys) => {
    const next = new Set(nextSpaceKeys === '' ? [] : nextSpaceKeys.split('\0'));
    settledSpaceKeys.value = new Set([...settledSpaceKeys.value].filter((key) => next.has(key)));
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

defineExpose({ openNewPage, refresh });
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
            :key="spaceKey(project)"
            :ref="(el) => (spaceRefs[index] = el as InstanceType<typeof NotesSpace> | null)"
            :project="project"
            :active-slug="activeSlug"
            :active-board-id="activeBoardId"
            @initial-settled="onSpaceSettled(spaceKey(project))"
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
