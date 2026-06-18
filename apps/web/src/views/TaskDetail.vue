<script setup lang="ts">
import { computed, watch } from 'vue';
import { useRoute } from 'vue-router';
import type { components } from '@/api/types.d.ts';
import SharePanel from '@/components/share/SharePanel.vue';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import ActivityFeed from '@/components/tareas/ActivityFeed.vue';
import AssigneeList from '@/components/tareas/AssigneeList.vue';
import Checklist from '@/components/tareas/Checklist.vue';
import ReferenceAdd from '@/components/tareas/ReferenceAdd.vue';
import ReferenceList from '@/components/tareas/ReferenceList.vue';
import TaskDescription from '@/components/tareas/TaskDescription.vue';
import Chip from '@/components/ui/Chip.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import MetaRow from '@/components/ui/MetaRow.vue';
import { useBoardsStore } from '@/stores/boards';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
import AppShell from '@/views/AppShell.vue';
import TasksSidebar from '@/views/TasksSidebar.vue';

const route = useRoute();
const workspace = useWorkspaceStore();
const tasks = useTasksStore();
const detail = useTaskDetailStore();
const boards = useBoardsStore();
const ui = useUiStore();

const readableId = computed(() => {
  const id = route.params.readableId;
  return typeof id === 'string' ? id : null;
});

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const task = computed(() => tasks.openTask);

const breadcrumbs = computed(() => ['Atlas', task.value?.readable_id ?? 'Task']);

const statusName = computed(() => {
  const columnId = task.value?.column_id;
  if (columnId === undefined) return null;
  return boards.columns.find((c) => c.id === columnId)?.name ?? null;
});

const assignableOptions = computed<DropdownOption[]>(() => {
  const assigned = new Set(detail.assignees.map((a) => a.assignee.id));
  return workspace.members
    .filter((m) => !assigned.has(m.id))
    .map((m) => ({ value: `${m.principal_type}:${m.id}`, label: m.display }));
});

async function load(): Promise<void> {
  if (readableId.value === null || ws.value === '') {
    return;
  }

  await tasks.loadTask(ws.value, readableId.value);

  const boardId = tasks.openTask?.board_id;
  await Promise.all([
    detail.loadAll(ws.value, readableId.value),
    workspace.loadMembers(ws.value),
    boardId === undefined ? Promise.resolve() : boards.loadColumns(ws.value, boardId),
  ]);
}

async function onAddAssignee(ref: string): Promise<void> {
  if (readableId.value === null) return;
  const [assignee_type, assignee_id] = ref.split(':');
  if (assignee_type === undefined || assignee_id === undefined) return;
  const ok = await detail.addAssignee(ws.value, readableId.value, { assignee_type, assignee_id });
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
}

async function onToggleChecklist(itemId: string): Promise<void> {
  if (readableId.value === null) {
    return;
  }
  const ok = await detail.toggleChecklistItem(ws.value, readableId.value, itemId);
  if (!ok && detail.error) {
    ui.showBanner(detail.error, 'error');
  }
}

async function onPromoteChecklist(itemId: string): Promise<void> {
  if (readableId.value === null || task.value === null) {
    return;
  }

  const result = await detail.promoteChecklistItem(
    ws.value,
    readableId.value,
    itemId,
    task.value.board_id,
    task.value.column_id,
  );

  if (result.ok) {
    ui.showBanner(`Promoted to ${result.readableId}`, 'success');
  } else if (detail.error) {
    ui.showBanner(detail.error, 'error');
  }
}

async function onRemoveAssignee(assigneeType: string, assigneeId: string): Promise<void> {
  if (readableId.value === null) {
    return;
  }
  const ok = await detail.removeAssignee(ws.value, readableId.value, assigneeType, assigneeId);
  if (!ok && detail.error) {
    ui.showBanner(detail.error, 'error');
  }
}

async function onAddChecklist(title: string): Promise<void> {
  if (readableId.value === null) return;
  const ok = await detail.addChecklistItem(ws.value, readableId.value, title);
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
}

async function onAddReference(body: components['schemas']['CreateReferenceRequest']): Promise<void> {
  if (readableId.value === null) return;
  const ok = await detail.addReference(ws.value, readableId.value, body);
  if (ok) ui.showBanner('Reference added', 'success');
  else if (detail.error) ui.showBanner(detail.error, 'error');
}

