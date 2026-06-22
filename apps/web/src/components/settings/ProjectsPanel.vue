<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import Btn from '@/components/ui/Btn.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

/**
 * Workspace > Projects. Lists every project in the active workspace and lets
 * editors rename each one or change its task prefix. The prefix validation
 * mirrors the server-side rule ^[A-Z][A-Z0-9]{1,9}$ and must stay in sync with
 * the backend's validate_task_prefix. The server is still the authority — 409
 * (duplicate prefix) and 422 (bad format) surface as error banners so the user
 * knows to correct the input.
 */

const TASK_PREFIX_RE = /^[A-Z][A-Z0-9]{1,9}$/;

const workspace = useWorkspaceStore();
const ui = useUiStore();

const ws = computed(() => workspace.activeWorkspaceSlug);

const editingSlug = ref<string | null>(null);
const draftName = ref('');
const draftPrefix = ref('');
const nameError = ref<string | null>(null);
const prefixError = ref<string | null>(null);
const saving = ref(false);

watch(ws, (slug) => {
  if (slug !== null) void workspace.loadProjects(slug);
  cancelEdit();
});

onMounted(() => {
  if (ws.value !== null) void workspace.loadProjects(ws.value);
});

function startEdit(slug: string): void {
  const project = workspace.projects.find((p) => p.slug === slug);
  if (project === undefined) return;

  editingSlug.value = slug;
  draftName.value = project.name;
  draftPrefix.value = project.task_prefix;
  nameError.value = null;
  prefixError.value = null;
}

function cancelEdit(): void {
  editingSlug.value = null;
  draftName.value = '';
  draftPrefix.value = '';
  nameError.value = null;
  prefixError.value = null;
}

function validateDraft(): boolean {
  let valid = true;

  const name = draftName.value.trim();
  if (name === '') {
    nameError.value = 'Project name is required';
    valid = false;
  } else {
    nameError.value = null;
  }

  const prefix = draftPrefix.value.trim();
  if (prefix === '') {
    prefixError.value = 'Task prefix is required';
    valid = false;
  } else if (!TASK_PREFIX_RE.test(prefix)) {
    prefixError.value = 'Must be 2–10 characters: start with a letter, then letters and digits only';
    valid = false;
  } else {
    prefixError.value = null;
  }

  return valid;
}

async function saveEdit(slug: string): Promise<void> {
  const wsSlug = ws.value;
  if (wsSlug === null) return;

  if (!validateDraft()) return;

  const project = workspace.projects.find((p) => p.slug === slug);
  if (project === undefined) return;

  const patch: { name?: string; task_prefix?: string } = {};
  const name = draftName.value.trim();
  const prefix = draftPrefix.value.trim();

  if (name !== project.name) patch.name = name;
  if (prefix !== project.task_prefix) patch.task_prefix = prefix;

  if (patch.name === undefined && patch.task_prefix === undefined) {
    cancelEdit();
    return;
  }

  saving.value = true;
  const ok = await workspace.updateProject(wsSlug, slug, patch);
  saving.value = false;

  if (ok) {
    ui.showBanner('Project updated', 'success');
    cancelEdit();
  } else if (workspace.error !== null) {
    ui.showBanner(workspace.error, 'error');
  }
}
</script>

<template>
  <div>
    <div class="atl-panel-head">
      <div class="atl-panel-title">Projects</div>
      <div class="atl-panel-sub">Rename a project or change the prefix used when generating task IDs</div>
    </div>

    <div v-if="workspace.projects.length === 0" class="atl-proj-empty">
      No projects in this workspace yet.
    </div>

    <div v-else class="atl-proj-list">
      <div
        v-for="project in workspace.projects"
        :key="project.slug"
        class="atl-proj-row"
      >
        <template v-if="editingSlug === project.slug">
          <div class="atl-proj-edit">
            <FormField
              label="Name"
              :model-value="draftName"
              :error="nameError"
              @update:model-value="(v) => { draftName = v; nameError = null; }"
              @keydown="(e) => { if (e.key === 'Enter') void saveEdit(project.slug); if (e.key === 'Escape') cancelEdit(); }"
            />

            <FormField
              label="Task prefix"
              :model-value="draftPrefix"
              :error="prefixError"
              mono
              @update:model-value="(v) => { draftPrefix = v.toUpperCase(); prefixError = null; }"
              @keydown="(e) => { if (e.key === 'Enter') void saveEdit(project.slug); if (e.key === 'Escape') cancelEdit(); }"
            />

            <div class="atl-prefix-note">
              <Icon name="info" :size="12" style="color: var(--c-muted); flex: none;" />
              <span>Changing the prefix only affects new task IDs — existing IDs keep their prefix.</span>
            </div>

            <div class="atl-proj-edit-actions">
              <Btn variant="primary" :disabled="saving" @click="saveEdit(project.slug)">
                <Icon name="check" :size="14" />Save
              </Btn>
              <button type="button" class="atl-rowact" @click="cancelEdit">Cancel</button>
            </div>
          </div>
        </template>

        <template v-else>
          <div class="atl-proj-info">
            <span class="atl-proj-name">{{ project.name }}</span>
            <code class="atl-proj-prefix">{{ project.task_prefix }}</code>
          </div>
          <button
            type="button"
            class="atl-rowact"
            title="Edit project"
            @click="startEdit(project.slug)"
          >
            <Icon name="pencil" :size="13" />
          </button>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-panel-head {
  margin-bottom: 20px;
}

.atl-panel-title {
  font-size: 15px;
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
}

.atl-panel-sub {
  font-size: 12px;
  color: var(--c-muted);
  margin-top: 3px;
}

.atl-proj-empty {
  font-size: 13px;
  color: var(--c-muted);
  padding: 8px 2px;
}

.atl-proj-list {
  border: 1px solid var(--c-border);
  border-radius: 4px;
  overflow: hidden;
  max-width: 560px;
}

.atl-proj-row {
  border-top: 1px solid var(--c-border);
  padding: 0 12px;
}

.atl-proj-row:first-child {
  border-top: none;
}

.atl-proj-row:not(:has(.atl-proj-edit)) {
  display: flex;
  align-items: center;
  height: 48px;
  gap: 8px;
}

.atl-proj-info {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: 1;
  min-width: 0;
}

.atl-proj-name {
  font-size: 13px;
  color: var(--c-foreground);
  font-weight: var(--fw-medium);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-proj-prefix {
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--c-muted);
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  padding: 1px 5px;
  flex-shrink: 0;
}

.atl-proj-edit {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 14px 0;
  max-width: 340px;
}

.atl-prefix-note {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  font-size: 11.5px;
  color: var(--c-muted);
  line-height: 1.4;
}

.atl-proj-edit-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.atl-rowact {
  display: inline-flex;
  align-items: center;
  height: 24px;
  padding: 0 8px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-foreground);
  cursor: pointer;
  font-size: 12px;
}

.atl-rowact:hover {
  background: var(--c-raised);
}
</style>
