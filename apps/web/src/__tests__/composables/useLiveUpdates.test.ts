import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { effectScope, nextTick, ref } from 'vue';
import { useLiveUpdates } from '@/composables/useLiveUpdates';
import { resetWorkspaceLiveUpdatesForTest } from '@/lib/workspaceLiveUpdates';

class FakeEventSource {
  static instances: FakeEventSource[] = [];
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;

  url: string;
  closed = false;
  readyState = FakeEventSource.CONNECTING;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  private listeners: Record<string, ((ev: Event) => void)[]> = {};

  constructor(url: string) {
    this.url = url;
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, cb: (ev: Event) => void): void {
    const existing = this.listeners[type] ?? [];
    existing.push(cb);
    this.listeners[type] = existing;
  }

  close(): void {
    this.closed = true;
    this.readyState = FakeEventSource.CLOSED;
  }

  emitOpen(): void {
    this.readyState = FakeEventSource.OPEN;
    this.onopen?.(new Event('open'));
  }

  emitError(readyState: number): void {
    this.readyState = readyState;
    this.onerror?.(new Event('error'));
  }

  emit(type: string, data: string): void {
    const event = new MessageEvent(type, { data });
    if (type === 'message') this.onmessage?.(event);
    for (const cb of this.listeners[type] ?? []) cb(event);
  }
}

function envelope(eventType: string, data: unknown): string {
  return JSON.stringify({
    id: 'evt-1',
    event_type: eventType,
    version: 1,
    source: 'test',
    workspace_id: 'ws-1',
    board_id: 'board-1',
    occurred_at: '2026-01-01T00:00:00Z',
    actor: { type: 'user', id: 'u1' },
    data,
  });
}

