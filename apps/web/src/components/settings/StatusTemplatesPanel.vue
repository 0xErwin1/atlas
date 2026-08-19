<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import StatusTemplateList, {
  type StatusTemplatePlacement,
} from '@/components/settings/StatusTemplateList.vue';
import Btn from '@/components/ui/Btn.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import { useBoardsStore } from '@/stores/boards';
import { useStatusTemplatesStore } from '@/stores/statusTemplates';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

/**
 * Workspace > Default statuses. Manages the workspace-level status templates new
 * boards are seeded from. The editable list itself is the shared
 * `StatusTemplateList`; this panel adds the workspace-only "Apply to a board"
 * affordance that copies the templates into an existing board's columns.
 */

const workspace = useWorkspaceStore();
const templatesStore = useStatusTemplatesStore();
const boards = useBoardsStore();
const ui = useUiStore();

const ws = computed(() => workspace.activeWorkspaceSlug);

const selectedBoardId = ref<string>('');
const applying = ref(false);

/** Every board in the workspace, grouped by project, for the apply-to-board picker. */
const boardOptions = computed<DropdownOption[]>(() =>
  workspace.projects.flatMap((project) =>
    boards.boardsFor(project.slug).map((board) => ({
      value: board.id,
      label: `${project.name} · ${board.name}`,
    })),
  ),
);

async function loadAll(): Promise<void> {
  const slug = ws.value;
  if (slug === null) return;

  await templatesStore.load(slug);
  await workspace.loadProjects(slug);
  await Promise.all(workspace.projects.map((p) => boards.loadBoardsForProject(slug, p.slug)));
}

watch(ws, () => {
  selectedBoardId.value = '';
  void loadAll();
});

onMounted(loadAll);

async function addTemplate(name: string): Promise<void> {
  const slug = ws.value;
  if (slug === null) return;

  const created = await templatesStore.create(slug, name);
  if (created) ui.showBanner('Status added', 'success');
  else if (templatesStore.error !== null) ui.showBanner(templatesStore.error, 'error');
}

async function saveEdit(id: string, patch: { name?: string; color?: string }): Promise<void> {
  const slug = ws.value;
  if (slug === null) return;

  const ok = await templatesStore.update(slug, id, patch);
  if (ok) ui.showBanner('Status updated', 'success');
  else if (templatesStore.error !== null) ui.showBanner(templatesStore.error, 'error');
}

async function move(id: string, placement: StatusTemplatePlacement): Promise<void> {
  const slug = ws.value;
  if (slug === null) return;

  const ok = await templatesStore.move(slug, id, placement);
  if (!ok && templatesStore.error !== null) ui.showBanner(templatesStore.error, 'error');
}

async function removeTemplate(id: string): Promise<void> {
  const slug = ws.value;
  if (slug === null) return;

  const ok = await templatesStore.remove(slug, id);
  if (ok) ui.showBanner('Status deleted', 'success');
  else if (templatesStore.error !== null) ui.showBanner(templatesStore.error, 'error');
}

async function applyToBoard(): Promise<void> {
  const slug = ws.value;
  if (slug === null || selectedBoardId.value === '') return;

  const boardLabel = boardOptions.value.find((o) => o.value === selectedBoardId.value)?.label ?? 'board';

  applying.value = true;
  const ok = await templatesStore.applyToBoard(slug, selectedBoardId.value);
  applying.value = false;

  if (ok) ui.showBanner(`Statuses applied to ${boardLabel}`, 'success');
  else if (templatesStore.error !== null) ui.showBanner(templatesStore.error, 'error');
}
</script>

<template>
  <div>
    <PanelHeader
      title="Default statuses"
      subtitle="Default statuses new boards start with; apply them to an existing board below."
    />

    <StatusTemplateList
      :templates="templatesStore.templates"
      empty-label="No default statuses yet. Add one below."
      delete-title="Delete this default status?"
      delete-message="It is removed from the workspace defaults. Boards already using it keep their column."
      @create="addTemplate"
      @update="saveEdit"
      @move="move"
      @remove="removeTemplate"
    />

    <div class="atl-apply-section">
      <div class="atl-apply-title">Apply to a board</div>
      <div class="atl-apply-sub">Adds any missing default statuses to an existing board. Existing statuses are left untouched.</div>
      <div class="atl-apply-row">
        <Dropdown
          :model-value="selectedBoardId"
          :options="boardOptions"
          placeholder="Pick a board…"
          icon="kanban"
          @update:model-value="(id) => { selectedBoardId = id; }"
        />
        <Btn
          variant="primary"
          :disabled="selectedBoardId === '' || applying || templatesStore.templates.length === 0"
          @click="applyToBoard"
        >
          Apply
        </Btn>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-apply-section {
  margin-top: 28px;
  padding-top: 20px;
  border-top: 1px solid var(--c-border);
  max-width: 560px;
}

.atl-apply-title {
  font-size: var(--fs-base);
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-apply-sub {
  font-size: var(--fs-sm);
  color: var(--c-muted);
  margin-top: 3px;
}

.atl-apply-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 12px;
}
</style>
