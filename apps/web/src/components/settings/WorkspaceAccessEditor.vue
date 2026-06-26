<script setup lang="ts">
import { computed, ref } from 'vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';

export interface WorkspaceRef {
  slug: string;
  name: string;
}

export interface RoleOption {
  value: string;
  label: string;
}

/**
 * Inline editor that assigns a principal's role across a fixed list of
 * workspaces, addressing each by slug so the active workspace never has to
 * switch. The parent owns the data: `roles` is the current `slug -> role`
 * lookup and the component emits `assign` / `remove` for the parent to persist.
 *
 * Removing access (role -> None) is gated behind a confirmation here so the
 * destructive intent is explained before the parent acts; assigning or changing
 * a role emits straight away.
 */
const props = withDefaults(
  defineProps<{
    workspaces: WorkspaceRef[];
    roles: Record<string, string>;
    options: RoleOption[];
    noAccessLabel?: string;
  }>(),
  {
    noAccessLabel: 'None',
  },
);

const emit = defineEmits<{
  assign: [slug: string, role: string];
  remove: [slug: string];
}>();

const NONE = '';

const removeTarget = ref<WorkspaceRef | null>(null);

/**
 * The "None" entry (remove access) followed by the assignable roles. The
 * displayed value is driven entirely by `:model-value`, so selecting None never
 * changes the control optimistically — it only opens the remove confirmation.
 */
const roleOptions = computed<DropdownOption[]>(() => [
  { value: NONE, label: props.noAccessLabel },
  ...props.options,
]);

function roleOf(slug: string): string {
  return props.roles[slug] ?? NONE;
}

function onChange(ws: WorkspaceRef, next: string): void {
  const current = roleOf(ws.slug);
  if (next === current) return;

  if (next === NONE) {
    removeTarget.value = ws;
    return;
  }

  emit('assign', ws.slug, next);
}

function confirmRemove(): void {
  const target = removeTarget.value;
  removeTarget.value = null;
  if (target === null) return;

  emit('remove', target.slug);
}

function cancelRemove(): void {
  removeTarget.value = null;
}
</script>

<template>
  <div>
    <div v-if="workspaces.length === 0" class="atl-wsa-empty">
      <Icon name="building-2" :size="13" style="color: var(--c-muted);" />
      <span>No workspaces to assign yet.</span>
    </div>

    <div v-else class="atl-wsa-list">
      <div v-for="ws in workspaces" :key="ws.slug" class="atl-wsa-row" data-wsa-row>
        <Icon name="building-2" :size="13" style="color: var(--c-muted); flex: 0 0 auto;" />
        <span class="atl-wsa-name">{{ ws.name }}</span>

        <div class="atl-wsa-control" data-wsa-role>
          <Dropdown
            :options="roleOptions"
            :model-value="roleOf(ws.slug)"
            @change="(value) => onChange(ws, value)"
          />
        </div>
      </div>
    </div>

    <ConfirmDialog
      :open="removeTarget !== null"
      tone="danger"
      title="Remove workspace access?"
      :message="
        removeTarget
          ? `They lose access to ${removeTarget.name} and everything in it until re-added.`
          : undefined
      "
      :detail="removeTarget ? removeTarget.name : undefined"
      detail-icon="building-2"
      confirm-label="Remove access"
      confirm-icon="user-minus"
      @confirm="confirmRemove"
      @cancel="cancelRemove"
    />
  </div>
</template>

<style scoped>
.atl-wsa-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.atl-wsa-row {
  display: flex;
  align-items: center;
  gap: 7px;
  min-height: 32px;
  padding: 4px 6px;
  border-radius: var(--r-md);
}

.atl-wsa-row:hover {
  background: var(--c-raised);
}

.atl-wsa-name {
  flex: 1;
  min-width: 0;
  font-size: 12.5px;
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-wsa-control {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
}

.atl-wsa-empty {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  color: var(--c-muted);
  padding: 4px 0;
}
</style>
