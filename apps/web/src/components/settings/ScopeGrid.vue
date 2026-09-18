<script setup lang="ts">
import { computed } from 'vue';
import SquareCheckbox from '@/components/ui/SquareCheckbox.vue';
import type { ApiKeyScope } from '@/stores/apiKeys';

const ACTIONS = ['read', 'create', 'update', 'delete'] as const;

type Action = (typeof ACTIONS)[number];

/**
 * A capability family and the subset of actions it actually exposes. The
 * catalog is asymmetric: most families cover all four CRUD actions, but
 * `grants` is read-only. Every value in `cells` is typed as `ApiKeyScope`, so
 * the compiler rejects any token absent from the generated union (e.g.
 * `custos::grants::create`) — the grid can only ever emit valid scopes.
 */
type ScopeRow = {
  family: string;
  label?: string;
  cells: Partial<Record<Action, ApiKeyScope>>;
};

const SCOPE_GRID: readonly ScopeRow[] = [
  {
    family: 'tasks',
    cells: {
      read: 'acta::tasks::read',
      create: 'acta::tasks::create',
      update: 'acta::tasks::update',
      delete: 'acta::tasks::delete',
    },
  },
  {
    family: 'docs',
    cells: {
      read: 'acta::docs::read',
      create: 'acta::docs::create',
      update: 'acta::docs::update',
      delete: 'acta::docs::delete',
    },
  },
  {
    family: 'boards',
    cells: {
      read: 'acta::boards::read',
      create: 'acta::boards::create',
      update: 'acta::boards::update',
      delete: 'acta::boards::delete',
    },
  },
  {
    family: 'folders',
    cells: {
      read: 'acta::folders::read',
      create: 'acta::folders::create',
      update: 'acta::folders::update',
      delete: 'acta::folders::delete',
    },
  },
  {
    family: 'projects',
    cells: {
      read: 'acta::projects::read',
      create: 'acta::projects::create',
      update: 'acta::projects::update',
      delete: 'acta::projects::delete',
    },
  },
  {
    family: 'webhooks',
    cells: {
      read: 'acta::webhooks::read',
      create: 'acta::webhooks::create',
      update: 'acta::webhooks::update',
      delete: 'acta::webhooks::delete',
    },
  },
  {
    family: 'config',
    cells: {
      read: 'acta::config::read',
      create: 'acta::config::create',
      update: 'acta::config::update',
      delete: 'acta::config::delete',
    },
  },
  {
    family: 'grants',
    cells: { read: 'custos::grants::read' },
  },
  {
    family: 'saved_searches',
    label: 'saved searches',
    cells: {
      read: 'acta::saved_searches::read',
      create: 'acta::saved_searches::create',
      update: 'acta::saved_searches::update',
      delete: 'acta::saved_searches::delete',
    },
  },
  {
    family: 'task_views',
    label: 'task views',
    cells: {
      read: 'acta::task_views::read',
      create: 'acta::task_views::create',
      update: 'acta::task_views::update',
      delete: 'acta::task_views::delete',
    },
  },
];

const props = withDefaults(defineProps<{ modelValue: ApiKeyScope[]; disabled?: boolean }>(), {
  disabled: false,
});

const emit = defineEmits<{ 'update:modelValue': [value: ApiKeyScope[]] }>();

const selected = computed(() => new Set<ApiKeyScope>(props.modelValue));

/**
 * Expands the catalog into fixed four-column rows aligned to `ACTIONS`. A
 * column is a scope when the family supports that action, otherwise `null`,
 * which renders as an inert cell instead of a checkbox.
 */
const rows = computed(() =>
  SCOPE_GRID.map((row) => ({
    family: row.family,
    label: row.label ?? row.family,
    columns: ACTIONS.map((action) => {
      const scope = row.cells[action];
      return scope ? { scope, action } : null;
    }),
  })),
);

function isChecked(scope: ApiKeyScope): boolean {
  return selected.value.has(scope);
}

/**
 * Rebuilds the selection in canonical family×action order so the emitted list
 * is deterministic regardless of the order cells were toggled in.
 */
function toggle(scope: ApiKeyScope): void {
  if (props.disabled) return;

  const next = new Set(selected.value);

  if (next.has(scope)) next.delete(scope);
  else next.add(scope);

  const ordered: ApiKeyScope[] = [];
  for (const row of SCOPE_GRID) {
    for (const action of ACTIONS) {
      const s = row.cells[action];
      if (s && next.has(s)) ordered.push(s);
    }
  }

  emit('update:modelValue', ordered);
}
</script>

<template>
  <div class="atl-scope-grid" data-scope-grid>
    <div class="atl-scope-head">
      <div class="atl-scope-corner"></div>
      <div v-for="a in ACTIONS" :key="a" class="atl-scope-action">{{ a }}</div>
    </div>

    <div v-for="row in rows" :key="row.family" class="atl-scope-row" data-scope-row>
      <div class="atl-scope-family">{{ row.label }}</div>
      <template v-for="(col, i) in row.columns" :key="i">
        <label v-if="col" class="atl-scope-cell">
          <SquareCheckbox
            :data-scope="col.scope"
            :model-value="isChecked(col.scope)"
            :label="`${row.label}: ${col.action} capability`"
            :disabled="disabled"
            @update:model-value="toggle(col.scope)"
          />
        </label>
        <div v-else class="atl-scope-cell atl-scope-cell--empty" aria-hidden="true"></div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.atl-scope-grid {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--c-border);
  overflow: hidden;
}

.atl-scope-head,
.atl-scope-row {
  display: grid;
  grid-template-columns: minmax(72px, 1.2fr) repeat(4, 1fr);
  align-items: center;
}

.atl-scope-head {
  background: var(--c-raised);
  border-bottom: 1px solid var(--c-border);
}

.atl-scope-row + .atl-scope-row {
  border-top: 1px solid var(--c-border);
}

.atl-scope-corner {
  height: 26px;
}

.atl-scope-action {
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--fs-label);
  font-weight: var(--fw-semibold);
  letter-spacing: var(--ls-label);
  font-family: var(--font-mono);
  text-transform: uppercase;
  color: var(--c-muted);
}

.atl-scope-family {
  padding: 0 10px;
  height: 30px;
  display: flex;
  align-items: center;
  font-size: var(--fs-sm);
  font-weight: var(--fw-medium);
  color: var(--c-foreground);
  text-transform: capitalize;
}

.atl-scope-cell {
  height: 30px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.atl-scope-cell--empty {
  cursor: default;
}

</style>
