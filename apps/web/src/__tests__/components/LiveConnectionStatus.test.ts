import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import LiveConnectionStatus from '@/components/shell/LiveConnectionStatus.vue';
import type { LiveConnectionState, WorkspaceLiveUpdateHandlers } from '@/lib/workspaceLiveUpdates';

/**
 * The indicator is the only place the app admits that live updates stopped
 * arriving, so it has to appear on a dropped stream and disappear on recovery.
 */

let handlers: WorkspaceLiveUpdateHandlers | null = null;

vi.mock('@/lib/workspaceLiveUpdates', async (importOriginal) => {
  const original = await importOriginal<typeof import('@/lib/workspaceLiveUpdates')>();
  return {
    ...original,
    acquireWorkspaceLiveUpdates: (_ws: string, subscriber: WorkspaceLiveUpdateHandlers) => {
      handlers = subscriber;
      subscriber.onConnectionState?.('connected');
      return { release: () => undefined };
    },
  };
});

async function announce(state: LiveConnectionState): Promise<void> {
  handlers?.onConnectionState?.(state);
  await nextTick();
}

describe('LiveConnectionStatus', () => {
  beforeEach(() => {
    handlers = null;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows nothing while the stream is healthy', () => {
    const wrapper = mount(LiveConnectionStatus, { props: { ws: 'atlas' }, shallow: true });

    expect(wrapper.find('.atl-live-status').exists()).toBe(false);
  });

  it('appears while reconnecting and flags the give-up as offline', async () => {
    const wrapper = mount(LiveConnectionStatus, { props: { ws: 'atlas' }, shallow: true });

    await announce('reconnecting');
    const reconnecting = wrapper.get('.atl-live-status');
    expect(reconnecting.classes()).not.toContain('offline');
    expect(reconnecting.attributes('aria-label')).toContain('Reconnecting');

    await announce('offline');
    const offline = wrapper.get('.atl-live-status');
    expect(offline.classes()).toContain('offline');
    expect(offline.attributes('aria-label')).toContain('out of date');
  });

  it('disappears again once the stream reconnects', async () => {
    const wrapper = mount(LiveConnectionStatus, { props: { ws: 'atlas' }, shallow: true });

    await announce('offline');
    await announce('connected');

    expect(wrapper.find('.atl-live-status').exists()).toBe(false);
  });
});
