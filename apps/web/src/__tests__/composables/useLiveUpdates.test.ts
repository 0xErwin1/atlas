import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { effectScope, nextTick, ref } from 'vue';
import { useLiveUpdates } from '@/composables/useLiveUpdates';

class FakeEventSource {
  static instances: FakeEventSource[] = [];

  url: string;
  closed = false;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
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
  }

  emitOpen(): void {
    this.onopen?.(new Event('open'));
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
    FakeEventSource.instances = [];
    vi.stubGlobal('EventSource', FakeEventSource);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('opens a stream for the workspace when the slug is set', () => {
    const ws = ref('acme');
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

    expect(FakeEventSource.instances).toHaveLength(1);
    expect(FakeEventSource.instances[0]?.url).toBe('/v1/workspaces/acme/events');

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
    expect(FakeEventSource.instances[1]?.url).toBe('/v1/workspaces/globex/events');

    scope.stop();
  });

  it('closes the stream on scope dispose', () => {
    const ws = ref('acme');
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync: vi.fn() }));

    scope.stop();

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

  it('does not resync on the first open but resyncs on a reconnect', () => {
    const ws = ref('acme');
    const onResync = vi.fn();
    const scope = effectScope();
    scope.run(() => useLiveUpdates(ws, { onEvent: vi.fn(), onResync }));

    const stream = FakeEventSource.instances[0];

    stream?.emitOpen();
    expect(onResync).not.toHaveBeenCalled();

    stream?.emitOpen();
    expect(onResync).toHaveBeenCalledTimes(1);

    scope.stop();
  });
});
