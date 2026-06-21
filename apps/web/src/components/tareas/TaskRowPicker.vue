<script setup lang="ts">
/**
 * Inline, anchored picker used inside task-list rows (status / priority /
 * assignee). The trigger is the row cell itself; clicking it opens a teleported
 * menu of options so the surface escapes the list's scrolling, clipping
 * container. Selecting an option emits `pick` with the option's value.
 *
 * The trigger is a non-button span (the row itself is a <button>, so nesting an
 * interactive button would be invalid) and stops click propagation so opening a
 * picker never also selects or opens the row. The host closes every open picker
 * on list scroll, since the fixed panel does not follow the trigger.
 */
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';

export interface PickerOption {
  value: string;
  label: string;
  /** Optional leading dot color (status columns / priority flags). */
  color?: string;
  /** Optional lucide icon name shown before the label. */
  icon?: string;
  /** Marks the option currently applied to the task (renders a check). */
  active?: boolean;
  /** Renders the option in the muted "clear" style. */
  muted?: boolean;
}

defineProps<{
  options: PickerOption[];
  /** Width of the teleported panel (CSS length). */
  width?: string;
}>();

const open = defineModel<boolean>('open', { default: false });

const emit = defineEmits<{ pick: [value: string] }>();

function choose(option: PickerOption, close: () => void): void {
  close();
  emit('pick', option.value);
}
</script>

<template>
  <Popover
    v-model:open="open"
    teleport
    placement="bottom-start"
    :width="width ?? '200px'"
  >
    <template #trigger="{ toggle }">
      <span class="atl-rp-trigger" @click.stop="toggle()">
        <slot name="trigger" />
      </span>
    </template>

    <template #default="{ close }">
      <div class="atl-rp-menu" @click.stop>
        <button
          v-for="option in options"
          :key="option.value"
          type="button"
          class="atl-rp-item"
          :class="{ muted: option.muted }"
          @click.stop="choose(option, close)"
        >
          <span class="atl-rp-lead">
            <span
              v-if="option.color"
              class="atl-rp-dot"
              :style="{ background: option.color }"
            />
            <Icon v-else-if="option.icon" :name="option.icon" :size="14" />
          </span>

          <span class="atl-rp-label">{{ option.label }}</span>

          <span class="atl-rp-check">
            <Icon v-if="option.active" name="check" :size="14" />
          </span>
        </button>
      </div>
    </template>
  </Popover>
</template>

<style scoped>
.atl-rp-trigger {
  display: inline-flex;
  align-items: center;
  cursor: pointer;
}

.atl-rp-menu {
  display: flex;
  flex-direction: column;
  padding: 4px;
  max-height: 60vh;
  overflow-y: auto;
}

.atl-rp-item {
  display: grid;
  grid-template-columns: 18px minmax(0, 1fr) 18px;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: 30px;
  padding: 0 8px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  text-align: left;
  font-size: var(--fs-sm);
  color: var(--c-foreground);
  cursor: pointer;
}

.atl-rp-item:hover {
  background: var(--c-raised);
}

.atl-rp-item.muted {
  color: var(--c-muted);
}

.atl-rp-lead {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--c-muted);
}

.atl-rp-dot {
  width: 9px;
  height: 9px;
  border-radius: var(--r-full);
}

.atl-rp-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-rp-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--c-primary);
}
</style>
