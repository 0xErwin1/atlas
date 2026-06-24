import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ActivityFeed from '@/components/tareas/ActivityFeed.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import type { ActivityEntryDto } from '@/stores/taskDetail';

const entry = (id: string, kind: string, actorType: string, name: string): ActivityEntryDto => ({
  id,
  kind,
  actor: { id: `a-${id}`, type: actorType, display_name: name },
  payload: {},
  created_at: '12m ago',
  task_id: `t-${id}`,
  task_readable_id: `ATL-${id}`,
});

describe('ActivityFeed actor attribution (REQ-W22)', () => {
  it('renders a row per activity entry with its actor name', () => {
    const wrapper = mount(ActivityFeed, {
      props: {
        items: [entry('1', 'created', 'user', 'Jordan'), entry('2', 'moved', 'api_key', 'Claude')],
      },
    });

    expect(wrapper.findAll('li')).toHaveLength(2);
    expect(wrapper.text()).toContain('Jordan');
    expect(wrapper.text()).toContain('Claude');
  });

  it('marks agent actors distinctly with the magenta AgentBadge, never humans', () => {
    const wrapper = mount(ActivityFeed, {
      props: {
        items: [entry('1', 'assigned', 'user', 'Jordan'), entry('2', 'moved', 'api_key', 'Claude')],
      },
    });

    const userRow = wrapper.get('[data-actor-kind="user"]');
    const agentRow = wrapper.get('[data-actor-kind="agent"]');

    expect(userRow.findComponent(AgentBadge).exists()).toBe(false);
    expect(agentRow.findComponent(AgentBadge).exists()).toBe(true);
  });

  it('renders a human-readable verb per activity kind', () => {
    const wrapper = mount(ActivityFeed, {
      props: { items: [entry('1', 'checklist_promoted', 'api_key', 'Claude')] },
    });

    expect(wrapper.text()).toContain('promoted a checklist item to a task');
  });

  it('shows an empty state when there is no activity', () => {
    const wrapper = mount(ActivityFeed, { props: { items: [] } });
    expect(wrapper.text()).toContain('No activity yet.');
  });
});
