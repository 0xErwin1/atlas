import { flushPromises } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import { createDesktopPlatformTransport, type DesktopBridge } from '@/platform/desktop';

const SESSION_ACTION_EVENT = 'atlas://session-action';

describe('DesktopWorkspaceEventSource', () => {
  it('stops the Rust workspace stream exactly once when listener registration fails', async () => {
    const invoke = vi.fn(
      (_command: string, _args?: Record<string, unknown>): Promise<unknown> => Promise.resolve(undefined),
    );
    const listen = vi.fn((event: string): Promise<() => void> => {
      if (event === SESSION_ACTION_EVENT) return Promise.resolve(() => {});
      return Promise.reject(new Error('listen registration failed'));
    });
    const bridge: DesktopBridge = {
      invoke: invoke as DesktopBridge['invoke'],
      listen: listen as DesktopBridge['listen'],
    };

    const transport = createDesktopPlatformTransport(bridge);
    transport.createWorkspaceEventSource('acme');

    await flushPromises();
    await flushPromises();

    const stopCalls = invoke.mock.calls.filter(([command]) => command === 'desktop_workspace_events_stop');
    expect(stopCalls).toHaveLength(1);
    expect(stopCalls[0]?.[1]).toEqual({ workspaceSlug: 'acme' });
  });
});
