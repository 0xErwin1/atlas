import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import PresenceAvatars from '@/components/ui/PresenceAvatars.vue';
import { type MeResponse, useAuthStore } from '@/stores/auth';
import type { ActorDto } from '@/stores/boards';

const actor = (id: string, type: string, displayName: string): ActorDto => ({
  id,
  type,
  display_name: displayName,
});

describe('PresenceAvatars', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('renders an avatar stack of the present actors', () => {
    const wrapper = mount(PresenceAvatars, {
      props: { actors: [actor('u1', 'user', 'Alice'), actor('a1', 'api_key', 'Agent')] },
    });

    expect(wrapper.find('.atl-assignees').exists()).toBe(true);
    expect(wrapper.findAll('.atl-assignees > span')).toHaveLength(2);
  });

  it('renders nothing when there are no present actors', () => {
    const wrapper = mount(PresenceAvatars, { props: { actors: [] } });

    expect(wrapper.find('.atl-assignees').exists()).toBe(false);
  });

  it('excludes the current user and renders nothing when they are alone', () => {
    const auth = useAuthStore();
    auth.user = { id: 'me', principal_type: 'user', username: 'me', is_root: false } as MeResponse;

    const wrapper = mount(PresenceAvatars, {
      props: { actors: [actor('me', 'user', 'My Self')] },
    });

    expect(wrapper.find('.atl-assignees').exists()).toBe(false);
  });
});
