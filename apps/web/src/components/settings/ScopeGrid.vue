<script setup lang="ts">
import { computed } from 'vue';
import type { ApiKeyScope } from '@/stores/apiKeys';

const ACTIONS = ['read', 'create', 'update', 'delete'] as const;

type Action = (typeof ACTIONS)[number];

/**
 * A capability family and the subset of actions it actually exposes. The
 * catalog is asymmetric: most families cover all four CRUD actions, but
 * `grants` is read-only. Every value in `cells` is typed as `ApiKeyScope`, so
 * the compiler rejects any token absent from the generated union (e.g.
 * `grants:create`) — the grid can only ever emit valid scopes.
 */
type ScopeRow = {
  family: string;
  label?: string;
  cells: Partial<Record<Action, ApiKeyScope>>;
};

const SCOPE_GRID: readonly ScopeRow[] = [
  {
    family: 'tasks',
    cells: { read: 'tasks:read', create: 'tasks:create', update: 'tasks:update', delete: 'tasks:delete' },
  },
  {
    family: 'docs',
    cells: { read: 'docs:read', create: 'docs:create', update: 'docs:update', delete: 'docs:delete' },
  },
  {
    family: 'boards',
    cells: { read: 'boards:read', create: 'boards:create', update: 'boards:update', delete: 'boards:delete' },
  },
  {
    family: 'folders',
    cells: { read: 'folders:read', create: 'folders:create', update: 'folders:update', delete: 'folders:delete' },
  },
  {
    family: 'projects',
    cells: {
      read: 'projects:read',
      create: 'projects:create',
      update: 'projects:update',
      delete: 'projects:delete',
    },
  },
  {
    family: 'webhooks',
    cells: {
      read: 'webhooks:read',
      create: 'webhooks:create',
      update: 'webhooks:update',
      delete: 'webhooks:delete',
    },
  },
  {
    family: 'config',
    cells: { read: 'config:read', create: 'config:create', update: 'config:update', delete: 'config:delete' },
  },
  {
    family: 'grants',
    cells: { read: 'grants:read' },
  },
  {
    family: 'saved_searches',
    label: 'saved searches',
    cells: {
      read: 'saved_searches:read',
      create: 'saved_searches:create',
      update: 'saved_searches:update',
      delete: 'saved_searches:delete',
    },
  },
  {
    family: 'task_views',
    label: 'task views',
    cells: {
      read: 'task_views:read',
      create: 'task_views:create',
      update: 'task_views:update',
      delete: 'task_views:delete',
    },
  },
];

const props = defineProps<{ modelValue: ApiKeyScope[] }>();

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
      return scope ? { scope } : null;
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
          <input
            type="checkbox"
            class="atl-scope-box"
            :data-scope="col.scope"
            :checked="isChecked(col.scope)"
            @change="toggle(col.scope)"
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
  border-radius: var(--r-md);
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
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-muted);
}

.atl-scope-family {
  padding: 0 10px;
  height: 30px;
  display: flex;
  align-items: center;
  font-size: 12px;
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

.atl-scope-box {
  width: 15px;
  height: 15px;
  cursor: pointer;
  accent-color: var(--c-primary);
}
</style>
