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
    expect(stopCalls[0]?.[1]).toEqual({ workspaceSlug: 'acme', generation: null });
  });

  it('fails the source when subscribe returns an application error without rejecting', async () => {
    const invoke = vi.fn(
      async (command: string, _args?: Record<string, unknown>): Promise<unknown> => {
        if (command === 'desktop_workspace_events_subscribe') {
          return { error: 'desktop session is unavailable' };
        }
        return {};
      },
    );
    const listen = vi.fn(async (): Promise<() => void> => () => {});
    const bridge: DesktopBridge = {
      invoke: invoke as DesktopBridge['invoke'],
      listen: listen as DesktopBridge['listen'],
    };

    const source = createDesktopPlatformTransport(bridge).createWorkspaceEventSource('acme');
    const onerror = vi.fn();
    source.onerror = onerror;

    await flushPromises();
    await flushPromises();

    expect(onerror).toHaveBeenCalledOnce();
    expect(source.readyState).toBe(2);
  });

  it('subscribes only after listeners are registered and keeps the generation for stop', async () => {
    const order: string[] = [];
    const invoke = vi.fn(
      async (command: string, _args?: Record<string, unknown>): Promise<unknown> => {
        order.push(command);
        if (command === 'desktop_workspace_events_subscribe') {
          return { data: { generation: 7 } };
        }
        return {};
      },
    );
    const listen = vi.fn(async (event: string): Promise<() => void> => {
      order.push(`listen:${event}`);
      return () => {};
    });
    const bridge: DesktopBridge = {
      invoke: invoke as DesktopBridge['invoke'],
      listen: listen as DesktopBridge['listen'],
    };

    const source = createDesktopPlatformTransport(bridge).createWorkspaceEventSource('acme');
    const onopen = vi.fn();
    source.onopen = onopen;

    await flushPromises();
    await flushPromises();

    expect(onopen).toHaveBeenCalledOnce();
    expect(source.readyState).toBe(1);
    expect(order.indexOf('listen:atlas://workspace-event')).toBeLessThan(
      order.indexOf('desktop_workspace_events_subscribe'),
    );

    source.close();
    await flushPromises();

    const stopCalls = invoke.mock.calls.filter(([command]) => command === 'desktop_workspace_events_stop');
    expect(stopCalls.at(-1)?.[1]).toEqual({ workspaceSlug: 'acme', generation: 7 });
  });
});
