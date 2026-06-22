import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import AssigneeAvatars from '@/components/tareas/AssigneeAvatars.vue';
import { type MeResponse, useAuthStore } from '@/stores/auth';
import type { ActorDto } from '@/stores/boards';

const actor = (id: string, type: string, displayName: string): ActorDto => ({
  id,
  type,
  display_name: displayName,
});

describe('AssigneeAvatars', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('renders all assignees when within the max', () => {
    const wrapper = mount(AssigneeAvatars, {
      props: {
        assignees: [actor('u1', 'user', 'Alice'), actor('a1', 'api_key', 'Agent Smith')],
        max: 3,
      },
    });

    expect(wrapper.find('.atl-assignee-more').exists()).toBe(false);
    expect(wrapper.findAll('.atl-assignees > span')).toHaveLength(2);
  });

  it('collapses overflow beyond max into a +N chip', () => {
    const wrapper = mount(AssigneeAvatars, {
      props: {
        assignees: [
          actor('u1', 'user', 'Alice'),
          actor('u2', 'user', 'Bob'),
          actor('u3', 'user', 'Carol'),
          actor('u4', 'user', 'Dan'),
          actor('u5', 'user', 'Eve'),
        ],
        max: 3,
      },
    });

    const more = wrapper.find('.atl-assignee-more');
    expect(more.exists()).toBe(true);
    expect(more.text()).toBe('+2');
    // 3 avatars + 1 overflow chip.
    expect(wrapper.findAll('.atl-assignees > span')).toHaveLength(4);
  });

  it('orders the current user first and marks them as "you"', () => {
    const auth = useAuthStore();
    auth.user = { id: 'me', principal_type: 'user', username: 'me', is_root: false } as MeResponse;

    const wrapper = mount(AssigneeAvatars, {
      props: {
        assignees: [
          actor('u1', 'user', 'Alice'),
          actor('a1', 'api_key', 'Agent Smith'),
          actor('me', 'user', 'My Self'),
        ],
        max: 3,
      },
    });

    const first = wrapper.find('.atl-assignees > span');
    expect(first.attributes('title')).toContain('(you)');
    expect(first.classes()).toContain('atl-assignee-me');
  });

  it('does not match a same-id actor of a different principal type as the current user', () => {
    const auth = useAuthStore();
    auth.user = { id: 'shared', principal_type: 'user', username: 'me', is_root: false } as MeResponse;

    const wrapper = mount(AssigneeAvatars, {
      props: {
        assignees: [actor('shared', 'api_key', 'Agent')],
        max: 3,
      },
    });

    expect(wrapper.find('.atl-assignee-me').exists()).toBe(false);
  });
});
