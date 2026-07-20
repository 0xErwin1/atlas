<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import Row from '@/components/ui/Row.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useInlineEdit } from '@/composables/useInlineEdit';
import { type TaskViewDto, useTaskViewsStore } from '@/stores/taskViews';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{
  activeViewId: string | null;
}>();

const router = useRouter();
const workspace = useWorkspaceStore();
const taskViews = useTaskViewsStore();
const ui = useUiStore();

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

// The workspace's built-in task views. Kept identical to the legacy tasks
// sidebar so relocating the block adds no new view types (spec: VIEWS block
// relocates existing views only).
const PREDEFINED_VIEWS = [
  { viewId: 'my-tasks', label: 'My tasks', icon: 'star', agent: false },
  { viewId: 'recently-updated', label: 'Recently updated', icon: 'clock', agent: false },
  { viewId: 'agent-activity', label: 'Agent activity', icon: 'sparkles', agent: true },
];

function openView(viewId: string): void {
  void router.push({ name: 'task-view', params: { viewId } });
}

type EditCtx = { kind: 'rename-view'; viewId: string; filters: TaskViewDto['filters'] };

const {
  active: editActive,
  value: editValue,
  inputRef,
  start: startEdit,
  commit: commitEdit,
  onKeydown: onEditKeydown,
} = useInlineEdit<EditCtx>(async (name, ctx) => {
  if (ws.value === '') return;
  const ok = await taskViews.update(ws.value, ctx.viewId, { name, filters: ctx.filters });
  if (!ok && taskViews.error) ui.showBanner(taskViews.error, 'error');
});

const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();
const menuTarget = ref<TaskViewDto | null>(null);

const menuItems = computed<MenuItem[]>(() => {
  const t = menuTarget.value;
  if (t === null) return [];
  return [
    { header: true, label: t.name },
    {
      label: 'Rename',
      icon: 'pencil',
      action: () => startEdit({ kind: 'rename-view', viewId: t.id, filters: t.filters }, t.name, true),
    },
    { sep: true },
    { label: 'Delete', icon: 'trash-2', danger: true, action: () => void removeView(t.id) },
  ];
});

function openViewMenu(event: MouseEvent, v: TaskViewDto): void {
  menuTarget.value = v;
  openAt(event);
}

async function removeView(id: string): Promise<void> {
  if (ws.value === '') return;
  const ok = await taskViews.remove(ws.value, id);
  if (!ok && taskViews.error) ui.showBanner(taskViews.error, 'error');
  if (props.activeViewId === id) void router.push({ name: 'tasks' });
}

function loadViews(): void {
  if (ws.value !== '') void taskViews.load(ws.value);
}

onMounted(loadViews);
watch(ws, loadViews);
</script>

<template>
  <div>
    <SectionLabel>Views</SectionLabel>

    <button
      v-for="view in PREDEFINED_VIEWS"
      :key="view.viewId"
      type="button"
      class="atl-row views-row"
      :class="{ 'views-row--active': activeViewId === view.viewId }"
      @click="openView(view.viewId)"
    >
      <span style="width: 12px; flex: 0 0 auto;" />
      <Icon
        :name="view.icon"
        :size="13"
        :style="{ color: view.agent ? 'var(--c-agent)' : 'var(--c-muted)', flexShrink: 0 }"
      />
      <span class="views-label">{{ view.label }}</span>
    </button>

    <template v-for="v in taskViews.items" :key="v.id">
      <div
        v-if="editActive?.kind === 'rename-view' && editActive.viewId === v.id"
        style="display: flex; align-items: center; gap: 6px; padding: 3px 8px;"
      >
        <Icon name="layout-list" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
        <input
          ref="inputRef"
          v-model="editValue"
          type="text"
          placeholder="View name…"
          class="views-inline-input"
          @keydown="onEditKeydown"
          @blur="commitEdit"
        />
      </div>
      <Row
        v-else
        :label="v.name"
        icon="layout-list"
        menu
        :active="activeViewId === v.id"
        @click="openView(v.id)"
        @menu="(event: MouseEvent) => openViewMenu(event, v)"
        @contextmenu.prevent.stop="(event: MouseEvent) => openViewMenu(event, v)"
      />
    </template>

    <ContextMenu :open="menuOpen" :x="menuX" :y="menuY" :items="menuItems" @close="closeMenu" />
  </div>
</template>

<style scoped>
.views-row {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  height: 24px;
  padding: 0 8px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-sm);
  font-weight: var(--fw-medium);
  color: var(--c-foreground);
  text-align: left;
}

.views-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.views-row--active {
  background: var(--c-selection);
  color: var(--c-primary);
}

.views-inline-input {
  flex: 1;
  height: 28px;
  padding: 0 6px;
  background: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  font-size: var(--fs-sm);
  font-family: var(--font-mono);
  color: var(--c-foreground);
  outline: none;
}
</style>
