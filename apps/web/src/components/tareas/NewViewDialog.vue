<script setup lang="ts">
/**
 * Create / edit dialog for a custom, workspace-wide task view. A custom view is
 * NOT scoped to the current board: it exposes only workspace-level filters
 * (assignee, creator type, priorities, labels, sort) that map 1:1 onto
 * TaskViewFiltersDto. Board / column scoping is intentionally omitted.
 *
 * `initial` null means create mode; a non-null value prefills edit mode. On a
 * valid submit the dialog emits `submit` with the assembled name + filters; the
 * host owns persistence and navigation. `cancel` closes without saving.
 */
import { computed, ref, watch } from 'vue';
import { z } from 'zod';
import type { components } from '@/api/types.d.ts';
import Btn from '@/components/ui/Btn.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import MultiSelect, { type MultiSelectOption } from '@/components/ui/MultiSelect.vue';
import Popover from '@/components/ui/Popover.vue';
import { validateForm } from '@/lib/validation';
import { useLabelColorsStore } from '@/stores/labelColors';

type TaskViewFiltersDto = components['schemas']['TaskViewFiltersDto'];

type AssigneeChoice = 'anyone' | 'me';
type ActorChoice = 'anyone' | 'user' | 'api_key';
type SortChoice = NonNullable<TaskViewFiltersDto['sort']>;

const props = defineProps<{
  open: boolean;
  initial?: { name: string; filters: TaskViewFiltersDto } | null;
}>();

const emit = defineEmits<{
  submit: [{ name: string; filters: TaskViewFiltersDto }];
  cancel: [];
}>();

const labelColors = useLabelColorsStore();

const nameSchema = z.object({
  name: z.string().trim().min(1, 'Name is required').max(100, 'Name is too long'),
});

const name = ref('');
const assignee = ref<AssigneeChoice>('anyone');
const actorType = ref<ActorChoice>('anyone');
const priorities = ref<string[]>([]);
const labels = ref<string[]>([]);
const sort = ref<SortChoice>('updated_at_desc');
const nameError = ref<string | null>(null);

const isEdit = computed(() => props.initial !== null && props.initial !== undefined);

const ASSIGNEE_OPTIONS: Array<{ value: AssigneeChoice; label: string }> = [
  { value: 'anyone', label: 'Anyone' },
  { value: 'me', label: 'Me' },
];

const ACTOR_OPTIONS: Array<{ value: ActorChoice; label: string }> = [
  { value: 'anyone', label: 'Anyone' },
  { value: 'user', label: 'Users' },
  { value: 'api_key', label: 'Agents' },
];

const PRIORITY_OPTIONS: MultiSelectOption[] = [
  { value: 'urgent', label: 'Urgent', dot: 'var(--c-danger)' },
  { value: 'high', label: 'High', dot: 'var(--c-primary)' },
  { value: 'medium', label: 'Medium', dot: 'var(--c-info)' },
  { value: 'low', label: 'Low', dot: 'var(--c-muted)' },
];

const SORT_OPTIONS: Array<{ value: SortChoice; label: string }> = [
  { value: 'updated_at_desc', label: 'Recently updated' },
  { value: 'updated_at_asc', label: 'Oldest updated' },
  { value: 'created_at_desc', label: 'Recently created' },
  { value: 'created_at_asc', label: 'Oldest created' },
  { value: 'priority_desc', label: 'Highest priority' },
  { value: 'title_asc', label: 'Title A–Z' },
];

const labelOptions = computed<MultiSelectOption[]>(() =>
  labelColors.tagNames.map((tag) => ({ value: tag, label: tag })),
);

const sortLabel = computed(
  () => SORT_OPTIONS.find((o) => o.value === sort.value)?.label ?? 'Recently updated',
);

function resetForm(): void {
  const initial = props.initial;

  if (initial === null || initial === undefined) {
    name.value = '';
    assignee.value = 'anyone';
    actorType.value = 'anyone';
    priorities.value = [];
    labels.value = [];
    sort.value = 'updated_at_desc';
    nameError.value = null;
    return;
  }

  const filters = initial.filters;

  name.value = initial.name;
  assignee.value = filters.assignee === 'me' ? 'me' : 'anyone';
  actorType.value =
    filters.actor_type === 'user' || filters.actor_type === 'api_key' ? filters.actor_type : 'anyone';
  priorities.value = [...(filters.priorities ?? [])];
  labels.value = [...(filters.labels ?? [])];
  sort.value = (filters.sort as SortChoice | null | undefined) ?? 'updated_at_desc';
  nameError.value = null;
}

watch(
  () => [props.open, props.initial] as const,
  ([open]) => {
    if (open) resetForm();
  },
  { immediate: true },
);

function buildFilters(): TaskViewFiltersDto {
  const filters: TaskViewFiltersDto = { sort: sort.value };

  if (assignee.value === 'me') filters.assignee = 'me';
  if (actorType.value !== 'anyone') filters.actor_type = actorType.value;
  if (priorities.value.length > 0) filters.priorities = [...priorities.value];
  if (labels.value.length > 0) filters.labels = [...labels.value];

  return filters;
}

function submit(): void {
  const validation = validateForm(nameSchema, { name: name.value });

  if (!validation.ok) {
    nameError.value = validation.errors.name ?? 'Name is invalid';
    return;
  }

  nameError.value = null;
  emit('submit', { name: validation.data.name, filters: buildFilters() });
}
</script>

