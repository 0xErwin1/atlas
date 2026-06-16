<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { type GrantRole, isRoleAllowedFor } from '@/lib/grantRoles';

const props = defineProps<{
  principalType: string;
  role: string;
}>();

const emit = defineEmits<{
  select: [role: GrantRole];
  remove: [];
}>();

interface RoleRow {
  value: GrantRole;
  label: string;
  icon: string;
}

const ROWS: RoleRow[] = [
  { value: 'viewer', label: 'Viewer', icon: 'eye' },
  { value: 'editor', label: 'Editor', icon: 'type' },
  { value: 'admin', label: 'Admin', icon: 'settings' },
];

const isAgent = computed(() => props.principalType !== 'user');

function allowed(role: GrantRole): boolean {
  return isRoleAllowedFor(props.principalType, role);
}

function choose(role: GrantRole) {
  if (!allowed(role)) return;
  emit('select', role);
}
</script>

<template>
  <div
    role="menu"
    style="
      position: absolute;
      top: 30px;
      right: 0;
      width: 200px;
      background-color: var(--c-panel);
      border: 1px solid var(--c-border);
      border-radius: var(--r-lg);
      box-shadow: var(--shadow-lg);
      padding: 4px;
      z-index: 5;
    "
  >
    <button
      v-for="row in ROWS"
      :key="row.value"
      type="button"
      role="menuitemradio"
      :data-role-option="row.value"
      :data-selectable-role="allowed(row.value) ? row.value : undefined"
      :aria-checked="props.role === row.value"
      :aria-disabled="!allowed(row.value)"
      :disabled="!allowed(row.value)"
      class="flex items-center gap-2 w-full text-left"
      :style="{
        height: '28px',
        padding: '0 9px',
        border: 'none',
        borderRadius: 'var(--r-md)',
        fontSize: 'var(--fs-sm)',
        cursor: allowed(row.value) ? 'pointer' : 'not-allowed',
        color: 'var(--c-foreground)',
        background: props.role === row.value ? 'var(--c-selection)' : 'transparent',
        boxShadow: props.role === row.value ? 'inset 2px 0 0 var(--c-primary)' : 'none',
        opacity: allowed(row.value) ? 1 : 0.55,
        fontWeight: props.role === row.value ? 'var(--fw-semibold)' : 'var(--fw-medium)',
      }"
      @click="choose(row.value)"
    >
      <Icon
        :name="!allowed(row.value) && row.value === 'admin' ? 'lock' : row.icon"
        :size="13"
      />
      <span class="flex-1">{{ row.label }}</span>
      <span
        v-if="!allowed(row.value) && row.value === 'admin'"
        style="font-size: 10px; font-weight: var(--fw-bold); font-family: var(--font-mono); color: var(--c-muted);"
      >
        Editor max
      </span>
      <Icon
        v-else-if="props.role === row.value"
        name="check"
        :size="13"
        :style="{ color: 'var(--c-primary)' }"
      />
    </button>

    <p
      v-if="isAgent"
      style="
        font-size: var(--fs-xs);
        color: var(--c-muted);
        padding: 3px 9px 5px;
        line-height: 1.35;
        margin: 0;
      "
    >
      Agents &amp; scripts can’t be Admin or manage grants (v2).
    </p>

    <div style="height: 1px; background-color: var(--c-border); margin: 4px 2px;" />

    <button
      type="button"
      role="menuitem"
      data-action="remove"
      class="flex items-center gap-2 w-full text-left"
      style="
        height: 28px;
        padding: 0 9px;
        border: none;
        border-radius: var(--r-md);
        font-size: var(--fs-sm);
        font-weight: var(--fw-medium);
        cursor: pointer;
        background: transparent;
        color: var(--c-danger);
      "
      @click="emit('remove')"
    >
      <Icon name="x" :size="13" />
      <span class="flex-1">Remove</span>
    </button>
  </div>
</template>
