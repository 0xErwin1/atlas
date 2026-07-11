import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import TaskInspector from '@/components/tareas/TaskInspector.vue';
import { useTaskDetailStore } from '@/stores/taskDetail';

const task = {
  id: 'task-1',
  readable_id: 'ATL-1',
  title: 'Task',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-02T00:00:00Z',
  created_by: { id: 'user-1', type: 'user', display_name: 'Jordan' },
};

const stubs = {
  AgentBadge: true,
  Avatar: true,
  ErrorState: { props: ['title'], template: '<p role="alert">{{ title }}</p>' },
  LoadingState: { props: ['label'], template: '<p>{{ label }}</p>' },
  MetaRow: { props: ['label'], template: '<p>{{ label }}<slot /></p>' },
  SharePanel: true,
  ActivityComments: { template: '<p>Activity feed</p>' },
  InspectorTabs: {
    emits: ['update:active'],
    template: '<button type="button" @click="$emit(\'update:active\', \'details\')">Details</button>',
  },
};

function mountInspector() {
  return mount(TaskInspector, {
    props: { task, ws: 'ws' },
    global: { stubs },
  });
}

describe('TaskInspector collection status presentation', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('keeps detail metadata visible when a sibling collection fails', async () => {
    const detail = useTaskDetailStore();
    detail.collectionStatus = { ...detail.collectionStatus, references: 'error', backlinks: 'ready' };
    detail.collectionErrors = { ...detail.collectionErrors, references: 'References unavailable' };

    const wrapper = mountInspector();
    await wrapper.get('button').trigger('click');

    expect(wrapper.get('[role="alert"]').text()).toBe('Could not load references');
    expect(wrapper.text()).toContain('CreatedJordan');
    expect(wrapper.text()).toContain('References0');
  });

  it('keeps the activity feed visible when comments fail independently', () => {
    const detail = useTaskDetailStore();
    detail.collectionStatus = { ...detail.collectionStatus, activity: 'ready', comments: 'error' };
    detail.collectionErrors = { ...detail.collectionErrors, comments: 'Comments unavailable' };

    const wrapper = mountInspector();

    expect(wrapper.get('[role="alert"]').text()).toBe('Could not load comments');
    expect(wrapper.text()).toContain('Activity feed');
  });

  it('keeps an empty ready activity collection out of the refresh loader', () => {
    const detail = useTaskDetailStore();
    detail.collectionStatus = { ...detail.collectionStatus, activity: 'pending', comments: 'ready' };
    detail.collectionLoaded = { ...detail.collectionLoaded, activity: true, comments: true };

    const wrapper = mountInspector();

    expect(wrapper.text()).not.toContain('Loading activity…');
    expect(wrapper.text()).toContain('Activity feed');
  });
});