describe('useLiveUpdates', () => {
  beforeEach(() => {
    resetWorkspaceLiveUpdatesForTest();
    FakeEventSource.instances = [];
    vi.stubGlobal('EventSource', FakeEventSource);
  });

  afterEach(() => {
    resetWorkspaceLiveUpdatesForTest();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('shares one native source and fans out events and resyncs across concurrent scopes', () => {
    const ws = ref('acme');
    const first = { onEvent: vi.fn(), onResync: vi.fn() };
    const second = { onEvent: vi.fn(), onResync: vi.fn() };
    const firstScope = effectScope();
    const secondScope = effectScope();

    firstScope.run(() => useLiveUpdates(ws, first));
    secondScope.run(() => useLiveUpdates(ws, second));

    expect(FakeEventSource.instances).toHaveLength(1);

    const stream = FakeEventSource.instances[0];
    stream?.emit('task.updated', envelope('task.updated', { task_id: 't1' }));
    stream?.emit('resync', 'reload');

    expect(first.onEvent).toHaveBeenCalledTimes(1);
    expect(second.onEvent).toHaveBeenCalledTimes(1);
    expect(first.onResync).toHaveBeenCalledTimes(1);
    expect(second.onResync).toHaveBeenCalledTimes(1);

    firstScope.stop();
    stream?.emit('task.updated', envelope('task.updated', { task_id: 't2' }));
    stream?.emit('resync', 'reload');

    expect(first.onEvent).toHaveBeenCalledTimes(1);
    expect(first.onResync).toHaveBeenCalledTimes(1);
    expect(second.onEvent).toHaveBeenCalledTimes(2);
    expect(second.onResync).toHaveBeenCalledTimes(2);

    secondScope.stop();
  });

  it('opens a stream for the workspace when the slug is set', () => {
    const ws = ref('acme');
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

    expect(FakeEventSource.instances).toHaveLength(1);
    expect(FakeEventSource.instances[0]?.url).toBe('/api/workspaces/acme/events');

    scope.stop();
  });

  it('does not open a stream for an empty slug', () => {
    const ws = ref('');
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

    expect(FakeEventSource.instances).toHaveLength(0);

    scope.stop();
  });

  it('closes the old stream and reopens on a slug change', async () => {
    const ws = ref('acme');
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

    ws.value = 'globex';
    await nextTick();

    expect(FakeEventSource.instances).toHaveLength(2);
    expect(FakeEventSource.instances[0]?.closed).toBe(true);
    expect(FakeEventSource.instances[1]?.url).toBe('/api/workspaces/globex/events');

    scope.stop();
  });

  it('retains the stream for the broker idle grace period on scope dispose', () => {
    vi.useFakeTimers();
    const ws = ref('acme');
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

    scope.stop();
    expect(FakeEventSource.instances[0]?.closed).toBe(false);

    vi.advanceTimersByTime(30_000);
    expect(FakeEventSource.instances[0]?.closed).toBe(true);
  });

  it('parses a named domain message and dispatches the envelope to onEvent', () => {
    const ws = ref('acme');
    const onEvent = vi.fn();
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent, onResync: vi.fn() }));

    FakeEventSource.instances[0]?.emit('task.moved', envelope('task.moved', { task_id: 't1' }));

    expect(onEvent).toHaveBeenCalledTimes(1);
    expect(onEvent.mock.calls[0]?.[0]).toMatchObject({
      type: 'task.moved',
      data: { task_id: 't1' },
    });

    scope.stop();
  });

  it('dispatches a live-only presence.updated frame through onEvent', () => {
    const ws = ref('acme');
    const onEvent = vi.fn();
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent, onResync: vi.fn() }));

    FakeEventSource.instances[0]?.emit(
      'presence.updated',
      envelope('presence.updated', { board_id: 'board-1', actors: [] }),
    );

    expect(onEvent).toHaveBeenCalledTimes(1);
    expect(onEvent.mock.calls[0]?.[0]).toMatchObject({
      type: 'presence.updated',
      data: { board_id: 'board-1', actors: [] },
    });

    scope.stop();
  });

  it('dispatches a default (unnamed) message via the message fallback', () => {
    const ws = ref('acme');
    const onEvent = vi.fn();
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent, onResync: vi.fn() }));

    FakeEventSource.instances[0]?.emit('message', envelope('task.created', { task_id: 't9' }));

    expect(onEvent).toHaveBeenCalledTimes(1);
    expect(onEvent.mock.calls[0]?.[0]).toMatchObject({ type: 'task.created' });

    scope.stop();
  });

  it('ignores an unparseable message without throwing or dispatching', () => {
    const ws = ref('acme');
    const onEvent = vi.fn();
    vi.spyOn(console, 'debug').mockImplementation(() => {});
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent, onResync: vi.fn() }));

    expect(() => FakeEventSource.instances[0]?.emit('message', 'not json')).not.toThrow();
    expect(onEvent).not.toHaveBeenCalled();

    scope.stop();
  });

  it('calls onResync for the server resync marker', () => {
    const ws = ref('acme');
    const onResync = vi.fn();
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync }));

    FakeEventSource.instances[0]?.emit('resync', 'reload');

    expect(onResync).toHaveBeenCalledTimes(1);

    scope.stop();
  });

  it('does not resync on the first open but resyncs after a reconnect', () => {
    vi.useFakeTimers();
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const ws = ref('acme');
    const onResync = vi.fn();
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync }));

    const stream = FakeEventSource.instances[0];
    stream?.emitOpen();
    expect(onResync).not.toHaveBeenCalled();

    stream?.emitError(FakeEventSource.CLOSED);
    vi.advanceTimersByTime(500);
    FakeEventSource.instances[1]?.emitOpen();
    expect(onResync).toHaveBeenCalledTimes(1);

    scope.stop();
  });

  describe('bounded reconnect with backoff', () => {
    beforeEach(() => {
      vi.useFakeTimers();
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('does not take over when readyState is CONNECTING (lets native retry)', () => {
      const ws = ref('acme');
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

      FakeEventSource.instances[0]?.emitError(FakeEventSource.CONNECTING);
      vi.advanceTimersByTime(60_000);

      expect(FakeEventSource.instances).toHaveLength(1);

      scope.stop();
    });

    it('schedules a reopen with base backoff after a CLOSED error', () => {
      vi.spyOn(Math, 'random').mockReturnValue(0);
      const ws = ref('acme');
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

      FakeEventSource.instances[0]?.emitError(FakeEventSource.CLOSED);

      vi.advanceTimersByTime(499);
      expect(FakeEventSource.instances).toHaveLength(1);

      vi.advanceTimersByTime(1);
      expect(FakeEventSource.instances).toHaveLength(2);
      expect(FakeEventSource.instances[1]?.url).toBe('/api/workspaces/acme/events');

      scope.stop();
    });

    it('doubles the backoff delay per attempt up to the 30s cap', () => {
      vi.spyOn(Math, 'random').mockReturnValue(0);
      const ws = ref('acme');
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

      const expectedDelays = [500, 1000, 2000, 4000, 8000, 15_000, 15_000];

      for (const delay of expectedDelays) {
        const before = FakeEventSource.instances.length;
        FakeEventSource.instances[before - 1]?.emitError(FakeEventSource.CLOSED);

        vi.advanceTimersByTime(delay - 1);
        expect(FakeEventSource.instances).toHaveLength(before);

        vi.advanceTimersByTime(1);
        expect(FakeEventSource.instances).toHaveLength(before + 1);
      }

      scope.stop();
    });

    it('fires onResync (not first) and resets the attempt counter when the reopened stream connects', () => {
      vi.spyOn(Math, 'random').mockReturnValue(0);
      const ws = ref('acme');
      const onResync = vi.fn();
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync }));

      const first = FakeEventSource.instances[0];
      first?.emitOpen();
      first?.emitError(FakeEventSource.CLOSED);
      vi.advanceTimersByTime(500);

      const reopened = FakeEventSource.instances[1];
      reopened?.emitOpen();
      expect(onResync).toHaveBeenCalledTimes(1);

      reopened?.emitError(FakeEventSource.CLOSED);
      vi.advanceTimersByTime(499);
      expect(FakeEventSource.instances).toHaveLength(2);
      vi.advanceTimersByTime(1);
      expect(FakeEventSource.instances).toHaveLength(3);

      scope.stop();
    });

    it('stops after the max reconnect attempts and calls onReconnectFailed', () => {
      vi.spyOn(Math, 'random').mockReturnValue(0);
      const ws = ref('acme');
      const onReconnectFailed = vi.fn();
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn(), onReconnectFailed }));

      for (let attempt = 0; attempt < 10; attempt++) {
        const last = FakeEventSource.instances[FakeEventSource.instances.length - 1];
        last?.emitError(FakeEventSource.CLOSED);
        vi.advanceTimersByTime(30_000);
      }

      expect(FakeEventSource.instances).toHaveLength(11);
      expect(onReconnectFailed).not.toHaveBeenCalled();

      const last = FakeEventSource.instances[FakeEventSource.instances.length - 1];
      last?.emitError(FakeEventSource.CLOSED);
      vi.advanceTimersByTime(30_000);

      expect(FakeEventSource.instances).toHaveLength(11);
      expect(onReconnectFailed).toHaveBeenCalledTimes(1);

      scope.stop();
    });

    it('retains a pending reconnect timer during the broker idle grace period', () => {
      vi.spyOn(Math, 'random').mockReturnValue(0);
      const ws = ref('acme');
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

      FakeEventSource.instances[0]?.emitError(FakeEventSource.CLOSED);
      scope.stop();

      vi.advanceTimersByTime(500);
      expect(FakeEventSource.instances).toHaveLength(2);
    });
  });

  describe('foreground recovery (visibility/focus/online)', () => {
    beforeEach(() => {
      vi.useFakeTimers();
      Object.defineProperty(document, 'visibilityState', {
        value: 'visible',
        configurable: true,
      });
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    function setVisibility(state: 'visible' | 'hidden'): void {
      Object.defineProperty(document, 'visibilityState', { value: state, configurable: true });
    }

    it('reopens and resyncs when the tab becomes visible on a stale/closed stream', () => {
      const ws = ref('acme');
      const onResync = vi.fn();
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync }));

      const stream = FakeEventSource.instances[0];
      stream?.emitOpen();
      // Simulate a frozen backgrounded tab: readyState looks CLOSED without
      // `onerror` ever firing, since the browser throttled the timers/socket.
      if (stream) stream.readyState = FakeEventSource.CLOSED;

      setVisibility('visible');
      document.dispatchEvent(new Event('visibilitychange'));
      vi.advanceTimersByTime(300);

      expect(FakeEventSource.instances).toHaveLength(2);

      FakeEventSource.instances[1]?.emitOpen();
      expect(onResync).toHaveBeenCalledTimes(1);

      scope.stop();
    });

    it('fires onResync only (no reopen) on window focus when the stream is healthy/OPEN', () => {
      const ws = ref('acme');
      const onResync = vi.fn();
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync }));

      FakeEventSource.instances[0]?.emitOpen();

      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(300);

      expect(FakeEventSource.instances).toHaveLength(1);
      expect(onResync).toHaveBeenCalledTimes(1);

      scope.stop();
    });

    it('reopens on the online event when the stream is CLOSED', () => {
      const ws = ref('acme');
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

      const stream = FakeEventSource.instances[0];
      stream?.emitOpen();
      if (stream) stream.readyState = FakeEventSource.CLOSED;

      window.dispatchEvent(new Event('online'));
      vi.advanceTimersByTime(300);

      expect(FakeEventSource.instances).toHaveLength(2);

      scope.stop();
    });

    it('does not disturb a healthy stream when visibility toggles', () => {
      const ws = ref('acme');
      const onResync = vi.fn();
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync }));

      FakeEventSource.instances[0]?.emitOpen();
      onResync.mockClear();

      setVisibility('hidden');
      document.dispatchEvent(new Event('visibilitychange'));
      vi.advanceTimersByTime(300);

      expect(FakeEventSource.instances).toHaveLength(1);
      expect(onResync).not.toHaveBeenCalled();

      scope.stop();
    });

    it('coalesces N rapid toggles within the debounce window into a single reopen', () => {
      const ws = ref('acme');
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

      const stream = FakeEventSource.instances[0];
      stream?.emitOpen();
      if (stream) stream.readyState = FakeEventSource.CLOSED;

      document.dispatchEvent(new Event('visibilitychange'));
      vi.advanceTimersByTime(100);
      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(100);
      window.dispatchEvent(new Event('online'));

      vi.advanceTimersByTime(299);
      expect(FakeEventSource.instances).toHaveLength(1);

      vi.advanceTimersByTime(1);
      expect(FakeEventSource.instances).toHaveLength(2);

      scope.stop();
    });

    it('does not open a second socket if a foreground reopen is already in flight', () => {
      const ws = ref('acme');
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

      const stream = FakeEventSource.instances[0];
      stream?.emitOpen();
      if (stream) stream.readyState = FakeEventSource.CLOSED;

      document.dispatchEvent(new Event('visibilitychange'));
      vi.advanceTimersByTime(300);
      expect(FakeEventSource.instances).toHaveLength(2);

      // The reopened stream is still CONNECTING (FakeEventSource default); a
      // second foreground signal before it settles must not spawn a third.
      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(300);
      expect(FakeEventSource.instances).toHaveLength(2);

      scope.stop();
    });

    it('replaces an established native CONNECTING source once and preserves the foreground replacement', () => {
      const ws = ref('acme');
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));
      const nativeSource = FakeEventSource.instances[0];

      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(300);

      expect(nativeSource?.closed).toBe(true);
      expect(FakeEventSource.instances).toHaveLength(2);
      const foregroundSource = FakeEventSource.instances[1];

      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(300);

      expect(foregroundSource?.closed).toBe(false);
      expect(FakeEventSource.instances).toHaveLength(2);

      foregroundSource?.emitError(FakeEventSource.CONNECTING);
      vi.advanceTimersByTime(60_000);
      expect(FakeEventSource.instances).toHaveLength(2);

      scope.stop();
    });

    it('removes visibility/focus/online listeners after idle teardown', () => {
      const addDocSpy = vi.spyOn(document, 'addEventListener');
      const removeDocSpy = vi.spyOn(document, 'removeEventListener');
      const addWinSpy = vi.spyOn(window, 'addEventListener');
      const removeWinSpy = vi.spyOn(window, 'removeEventListener');

      const ws = ref('acme');
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

      expect(addDocSpy).toHaveBeenCalledWith('visibilitychange', expect.any(Function));
      expect(addWinSpy).toHaveBeenCalledWith('focus', expect.any(Function));
      expect(addWinSpy).toHaveBeenCalledWith('online', expect.any(Function));

      scope.stop();
      vi.advanceTimersByTime(30_000);

      expect(removeDocSpy).toHaveBeenCalledWith('visibilitychange', expect.any(Function));
      expect(removeWinSpy).toHaveBeenCalledWith('focus', expect.any(Function));
      expect(removeWinSpy).toHaveBeenCalledWith('online', expect.any(Function));
    });

    it('does not double-register listeners across a slug-change reopen', async () => {
      const addWinSpy = vi.spyOn(window, 'addEventListener');

      const ws = ref('acme');
      const scope = effectScope();
      scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

      ws.value = 'globex';
      await nextTick();

      const focusRegistrations = addWinSpy.mock.calls.filter(([type]) => type === 'focus');
      expect(focusRegistrations).toHaveLength(2);

      scope.stop();
    });
  });
});
