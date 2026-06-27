<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { availableRolesFor, type GrantRole, grantRoleIcon, grantRoleLabel } from '@/lib/grantRoles';

const props = defineProps<{
  principalType: string;
  role: string;
}>();

const emit = defineEmits<{
  select: [role: GrantRole];
  remove: [];
}>();

// Agents (api_key / unknown) are capped at editor; admin is never offered to
// them. `availableRolesFor` owns that cap, the label/icon maps own the display.
const rows = computed(() =>
  availableRolesFor(props.principalType).map((value) => ({
    value,
    label: grantRoleLabel(value),
    icon: grantRoleIcon(value),
  })),
);

const isAgent = computed(() => props.principalType !== 'user' && props.principalType !== 'group');

function choose(role: GrantRole) {
  emit('select', role);
}
</script>

<template>
  <div style="padding: 4px;">
    <button
      v-for="row in rows"
      :key="row.value"
      type="button"
      role="menuitemradio"
      :data-role-option="row.value"
      :data-selectable-role="row.value"
      :aria-checked="props.role === row.value"
      class="atl-row flex items-center gap-2 w-full text-left"
      :style="{
        height: '28px',
        padding: '0 9px',
        border: 'none',
        borderRadius: 'var(--r-md)',
        fontSize: 'var(--fs-sm)',
        cursor: 'pointer',
        color: 'var(--c-foreground)',
        background: props.role === row.value ? 'var(--c-selection)' : 'transparent',
        boxShadow: props.role === row.value ? 'inset 2px 0 0 var(--c-primary)' : 'none',
        fontWeight: props.role === row.value ? 'var(--fw-semibold)' : 'var(--fw-medium)',
      }"
      @click="choose(row.value)"
    >
      <Icon :name="row.icon" :size="13" />
      <span class="flex-1">{{ row.label }}</span>
      <Icon
        v-if="props.role === row.value"
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
      Agents &amp; scripts are capped at Editor max — they can’t be Admin or manage grants (v2).
    </p>

    <div style="height: 1px; background-color: var(--c-border); margin: 4px 2px;" />

    <button
      type="button"
      role="menuitem"
      data-action="remove"
      class="atl-row flex items-center gap-2 w-full text-left"
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
