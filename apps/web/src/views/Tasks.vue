<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import SharePanel from '@/components/share/SharePanel.vue';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import ActivityFeed from '@/components/tareas/ActivityFeed.vue';
import AssigneeList from '@/components/tareas/AssigneeList.vue';
import KanbanBoard from '@/components/tareas/KanbanBoard.vue';
import ReferenceList from '@/components/tareas/ReferenceList.vue';
import Btn from '@/components/ui/Btn.vue';
import Chip from '@/components/ui/Chip.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import MetaRow from '@/components/ui/MetaRow.vue';
import { useInlineEdit } from '@/composables/useInlineEdit';
import { useBoardsStore } from '@/stores/boards';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
import AppShell from '@/views/AppShell.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import TasksSidebar from '@/views/TasksSidebar.vue';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const boards = useBoardsStore();
const detail = useTaskDetailStore();
const ui = useUiStore();

const boardId = computed(() => {
  const id = route.params.boardId;
  return typeof id === 'string' ? id : null;
});

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const sidebarRef = ref<InstanceType<typeof TasksSidebar> | null>(null);

const breadcrumbs = computed(() => ['Atlas', boards.board?.name ?? 'Board']);

// Linear-style peek: the selected card's details show in the inspector dock
// without leaving the board. Live fields (title, priority, status) come from the
// reactive board summary so a context-menu change reflects immediately; the
// richer data (assignees, backlinks, activity) is loaded into the detail store.
const selectedReadableId = ref<string | null>(null);

const selected = computed(() =>
  selectedReadableId.value === null ? null : (boards.findTaskByReadableId(selectedReadableId.value) ?? null),
);

const selectedStatus = computed(() => {
  const columnId = selected.value?.column_id;
  return boards.columns.find((c) => c.id === columnId)?.name ?? null;
});

async function onSelect(readableId: string): Promise<void> {
  selectedReadableId.value = readableId;
  ui.inspectorOpen = true;
  ui.setInspectorTab('properties');
  await Promise.all([detail.loadAll(ws.value, readableId), workspace.loadMembers(ws.value)]);
}

const PRIORITY_OPTIONS: DropdownOption[] = [
  { value: '', label: 'None' },
  { value: 'urgent', label: 'Urgent' },
  { value: 'high', label: 'High' },
  { value: 'medium', label: 'Medium' },
  { value: 'low', label: 'Low' },
];

const statusOptions = computed<DropdownOption[]>(() =>
  boards.columns.map((c) => ({ value: c.id, label: c.name })),
);

// Members not already assigned, offered in the "add assignee" picker.
const assignableOptions = computed<DropdownOption[]>(() => {
  const assigned = new Set(detail.assignees.map((a) => a.assignee.id));
  return workspace.members
    .filter((m) => !assigned.has(m.id))
    .map((m) => ({ value: `${m.principal_type}:${m.id}`, label: m.display }));
});

// Inline title editing: the context is the task's readable id so a commit fires
// against the right task even if selection changed during a blur.
const {
  active: titleActive,
  value: titleValue,
  inputRef: titleInputRef,
  start: startTitle,
  commit: commitTitleEdit,
  onKeydown: onTitleKeydown,
} = useInlineEdit<string>((title, readableId) => {
  void commitTitle(readableId, title);
});

async function commitTitle(readableId: string, title: string): Promise<void> {
  const ok = await boards.updateTask(ws.value, readableId, { title });
  if (!ok && boards.error) ui.showBanner(boards.error, 'error');
}

async function onChangeStatus(columnId: string): Promise<void> {
  if (selectedReadableId.value === null) return;
  const ok = await boards.moveTaskToColumn(ws.value, selectedReadableId.value, columnId);
  if (!ok && boards.error) ui.showBanner(boards.error, 'error');
}

async function onChangePriority(value: string): Promise<void> {
  if (selectedReadableId.value === null) return;
  const ok = await boards.updateTask(ws.value, selectedReadableId.value, {
    priority: value === '' ? null : value,
  });
  if (!ok && boards.error) ui.showBanner(boards.error, 'error');
}

async function onAddAssignee(ref: string): Promise<void> {
  if (selectedReadableId.value === null) return;
  const [assignee_type, assignee_id] = ref.split(':');
  if (assignee_type === undefined || assignee_id === undefined) return;
  const ok = await detail.addAssignee(ws.value, selectedReadableId.value, { assignee_type, assignee_id });
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
}

async function onRemoveAssignee(assigneeType: string, assigneeId: string): Promise<void> {
  if (selectedReadableId.value === null) return;
  const ok = await detail.removeAssignee(ws.value, selectedReadableId.value, assigneeType, assigneeId);
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
}

async function onRemoveReference(referenceId: string): Promise<void> {
  if (selectedReadableId.value === null) return;
  const ok = await detail.removeReference(ws.value, selectedReadableId.value, referenceId);
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
}

async function loadBoard(): Promise<void> {
  if (ws.value === '') return;

  // A different board invalidates the current selection/peek.
  selectedReadableId.value = null;

  // No board in the URL (e.g. the rail "Tasks" button): pick the project's first
  // board and redirect to it, mirroring how /n opens without a slug.
  if (boardId.value === null) {
    await resolveDefaultBoard();
    return;
  }

  await boards.loadBoard(ws.value, boardId.value);
  await Promise.all([boards.loadColumns(ws.value, boardId.value), boards.loadTasks(ws.value, boardId.value)]);
}

