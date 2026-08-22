import { flushPromises } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import { createDesktopPlatformTransport, type DesktopBridge } from '@/platform/desktop';

const SESSION_ACTION_EVENT = 'atlas://session-action';

describe('DesktopWorkspaceEventSource', () => {
  it('does not stop the Rust workspace stream when listener registration fails before subscribe', async () => {
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
    expect(stopCalls).toHaveLength(0);
  });

  it('fails the source when subscribe returns an application error without rejecting', async () => {
    const invoke = vi.fn(async (command: string, _args?: Record<string, unknown>): Promise<unknown> => {
      if (command === 'desktop_workspace_events_subscribe') {
        return { error: 'desktop session is unavailable' };
      }
      return {};
    });
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

  it('treats a successful IpcResult with error: null as open (Rust wire shape)', async () => {
    const invoke = vi.fn(async (command: string, _args?: Record<string, unknown>): Promise<unknown> => {
      if (command === 'desktop_workspace_events_subscribe') {
        // Matches serde Option::None → JSON null on the desktop host.
        return { data: { generation: 3 }, error: null };
      }
      return {};
    });
    const listen = vi.fn(async (): Promise<() => void> => () => {});
    const bridge: DesktopBridge = {
      invoke: invoke as DesktopBridge['invoke'],
      listen: listen as DesktopBridge['listen'],
    };

    const source = createDesktopPlatformTransport(bridge).createWorkspaceEventSource('acme');
    const onopen = vi.fn();
    const onerror = vi.fn();
    source.onopen = onopen;
    source.onerror = onerror;

    await flushPromises();
    await flushPromises();

    expect(onopen).toHaveBeenCalledOnce();
    expect(onerror).not.toHaveBeenCalled();
    expect(source.readyState).toBe(1);
  });

  it('subscribes only after listeners are registered and keeps the generation for stop', async () => {
    const order: string[] = [];
    const invoke = vi.fn(async (command: string, _args?: Record<string, unknown>): Promise<unknown> => {
      order.push(command);
      if (command === 'desktop_workspace_events_subscribe') {
        return { data: { generation: 7 }, error: null };
      }
      return {};
    });
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

  it('does not let stale cleanup stop a replacement workspace stream', async () => {
    let resolveFirstSubscribe: ((result: unknown) => void) | undefined;
    const firstSubscribe = new Promise<unknown>((resolve) => {
      resolveFirstSubscribe = resolve;
    });
    let releaseWildcardStop: (() => void) | undefined;
    const wildcardStop = new Promise<void>((resolve) => {
      releaseWildcardStop = resolve;
    });
    let subscribeCount = 0;
    let currentNativeGeneration: number | null = null;
    const invoke = vi.fn((command: string, args?: Record<string, unknown>): Promise<unknown> => {
      if (command === 'desktop_workspace_events_subscribe') {
        subscribeCount += 1;
        if (subscribeCount === 1) return firstSubscribe;

        currentNativeGeneration = 2;
        return Promise.resolve({ data: { generation: 2 }, error: null });
      }
      if (command === 'desktop_workspace_events_stop') {
        return wildcardStop.then(() => {
          const generation = args?.generation;
          if (generation === null || generation === currentNativeGeneration) {
            currentNativeGeneration = null;
          }
          return {};
        });
      }
      return Promise.resolve({});
    });
    const listen = vi.fn(async (): Promise<() => void> => () => {});
    const bridge: DesktopBridge = {
      invoke: invoke as DesktopBridge['invoke'],
      listen: listen as DesktopBridge['listen'],
    };
    const transport = createDesktopPlatformTransport(bridge);

    const sourceA = transport.createWorkspaceEventSource('acme');
    await flushPromises();
    await flushPromises();
    expect(subscribeCount).toBe(1);

    sourceA.close();
    const sourceB = transport.createWorkspaceEventSource('acme');
    await flushPromises();
    await flushPromises();
    expect(sourceB.readyState).toBe(1);
    expect(currentNativeGeneration).toBe(2);

    if (releaseWildcardStop === undefined) throw new Error('wildcard stop gate was not initialized');
    releaseWildcardStop();
    await flushPromises();
    expect(sourceB.readyState).toBe(1);
    expect(currentNativeGeneration).toBe(2);

    if (resolveFirstSubscribe === undefined) throw new Error('first subscribe was not started');
    resolveFirstSubscribe({ data: { generation: 1 }, error: null });
    await flushPromises();
    await flushPromises();

    const stopCalls = invoke.mock.calls.filter(([command]) => command === 'desktop_workspace_events_stop');
    expect(stopCalls).toHaveLength(1);
    expect(stopCalls[0]?.[1]).toEqual({ workspaceSlug: 'acme', generation: 1 });
    expect(currentNativeGeneration).toBe(2);
  });
});

describe('desktop preference transport', () => {
  it('uses the desktop preference command names and camel-case argument keys', async () => {
    const invoke = vi.fn(async (): Promise<unknown> => ({ data: {} }));
    const listen = vi.fn(async (): Promise<() => void> => () => {});
    const bridge: DesktopBridge = {
      invoke: invoke as DesktopBridge['invoke'],
      listen: listen as DesktopBridge['listen'],
    };
    const transport = createDesktopPlatformTransport(bridge);

    await transport.getStartOnLogin();
    await transport.setStartOnLogin(true);
    await transport.getSystemTray();
    await transport.setSystemTray(false);

    expect(invoke).toHaveBeenCalledWith('desktop_get_start_on_login');
    expect(invoke).toHaveBeenCalledWith('desktop_set_start_on_login', { startOnLogin: true });
    expect(invoke).toHaveBeenCalledWith('desktop_get_system_tray');
    expect(invoke).toHaveBeenCalledWith('desktop_set_system_tray', { systemTray: false });
  });
});

describe('desktop publicBase cache', () => {
  function bridgeWithOrigin(result: (command: string) => Promise<unknown>): {
    bridge: DesktopBridge;
    invoke: ReturnType<typeof vi.fn>;
  } {
    const invoke = vi.fn(async (command: string): Promise<unknown> => result(command));
    const listen = vi.fn(async (): Promise<() => void> => () => {});
    return {
      bridge: { invoke: invoke as DesktopBridge['invoke'], listen: listen as DesktopBridge['listen'] },
      invoke,
    };
  }

  it('fires the origin lookup un-awaited at construction', () => {
    const { bridge, invoke } = bridgeWithOrigin(async () => ({ data: { origin: 'https://atlas.test' } }));

    createDesktopPlatformTransport(bridge);

    expect(invoke).toHaveBeenCalledWith('desktop_get_origin');
  });

  it('publicBase is empty before the eager lookup resolves', () => {
    const { bridge } = bridgeWithOrigin(async () => ({ data: { origin: 'https://atlas.test' } }));

    const transport = createDesktopPlatformTransport(bridge);

    expect(transport.publicBase()).toBe('');
  });

  it('publicBase reflects the origin once the eager lookup resolves', async () => {
    const { bridge } = bridgeWithOrigin(async () => ({ data: { origin: 'https://atlas.test' } }));

    const transport = createDesktopPlatformTransport(bridge);
    await flushPromises();
    await flushPromises();

    expect(transport.publicBase()).toBe('https://atlas.test');
  });

  it('leaves publicBase empty when the eager lookup returns an IpcResult error, with no unhandled rejection', async () => {
    const { bridge } = bridgeWithOrigin(async () => ({ error: 'desktop session is unavailable' }));

    const transport = createDesktopPlatformTransport(bridge);
    await flushPromises();
    await flushPromises();

    expect(transport.publicBase()).toBe('');
  });

  it('catches a rejecting eager lookup and leaves publicBase empty', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    const invoke = vi.fn(async (command: string): Promise<unknown> => {
      if (command === 'desktop_get_origin') throw new Error('bridge unavailable');
      return {};
    });
    const listen = vi.fn(async (): Promise<() => void> => () => {});
    const bridge: DesktopBridge = {
      invoke: invoke as DesktopBridge['invoke'],
      listen: listen as DesktopBridge['listen'],
    };

    const transport = createDesktopPlatformTransport(bridge);
    await flushPromises();
    await flushPromises();

    expect(transport.publicBase()).toBe('');
    expect(errorSpy).toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('setOrigin success updates the cached publicBase', async () => {
    const { bridge } = bridgeWithOrigin(async () => ({ data: { origin: 'https://old.test' } }));
    const transport = createDesktopPlatformTransport(bridge);
    await flushPromises();
    await flushPromises();

    (bridge.invoke as ReturnType<typeof vi.fn>).mockImplementationOnce(async () => ({
      data: { origin: 'https://new.test' },
    }));
    const result = await transport.setOrigin('https://new.test');

    expect(result).toEqual({ data: { origin: 'https://new.test' } });
    expect(transport.publicBase()).toBe('https://new.test');
  });

  it('setOrigin failure leaves the previous publicBase in place', async () => {
    const { bridge } = bridgeWithOrigin(async () => ({ data: { origin: 'https://old.test' } }));
    const transport = createDesktopPlatformTransport(bridge);
    await flushPromises();
    await flushPromises();

    (bridge.invoke as ReturnType<typeof vi.fn>).mockImplementationOnce(async () => ({
      error: 'origin rejected',
    }));
    await transport.setOrigin('https://blocked.test');

    expect(transport.publicBase()).toBe('https://old.test');
  });
});

describe('desktop openExternal', () => {
  it('invokes desktop_open_external with the url and returns the raw result', async () => {
    const invoke = vi.fn(async (): Promise<unknown> => ({}));
    const listen = vi.fn(async (): Promise<() => void> => () => {});
    const bridge: DesktopBridge = {
      invoke: invoke as DesktopBridge['invoke'],
      listen: listen as DesktopBridge['listen'],
    };
    const transport = createDesktopPlatformTransport(bridge);

    const result = await transport.openExternal('https://example.com');

    expect(invoke).toHaveBeenCalledWith('desktop_open_external', { url: 'https://example.com' });
    expect(result).toEqual({});
  });
});
