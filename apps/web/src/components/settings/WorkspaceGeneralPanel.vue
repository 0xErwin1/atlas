<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { z } from 'zod';
import Btn from '@/components/ui/Btn.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import { validateForm } from '@/lib/validation';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

/**
 * Workspace > General. Renames the active workspace. The slug is never
 * re-derived server-side, so the URL/identity stays stable; only the display
 * name changes and the store's cached workspace reflects it immediately.
 */

const workspace = useWorkspaceStore();
const ui = useUiStore();

const activeWorkspace = computed(() =>
  workspace.workspaces.find((w) => w.slug === workspace.activeWorkspaceSlug),
);

const name = ref(activeWorkspace.value?.name ?? '');
const nameError = ref<string | null>(null);
const saving = ref(false);

watch(activeWorkspace, (ws) => {
  name.value = ws?.name ?? '';
  nameError.value = null;
});

const nameSchema = z.object({ name: z.string().trim().min(1, 'Workspace name is required') });

const dirty = computed(() => name.value.trim() !== (activeWorkspace.value?.name ?? ''));

async function save(): Promise<void> {
  const slug = workspace.activeWorkspaceSlug;
  if (slug === null) return;

  nameError.value = null;
  const result = validateForm(nameSchema, { name: name.value });
  if (!result.ok) {
    nameError.value = result.errors.name ?? null;
    return;
  }

  saving.value = true;
  const ok = await workspace.renameWorkspace(slug, result.data.name);
  saving.value = false;

  if (ok) ui.showBanner('Workspace renamed', 'success');
  else if (workspace.error) ui.showBanner(workspace.error, 'error');
}
</script>

<template>
  <div>
    <div class="atl-panel-head">
      <div class="atl-panel-title">General</div>
      <div class="atl-panel-sub">Rename this workspace — its URL slug stays the same</div>
    </div>

    <div class="flex flex-col" style="gap: 14px; max-width: 430px;">
      <FormField
        label="Workspace name"
        :model-value="name"
        :error="nameError"
        @update:model-value="(v) => { name = v; nameError = null; }"
      />
      <div v-if="activeWorkspace" class="atl-slug-note">
        <Icon name="link" :size="13" style="color: var(--c-muted);" />
        <span>Slug: <code>{{ activeWorkspace.slug }}</code></span>
      </div>
    </div>

    <div class="flex" style="gap: 8px; margin-top: 20px;">
      <Btn variant="primary" :disabled="saving || !dirty" @click="save">
        <Icon name="check" :size="14" />Save changes
      </Btn>
    </div>
  </div>
</template>

<style scoped>
.atl-panel-head {
  margin-bottom: 16px;
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

.atl-slug-note {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  color: var(--c-muted);
}

.atl-slug-note code {
  font-family: var(--font-mono);
  color: var(--c-foreground);
}
</style>