async function resolveDefaultBoard(): Promise<void> {
  if (workspace.projects.length === 0) {
    await workspace.loadProjects(ws.value);
  }

  const project = workspace.projects[0];
  if (project === undefined) return;

  await boards.loadBoards(ws.value, project.slug);

  const first = boards.boardSummaries[0];
  if (first !== undefined) {
    await router.replace({ name: 'tasks', params: { boardId: first.id } });
  }
}

function openTask(readableId: string): void {
  void router.push({ name: 'task-detail', params: { readableId } });
}

watch([boardId, ws], loadBoard, { immediate: true });
</script>

<template>
  <AppShell sidebar-title="Tasks" sidebar-icon="square-kanban">
    <template #sidebar-actions>
      <button type="button" class="atl-gbtn" title="Filter" aria-label="Filter">
        <Icon name="search" :size="14" />
      </button>
      <button type="button" class="atl-gbtn" title="Collapse" aria-label="Collapse sidebar">
        <Icon name="panel-left" :size="13" />
      </button>
    </template>

    <template #sidebar>
      <TasksSidebar ref="sidebarRef" />
    </template>

    <template #sidebar-footer>
      <button
        type="button"
        class="atl-gbtn"
        style="width: 100%; justify-content: flex-start; height: 26px; gap: 7px; color: var(--c-foreground);"
        @click="sidebarRef?.openNewProject()"
      >
        <Icon name="plus" :size="14" />
        New project
      </button>
    </template>

    <EditorToolbar :breadcrumbs="breadcrumbs" :dirty="false">
      <button type="button" class="atl-gbtn" title="Filter" aria-label="Filter">
        <Icon name="filter" :size="14" />
        Filter
      </button>
      <button type="button" class="atl-gbtn" title="Command palette ⌘K" aria-label="Command palette">
        <Icon name="command" :size="14" />
      </button>
      <button
        type="button"
        class="atl-gbtn"
        title="Toggle inspector"
        aria-label="Toggle inspector"
        @click="ui.toggleInspector()"
      >
        <Icon name="panel-right" :size="14" />
      </button>
    </EditorToolbar>

    <ErrorState
      v-if="boards.error"
      title="Couldn’t load board"
      :hint="boards.error"
      @retry="loadBoard"
    />
    <LoadingState v-else-if="boards.loading" label="Loading…" />
    <EmptyState
      v-else-if="boardId === null"
      title="No board selected"
      hint="Pick a board from the sidebar to see its tasks"
      icon="square-kanban"
    />
    <KanbanBoard
      v-else
      :ws="ws"
      :selected-readable-id="selectedReadableId"
      @select="onSelect"
      @open="openTask"
    />

    <template #inspector-properties>
      <div v-if="selected" class="flex flex-col" style="gap: 12px;">
        <div>
          <div style="font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-muted);">
            {{ selected.readable_id }}
          </div>
          <input
            v-if="titleActive === selected.readable_id"
            ref="titleInputRef"
            v-model="titleValue"
            class="atl-peek-title-input"
            @keydown="onTitleKeydown"
            @blur="commitTitleEdit"
          />
          <div
            v-else
            class="atl-peek-title"
            title="Click to rename"
            @click="startTitle(selected.readable_id, selected.title, true)"
          >
            {{ selected.title }}
          </div>
        </div>

        <div class="flex flex-col" style="gap: 8px;">
          <MetaRow label="Status">
            <Dropdown
              :options="statusOptions"
              :model-value="selected.column_id"
              placeholder="—"
              @change="onChangeStatus"
            />
          </MetaRow>
          <MetaRow label="Priority">
            <Dropdown
              :options="PRIORITY_OPTIONS"
              :model-value="selected.priority ?? ''"
              @change="onChangePriority"
            />
          </MetaRow>
          <MetaRow label="Assignees">
            <div class="flex flex-col" style="gap: 6px; align-items: flex-start;">
              <AssigneeList :assignees="detail.assignees" @remove="onRemoveAssignee" />
              <Dropdown
                v-if="assignableOptions.length"
                :options="assignableOptions"
                placeholder="+ Add assignee"
                @change="onAddAssignee"
              />
            </div>
          </MetaRow>
        </div>

        <Btn variant="secondary" @click="openTask(selected.readable_id)">Open full task</Btn>
      </div>
      <EmptyState
        v-else
        icon="square-kanban"
        title="No task selected"
        hint="Click a task on the board to see its details here."
      />
    </template>

    <template #inspector-backlinks>
      <ReferenceList v-if="selected" :references="detail.references" @remove="onRemoveReference" />
      <EmptyState v-else icon="link" title="No task selected" hint="Click a task to see its backlinks." />
    </template>

    <template #inspector-activity>
      <ActivityFeed v-if="selected" :items="detail.activity" />
      <EmptyState v-else icon="clock" title="No task selected" hint="Click a task to see its activity." />
    </template>

    <template #inspector-share>
      <SharePanel v-if="selected" :resource-label="`${selected.readable_id} · task`" />
      <EmptyState v-else icon="user" title="No task selected" hint="Click a task to share it." />
    </template>
  </AppShell>
</template>

<style scoped>
.atl-peek-title {
  margin-top: 2px;
  padding: 2px 4px;
  margin-left: -4px;
  border-radius: var(--r-sm);
  font-size: var(--fs-md);
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
  cursor: text;
}

.atl-peek-title:hover {
  background: var(--c-raised);
}

.atl-peek-title-input {
  width: 100%;
  margin-top: 2px;
  padding: 2px 4px;
  background: var(--c-panel);
  border: 1px solid var(--c-primary);
  border-radius: var(--r-sm);
  font-size: var(--fs-md);
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
  outline: none;
}
</style>
