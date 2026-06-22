<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import Btn from '@/components/ui/Btn.vue';
import Chip from '@/components/ui/Chip.vue';
import ColorPicker from '@/components/ui/ColorPicker.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Icon from '@/components/ui/Icon.vue';
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
const registeringLabel = ref<string | null>(null);

const editingId = ref<string | null>(null);
const draftName = ref('');
const draftColor = ref('');
const renaming = ref(false);

const deleteTargetId = ref<string | null>(null);

const deleteTargetName = computed(
  () => tagsStore.tags.find((t) => t.id === deleteTargetId.value)?.name ?? '',
);

function loadAll(slug: string, force = false): void {
  void tagsStore.load(slug, force);
  void tagsStore.loadUsed(slug, force);
}

watch(ws, (slug) => {
  if (slug !== null) loadAll(slug, true);
});

onMounted(() => {
  if (ws.value !== null) loadAll(ws.value);
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

/**
 * Promotes a usage-derived label into the managed registry. On success the
 * label is now registered, so `unregisteredLabels` drops it and it surfaces in
 * tier 1.
 */
async function registerLabel(label: string): Promise<void> {
  const slug = ws.value;
  if (slug === null || registeringLabel.value !== null) return;

  registeringLabel.value = label;
  const created = await tagsStore.create(slug, label);
  registeringLabel.value = null;

  if (created) ui.showBanner('Tag registered', 'success');
  else if (tagsStore.error) ui.showBanner(tagsStore.error, 'error');
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

    <div
      v-if="tagsStore.tags.length === 0 && tagsStore.unregisteredLabels.length === 0"
      class="atl-tag-empty"
    >
      No tags yet. Create one above.
    </div>

    <template v-else>
      <section class="atl-tag-section">
        <div class="atl-section-head">Registered</div>

        <div v-if="tagsStore.tags.length === 0" class="atl-tag-empty">
          No registered tags yet.
        </div>

        <div v-else class="atl-tag-list">
          <div
            v-for="tag in tagsStore.tags"
            :key="tag.id"
            class="atl-tag-row"
            :class="{ editing: editingId === tag.id }"
          >
            <template v-if="editingId === tag.id">
              <div class="atl-edit-line">
                <span class="atl-dot" :style="{ backgroundColor: swatchById(draftColor).fg }" />
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
              </div>

              <ColorPicker
                class="atl-edit-picker"
                :selected="draftColor"
                @select="(id) => { draftColor = id; }"
              />
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
      </section>

      <section v-if="tagsStore.unregisteredLabels.length > 0" class="atl-tag-section">
        <div class="atl-section-head">Used in tasks — not registered</div>
        <div class="atl-section-hint">
          These labels appear on tasks but aren't in the registry. Register one to manage its
          name and color.
        </div>

        <div class="atl-tag-list atl-used-list">
          <div v-for="label in tagsStore.unregisteredLabels" :key="label" class="atl-tag-row atl-used-row">
            <Chip :color="tagsStore.colorFor(label)" icon="dot" class="atl-used-chip">{{ label }}</Chip>
            <span class="flex-1" />
            <button
              type="button"
              class="atl-rowact atl-register-btn"
              title="Add this label to the registry"
              :disabled="registeringLabel !== null"
              @click="registerLabel(label)"
            >
              <Icon name="plus" :size="13" />Register
            </button>
          </div>
        </div>
      </section>
    </template>

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

.atl-tag-section + .atl-tag-section {
  margin-top: 22px;
}

.atl-section-head {
  font-size: 11px;
  font-weight: var(--fw-bold);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--c-muted);
  margin-bottom: 8px;
}

.atl-section-hint {
  font-size: 12px;
  color: var(--c-muted);
  margin: -2px 0 10px;
  max-width: 430px;
}

.atl-used-list {
  border-style: dashed;
}

.atl-used-row .atl-used-chip {
  opacity: 0.7;
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

.atl-tag-row.editing {
  height: auto;
  flex-direction: column;
  align-items: stretch;
  gap: 10px;
  padding-top: 12px;
  padding-bottom: 14px;
}

.atl-edit-line {
  display: flex;
  align-items: center;
  gap: 8px;
}

.atl-edit-picker {
  align-self: flex-start;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: var(--c-raised);
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
