import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import AssigneeList from '@/components/tareas/AssigneeList.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import type { AssigneeDto } from '@/stores/taskDetail';

const assignee = (id: string, type: string, name: string): AssigneeDto => ({
  assignee: { id, type, display_name: name },
  assigned_by: { id: 'admin', type: 'user', display_name: 'Admin' },
  assigned_at: '2026-01-01T00:00:00Z',
});

describe('AssigneeList (REQ-W22)', () => {
  it('renders user and agent assignees, badging only the agent', () => {
    const wrapper = mount(AssigneeList, {
      props: { assignees: [assignee('u1', 'user', 'Jordan'), assignee('k1', 'api_key', 'Claude')] },
    });

    const userRow = wrapper.get('[data-assignee-kind="user"]');
    const agentRow = wrapper.get('[data-assignee-kind="agent"]');

    expect(userRow.findComponent(AgentBadge).exists()).toBe(false);
    expect(agentRow.findComponent(AgentBadge).exists()).toBe(true);
    expect(wrapper.text()).toContain('Jordan');
    expect(wrapper.text()).toContain('Claude');
  });

  it('emits remove with the actor type and id (so the api_key:{id} ref can be built)', async () => {
    const wrapper = mount(AssigneeList, {
      props: { assignees: [assignee('k1', 'api_key', 'Claude')] },
    });

    await wrapper.get('button[aria-label="Remove Claude"]').trigger('click');

    expect(wrapper.emitted('remove')).toEqual([['api_key', 'k1']]);
  });

  it('shows Unassigned when there are no assignees', () => {
    const wrapper = mount(AssigneeList, { props: { assignees: [] } });
    expect(wrapper.text()).toContain('Unassigned');
  });
});