async function onRemoveReference(referenceId: string): Promise<void> {
  if (readableId.value === null) {
    return;
  }
  const ok = await detail.removeReference(ws.value, readableId.value, referenceId);
  if (!ok && detail.error) {
    ui.showBanner(detail.error, 'error');
  }
}

watch([readableId, ws], load, { immediate: true });
</script>

<template>
  <AppShell>
    <template #sidebar>
      <TasksSidebar />
    </template>

    <EditorToolbar :breadcrumbs="breadcrumbs" :dirty="false" :share-label="`${task?.readable_id ?? 'Task'} · task`">
      <button
        type="button"
        title="Toggle inspector"
        aria-label="Toggle inspector"
        class="flex items-center justify-center"
        style="width: 28px; height: 28px; border: none; background: transparent; cursor: pointer; color: var(--c-muted);"
        @click="ui.toggleInspector()"
      >
        <Icon name="panel-right" :size="16" />
      </button>
    </EditorToolbar>

    <div class="flex-1 overflow-y-auto">
      <div
        v-if="task"
        style="max-width: 680px; margin: 0 auto; padding: 26px 40px;"
      >
        <div class="flex items-center" style="gap: 9px; margin-bottom: 8px; flex-wrap: wrap;">
          <span style="font-family: var(--font-mono); font-size: var(--fs-base); color: var(--c-muted);">
            {{ task.readable_id }}
          </span>
          <Chip v-for="label in task.labels ?? []" :key="label" tone="info">{{ label }}</Chip>
        </div>

        <h1
          style="font-size: var(--fs-title); font-weight: var(--fw-bold); color: var(--c-foreground); margin-bottom: 16px;"
        >
          {{ task.title }}
        </h1>

        <div
          class="flex flex-col"
          style="gap: 2px; background: var(--c-raised); border: 1px solid var(--c-border); border-radius: var(--r-md); padding: 10px 14px; margin-bottom: 20px;"
        >
          <MetaRow label="Priority">
            <Chip v-if="task.priority" tone="info">{{ task.priority }}</Chip>
            <span v-else style="color: var(--c-muted);">None</span>
          </MetaRow>
          <MetaRow label="Assignees">
            <AssigneeList :assignees="detail.assignees" @remove="onRemoveAssignee" />
          </MetaRow>
          <MetaRow v-if="task.estimate != null" label="Estimate">
            <span style="font-family: var(--font-mono);">{{ task.estimate }} pts</span>
          </MetaRow>
        </div>

        <TaskDescription
          :markdown="task.description"
          :ws="ws"
          :readable-id="task.readable_id"
        />

        <div style="margin-top: 20px;">
          <Checklist
            :items="detail.checklist"
            @toggle="onToggleChecklist"
            @promote="onPromoteChecklist"
            @add="onAddChecklist"
          />
        </div>
      </div>

      <p
        v-else-if="tasks.error"
        style="margin: 16px; padding: 8px 12px; border-radius: var(--r-md); background: var(--c-banner-err-bg); color: var(--c-banner-err-fg); font-size: var(--fs-sm);"
      >
        {{ tasks.error }}
      </p>

      <p
        v-else
        style="margin: 16px; font-size: var(--fs-sm); color: var(--c-muted);"
      >
        Loading task…
      </p>
    </div>

    <template #inspector-properties>
      <div class="flex flex-col" style="gap: 8px;">
        <MetaRow label="Status">
          <Chip v-if="statusName" tone="info">{{ statusName }}</Chip>
          <span v-else style="font-size: var(--fs-sm); color: var(--c-muted);">—</span>
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
        <MetaRow v-if="task?.priority" label="Priority">
          <Chip tone="info">{{ task.priority }}</Chip>
        </MetaRow>
      </div>
    </template>

    <template #inspector-backlinks>
      <ReferenceList :references="detail.references" @remove="onRemoveReference" />
      <ReferenceAdd :ws="ws" @add="onAddReference" />
    </template>

    <template #inspector-activity>
      <ActivityFeed :items="detail.activity" />
    </template>

    <template #inspector-share>
      <SharePanel :resource-label="`${task?.readable_id ?? 'Task'} · task`" />
    </template>
  </AppShell>
</template>
