<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import Btn from '@/components/ui/Btn.vue';
import Chip from '@/components/ui/Chip.vue';
import ColorPicker from '@/components/ui/ColorPicker.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import { swatchById } from '@/lib/swatches';
import { useTagsStore } from '@/stores/tags';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

/**
 * Workspace > Tags. The shared tag registry: create, rename, recolor and delete
 * the workspace's tags. Renaming backfills task labels server-side, so no
 * warning is needed. Color is persisted on the tag (a swatch id).
 */

const tagsStore = useTagsStore();
const workspace = useWorkspaceStore();
const ui = useUiStore();

const ws = computed(() => workspace.activeWorkspaceSlug);

const newName = ref('');
const creating = ref(false);

const editingId = ref<string | null>(null);
const draftName = ref('');
const draftColor = ref('');
const renaming = ref(false);

const deleteTargetId = ref<string | null>(null);

const deleteTargetName = computed(
  () => tagsStore.tags.find((t) => t.id === deleteTargetId.value)?.name ?? '',
);

watch(ws, (slug) => {
  if (slug !== null) void tagsStore.load(slug, true);
});

onMounted(() => {
  if (ws.value !== null) void tagsStore.load(ws.value);
});

async function createTag(): Promise<void> {
  const slug = ws.value;
  const name = newName.value.trim();
  if (slug === null || name === '') return;

  creating.value = true;
  const created = await tagsStore.create(slug, name);
  creating.value = false;

  if (created) {
    newName.value = '';
    ui.showBanner('Tag created', 'success');
  } else if (tagsStore.error) {
    ui.showBanner(tagsStore.error, 'error');
  }
}

function startRename(id: string, name: string): void {
  editingId.value = id;
  draftName.value = name;
  draftColor.value = tagsStore.colorFor(name);
}

function cancelRename(): void {
  editingId.value = null;
  draftName.value = '';
  draftColor.value = '';
}

/**
 * Persists the name and color edited together in the row's edit mode. Sends both
 * in a single PATCH; only the changed fields are included so an untouched name
 * or color is left as-is on the server.
 */
async function saveEdit(id: string, current: string): Promise<void> {
  const slug = ws.value;
  if (slug === null) {
    cancelRename();
    return;
  }

  const nextName = draftName.value.trim();
  const patch: { name?: string; color?: string } = {};
  if (nextName !== '' && nextName !== current) patch.name = nextName;
  if (draftColor.value !== tagsStore.colorFor(current)) patch.color = draftColor.value;

  if (patch.name === undefined && patch.color === undefined) {
    cancelRename();
    return;
  }

  renaming.value = true;
  const ok = await tagsStore.update(slug, id, patch);
  renaming.value = false;

  if (ok) {
    ui.showBanner('Tag updated', 'success');
    cancelRename();
  } else if (tagsStore.error) {
    ui.showBanner(tagsStore.error, 'error');
  }
}

async function confirmDelete(): Promise<void> {
  const slug = ws.value;
  const id = deleteTargetId.value;
  deleteTargetId.value = null;
  if (slug === null || id === null) return;

  const ok = await tagsStore.remove(slug, id);
  if (ok) ui.showBanner('Tag deleted', 'success');
  else if (tagsStore.error) ui.showBanner(tagsStore.error, 'error');
}
</script>

<template>
  <div>
    <div class="atl-panel-head">
      <div class="atl-panel-title">Tags</div>
      <div class="atl-panel-sub">The shared tag registry — rename to update every task that uses it</div>
    </div>

    <div class="atl-tag-new">
      <input
        v-model="newName"
        type="text"
        placeholder="New tag name…"
        class="atl-tag-input"
        @keydown.enter="createTag"
      />
      <Btn variant="primary" :disabled="creating || newName.trim() === ''" @click="createTag">
        <Icon name="plus" :size="14" />Add tag
      </Btn>
    </div>

    <div v-if="tagsStore.tags.length === 0" class="atl-tag-empty">
      No tags yet. Create one above.
    </div>

    <div v-else class="atl-tag-list">
      <div v-for="tag in tagsStore.tags" :key="tag.id" class="atl-tag-row">
        <template v-if="editingId === tag.id">
          <Popover placement="bottom-start" teleport>
            <template #trigger="{ toggle }">
              <button type="button" class="atl-color-trigger" title="Pick a color" @click="toggle">
                <span class="atl-dot" :style="{ backgroundColor: swatchById(draftColor).fg }" />
              </button>
            </template>
            <template #default>
              <ColorPicker :selected="draftColor" @select="(id) => { draftColor = id; }" />
            </template>
          </Popover>

          <input
            v-model="draftName"
            type="text"
            class="atl-tag-rename"
            @keydown.enter="saveEdit(tag.id, tag.name)"
            @keydown.esc="cancelRename"
          />
          <span class="flex-1" />
          <Btn variant="primary" :disabled="renaming" @click="saveEdit(tag.id, tag.name)">Save</Btn>
          <button type="button" class="atl-rowact" @click="cancelRename">Cancel</button>
        </template>

        <template v-else>
          <Chip :color="tagsStore.colorFor(tag.name)" icon="dot">{{ tag.name }}</Chip>
          <span class="flex-1" />
          <button
            type="button"
            class="atl-rowact"
            title="Edit name & color"
            @click="startRename(tag.id, tag.name)"
          >
            <Icon name="pencil" :size="13" />
          </button>
          <button
            type="button"
            class="atl-rowact danger"
            title="Delete tag"
            @click="deleteTargetId = tag.id"
          >
            <Icon name="trash" :size="13" />
          </button>
        </template>
      </div>
    </div>

    <ConfirmDialog
      :open="deleteTargetId !== null"
      tone="danger"
      title="Delete this tag?"
      message="It is removed from the registry. Existing task labels keep their text but the tag is no longer suggested."
      :detail="deleteTargetName"
      detail-icon="tag"
      confirm-label="Delete tag"
      confirm-icon="trash"
      @confirm="confirmDelete"
      @cancel="deleteTargetId = null"
    />
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

.atl-tag-new {
  display: flex;
  gap: 8px;
  margin-bottom: 18px;
  max-width: 430px;
}

.atl-tag-input,
.atl-tag-rename {
  height: var(--h-button);
  padding: 0 10px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  font-size: 13px;
  color: var(--c-foreground);
  outline: none;
}

.atl-tag-input {
  flex: 1;
}

.atl-tag-rename {
  width: 200px;
  border-color: var(--c-primary);
}

.atl-tag-empty {
  font-size: 13px;
  color: var(--c-muted);
  padding: 8px 2px;
}

.atl-tag-list {
  border: 1px solid var(--c-border);
  border-radius: 4px;
  overflow: hidden;
}

.atl-tag-row {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 48px;
  padding: 0 12px;
  border-top: 1px solid var(--c-border);
}

.atl-tag-row:first-child {
  border-top: none;
}

.atl-color-trigger {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  background: var(--c-raised);
  cursor: pointer;
}

.atl-color-trigger:hover {
  border-color: var(--c-primary);
}

.atl-dot {
  flex: none;
  width: 9px;
  height: 9px;
  border-radius: var(--r-full);
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

.atl-rowact.danger {
  color: var(--c-danger);
}
</style>
