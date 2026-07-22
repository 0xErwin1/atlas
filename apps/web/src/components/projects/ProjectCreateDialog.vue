<script setup lang="ts">
import { ref } from 'vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ cancel: []; created: [slug: string] }>();
const workspace = useWorkspaceStore();
const error = ref('');
const creating = ref(false);

async function submit(name: string): Promise<void> {
  const workspaceSlug = workspace.activeWorkspaceSlug;
  if (workspaceSlug === null) return;
  if (name.trim() === '') {
    error.value = 'Project name is required';
    return;
  }

  creating.value = true;
  const slug = await workspace.createProject(workspaceSlug, name.trim());
  creating.value = false;
  if (slug === null) {
    error.value = workspace.error ?? 'Failed to create project';
    return;
  }

  error.value = '';
  emit('created', slug);
}
</script>

<template>
  <PromptDialog
    :open="props.open"
    title="New project"
    placeholder="Project name"
    confirm-label="Create"
    :error="error"
    @confirm="submit"
    @cancel="error = ''; emit('cancel')"
  />
</template>
