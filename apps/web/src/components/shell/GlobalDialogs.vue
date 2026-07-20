<script setup lang="ts">
import { computed } from 'vue';
import ShareDialog, { type Visibility } from '@/components/share/ShareDialog.vue';
import BannerToast from '@/components/shell/BannerToast.vue';
import AskAiDialog from '@/components/tareas/AskAiDialog.vue';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

// The workspace-lifetime overlays (banner, share, ask-ai) shared by every app
// shell. Kept in one component so the Docs and Search shells wire them
// identically instead of each re-declaring the share-visibility glue.
const ui = useUiStore();
const workspace = useWorkspaceStore();

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const shareVisibility = computed<Visibility>(() => {
  const projectSlug = ui.shareProjectSlug;
  if (projectSlug === null) return 'workspace';

  const visibility = workspace.projects.find((project) => project.slug === projectSlug)?.visibility;
  return visibility === 'private' || visibility === 'workspace' || visibility === 'public'
    ? visibility
    : 'workspace';
});

async function updateShareVisibility(visibility: Visibility): Promise<void> {
  const projectSlug = ui.shareProjectSlug;
  if (projectSlug === null || ws.value === '') return;

  const ok = await workspace.updateProject(ws.value, projectSlug, { visibility });
  if (!ok && workspace.error !== null) {
    ui.showBanner(workspace.error, 'error');
  }
}
</script>

<template>
  <BannerToast />

  <ShareDialog
    :open="ui.shareOpen"
    :ws="ws"
    :project-slug="ui.shareProjectSlug ?? undefined"
    :resource-label="ui.shareResourceLabel"
    :visibility="shareVisibility"
    @update:visibility="updateShareVisibility"
    @close="ui.closeShare()"
  />

  <AskAiDialog
    :open="ui.askAiOpen"
    :task="ui.askAiTask"
    :status-name="ui.askAiStatus"
    :action="ui.askAiAction"
    @close="ui.closeAskAi()"
  />
</template>
