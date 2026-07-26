import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { components } from '@/api/types.d.ts';

const { load, updateTask, patchOpenTask, showBanner } = vi.hoisted(() => ({
  load: vi.fn(),
  updateTask: vi.fn(),
  patchOpenTask: vi.fn(),
  showBanner: vi.fn(),
}));

vi.mock('@/stores/propertyDefinitions', () => ({
  usePropertyDefinitionsStore: () => ({
    definitions: [
      {
        applies_to: 'task',
        created_at: '2026-01-01T00:00:00Z',
        id: 'field-1',
        key: 'billable',
        kind: 'boolean',
        name: 'Billable',
      },
    ],
    error: null,
    load,
    create: vi.fn(),
    remove: vi.fn(),
  }),
}));

vi.mock('@/stores/boards', () => ({
  useBoardsStore: () => ({ updateTask, error: 'Unable to update custom fields' }),
}));

vi.mock('@/stores/tasks', () => ({
  useTasksStore: () => ({ patchOpenTask }),
}));

vi.mock('@/stores/ui', () => ({
  useUiStore: () => ({ showBanner }),
}));

import CustomFieldsSection from '@/components/tareas/CustomFieldsSection.vue';

type TaskDto = components['schemas']['TaskDto'];

const task: TaskDto = {
  board_id: 'board-1',
  board_name: 'Board',
  column_id: 'column-1',
  column_name: 'To Do',
  created_at: '2026-01-01T00:00:00Z',
  created_by: { id: 'user-1', type: 'user' },
  description: '',
  id: 'task-1',
  project_id: 'project-1',
  properties: { billable: false },
  readable_id: 'ATL-1',
  title: 'Example',
  updated_at: '2026-01-01T00:00:00Z',
  workspace_id: 'workspace-1',
};

describe('CustomFieldsSection', () => {
  beforeEach(() => {
    load.mockReset();
    updateTask.mockReset();
    patchOpenTask.mockReset();
    showBanner.mockReset();
  });

  it('keeps the stored boolean value when persistence fails', async () => {
    updateTask.mockResolvedValue(false);
    const wrapper = mount(CustomFieldsSection, { props: { ws: 'atlas', task } });
    const checkbox = wrapper.get<HTMLInputElement>('input[aria-label="Billable custom field"]');

    await checkbox.setValue(true);
    await flushPromises();

    expect(updateTask).toHaveBeenCalledWith('atlas', 'ATL-1', { properties: { billable: true } });
    expect(checkbox.element.checked).toBe(false);
    expect(patchOpenTask).not.toHaveBeenCalled();
    expect(showBanner).toHaveBeenCalledWith('Unable to update custom fields', 'error');
  });
});
