<script setup lang="ts">
import { computed } from 'vue';
import type { ApiKeyScope } from '@/stores/apiKeys';

const FAMILIES = ['tasks', 'docs', 'boards', 'folders', 'projects', 'webhooks'] as const;
const ACTIONS = ['read', 'create', 'update', 'delete'] as const;

type Family = (typeof FAMILIES)[number];
type Action = (typeof ACTIONS)[number];

const props = defineProps<{ modelValue: ApiKeyScope[] }>();

const emit = defineEmits<{ 'update:modelValue': [value: ApiKeyScope[]] }>();

const selected = computed(() => new Set<string>(props.modelValue));

function scopeOf(family: Family, action: Action): ApiKeyScope {
  return `${family}:${action}`;
}

function isChecked(family: Family, action: Action): boolean {
  return selected.value.has(scopeOf(family, action));
}

/**
 * Rebuilds the selection in canonical family×action order so the emitted list
 * is deterministic regardless of the order cells were toggled in.
 */
function toggle(family: Family, action: Action): void {
  const next = new Set(selected.value);
  const scope = scopeOf(family, action);

  if (next.has(scope)) next.delete(scope);
  else next.add(scope);

  const ordered: ApiKeyScope[] = [];
  for (const f of FAMILIES) {
    for (const a of ACTIONS) {
      const s = scopeOf(f, a);
      if (next.has(s)) ordered.push(s);
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

    <div v-for="fam in FAMILIES" :key="fam" class="atl-scope-row" data-scope-row>
      <div class="atl-scope-family">{{ fam }}</div>
      <label v-for="a in ACTIONS" :key="a" class="atl-scope-cell">
        <input
          type="checkbox"
          class="atl-scope-box"
          :data-scope="scopeOf(fam, a)"
          :checked="isChecked(fam, a)"
          @change="toggle(fam, a)"
        />
      </label>
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

.atl-scope-box {
  width: 15px;
  height: 15px;
  cursor: pointer;
  accent-color: var(--c-primary);
}
</style>