<template>
  <div
    v-if="open"
    class="fixed inset-0 flex items-center justify-center"
    style="background-color: var(--c-overlay); z-index: 60;"
    @click.self="emit('cancel')"
  >
    <div
      role="dialog"
      :aria-label="isEdit ? 'Edit view' : 'New view'"
      style="width: 440px; max-width: calc(100vw - 32px); background-color: var(--c-panel); border: 1px solid var(--c-border); border-radius: var(--r-lg); box-shadow: var(--shadow-lg); overflow: visible;"
    >
      <div
        class="flex items-center"
        style="gap: 10px; padding: 13px 16px; border-bottom: 1px solid var(--c-border);"
      >
        <Icon name="layout-list" :size="17" :style="{ color: 'var(--c-foreground)' }" />
        <div
          class="flex-1 min-w-0"
          style="font-size: var(--fs-xl); font-weight: var(--fw-bold); color: var(--c-foreground);"
        >
          {{ isEdit ? 'Edit view' : 'New view' }}
        </div>
        <button
          type="button"
          title="Close"
          aria-label="Close"
          class="atl-gbtn"
          style="width: 26px; height: 26px;"
          @click="emit('cancel')"
        >
          <Icon name="x" :size="16" />
        </button>
      </div>

      <div class="flex flex-col" style="gap: 16px; padding: 16px;">
        <FormField
          v-model="name"
          label="Name"
          placeholder="My urgent work"
          :error="nameError"
          @keydown.enter.prevent="submit"
        />

        <div class="atl-nv-field">
          <span class="atl-nv-label">Assignee</span>
          <div class="atl-nv-seg">
            <button
              v-for="opt in ASSIGNEE_OPTIONS"
              :key="opt.value"
              type="button"
              class="atl-nv-segbtn"
              :class="{ active: assignee === opt.value }"
              @click="assignee = opt.value"
            >
              {{ opt.label }}
            </button>
          </div>
        </div>

        <div class="atl-nv-field">
          <span class="atl-nv-label">Created by</span>
          <div class="atl-nv-seg">
            <button
              v-for="opt in ACTOR_OPTIONS"
              :key="opt.value"
              type="button"
              class="atl-nv-segbtn"
              :class="{ active: actorType === opt.value }"
              @click="actorType = opt.value"
            >
              {{ opt.label }}
            </button>
          </div>
        </div>

        <div class="atl-nv-field">
          <span class="atl-nv-label">Priorities</span>
          <MultiSelect
            v-model="priorities"
            :options="PRIORITY_OPTIONS"
            placeholder="Any priority"
            icon="flag"
          />
        </div>

        <div class="atl-nv-field">
          <span class="atl-nv-label">Labels</span>
          <MultiSelect
            v-model="labels"
            :options="labelOptions"
            placeholder="Any label"
            icon="tag"
          />
        </div>

        <div class="atl-nv-field">
          <span class="atl-nv-label">Sort</span>
          <Popover placement="bottom-start" block>
            <template #trigger="{ open: menuOpen, toggle }">
              <button
                type="button"
                class="atl-nv-sort"
                :style="{ borderColor: menuOpen ? 'var(--c-primary)' : 'var(--c-border)' }"
                @click="toggle"
              >
                <span>{{ sortLabel }}</span>
                <Icon
                  name="chevron-down"
                  :size="13"
                  :style="{
                    flex: '0 0 auto',
                    color: 'var(--c-muted)',
                    transform: menuOpen ? 'rotate(180deg)' : 'none',
                    transition: 'transform 0.1s',
                  }"
                />
              </button>
            </template>

            <template #default="{ close }">
              <ul role="listbox" style="list-style: none; padding: 3px; min-width: 100%;">
                <li
                  v-for="opt in SORT_OPTIONS"
                  :key="opt.value"
                  role="option"
                  :aria-selected="opt.value === sort"
                  class="atl-nv-sortopt"
                  :class="{ active: opt.value === sort }"
                  @click="sort = opt.value, close()"
                >
                  {{ opt.label }}
                </li>
              </ul>
            </template>
          </Popover>
        </div>
      </div>

      <div
        class="flex items-center"
        style="gap: 8px; padding: 12px 16px; border-top: 1px solid var(--c-border); justify-content: flex-end;"
      >
        <Btn variant="secondary" @click="emit('cancel')">Cancel</Btn>
        <Btn variant="primary" @click="submit">{{ isEdit ? 'Save' : 'Create view' }}</Btn>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-nv-field {
  display: flex;
  flex-direction: column;
}

.atl-nv-label {
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--c-muted);
  margin-bottom: 5px;
}

.atl-nv-seg {
  display: inline-flex;
  align-self: flex-start;
  gap: 2px;
  padding: 2px;
  background: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
}

.atl-nv-segbtn {
  height: 24px;
  padding: 0 12px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-muted);
  font-size: var(--fs-sm);
  font-weight: var(--fw-medium);
  cursor: pointer;
}

.atl-nv-segbtn:hover {
  color: var(--c-foreground);
}

.atl-nv-segbtn.active {
  background: var(--c-selection);
  color: var(--c-primary);
}

.atl-nv-sort {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  width: 100%;
  height: 26px;
  padding: 0 8px;
  background: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  color: var(--c-foreground);
  font-size: 11.5px;
  cursor: pointer;
}

.atl-nv-sortopt {
  display: flex;
  align-items: center;
  height: 26px;
  padding: 0 8px;
  border-radius: 3px;
  font-size: var(--fs-sm);
  color: var(--c-foreground);
  white-space: nowrap;
  cursor: pointer;
}

.atl-nv-sortopt:hover {
  background: var(--c-raised);
}

.atl-nv-sortopt.active {
  background: var(--c-selection);
  color: var(--c-primary);
}
</style>
