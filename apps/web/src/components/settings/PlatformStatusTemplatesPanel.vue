<script setup lang="ts">
import { onMounted } from 'vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import StatusTemplateList, {
  type StatusTemplatePlacement,
} from '@/components/settings/StatusTemplateList.vue';
import { usePlatformStatusTemplatesStore } from '@/stores/platformStatusTemplates';
import { useUiStore } from '@/stores/ui';

/**
 * Administration > Default statuses. Manages the Atlas-wide list every newly
 * created workspace is seeded from. Editing it never touches an existing
 * workspace — those keep the status templates they were created with, and each
 * workspace can still edit its own under Workspace > Default statuses.
 */

const store = usePlatformStatusTemplatesStore();
const ui = useUiStore();

onMounted(async () => {
  await store.load();
  if (store.error !== null) ui.showBanner(store.error, 'error');
});

async function addTemplate(name: string): Promise<void> {
  const created = await store.create(name);
  if (created) ui.showBanner('Default status added', 'success');
  else if (store.error !== null) ui.showBanner(store.error, 'error');
}

async function saveEdit(id: string, patch: { name?: string; color?: string }): Promise<void> {
  const ok = await store.update(id, patch);
  if (ok) ui.showBanner('Default status updated', 'success');
  else if (store.error !== null) ui.showBanner(store.error, 'error');
}

async function move(id: string, placement: StatusTemplatePlacement): Promise<void> {
  const ok = await store.move(id, placement);
  if (!ok && store.error !== null) ui.showBanner(store.error, 'error');
}

async function removeTemplate(id: string): Promise<void> {
  const ok = await store.remove(id);
  if (ok) ui.showBanner('Default status deleted', 'success');
  else if (store.error !== null) ui.showBanner(store.error, 'error');
}
</script>

<template>
  <div>
    <PanelHeader
      title="Default statuses"
      subtitle="Statuses every new workspace starts with. Existing workspaces keep their own."
    />

    <StatusTemplateList
      :templates="store.templates"
      empty-label="No Atlas default statuses yet. Add one below, or new workspaces fall back to To Do / In Progress / Done."
      delete-title="Delete this Atlas default status?"
      delete-message="New workspaces stop being seeded with it. Existing workspaces are untouched."
      @create="addTemplate"
      @update="saveEdit"
      @move="move"
      @remove="removeTemplate"
    />
  </div>
</template>
