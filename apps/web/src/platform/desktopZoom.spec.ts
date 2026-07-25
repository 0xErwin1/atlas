import { flushPromises } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { installDesktopZoom } from '@/platform/desktopZoom';

function desktopPreferences(zoomFactor: number) {
  return {
    window_decorations: true,
    zoom_factor: zoomFactor,
    start_on_login: false,
    system_tray: true,
  };
}

function makeTransport(initialZoom: number) {
  const getZoom = vi.fn().mockResolvedValue({ data: desktopPreferences(initialZoom) });
  const setZoom = vi.fn((zoomFactor: number) => Promise.resolve({ data: desktopPreferences(zoomFactor) }));
  return { getZoom, setZoom };
}

async function dispatchKeydown(init: KeyboardEventInit): Promise<KeyboardEvent> {
  const event = new KeyboardEvent('keydown', { cancelable: true, ...init });
  window.dispatchEvent(event);
  await flushPromises();
  return event;
}

describe('installDesktopZoom', () => {
  let teardown: (() => void) | null = null;

  afterEach(() => {
    teardown?.();
    teardown = null;
  });

  it('zooms in on Ctrl+= and syncs to the clamped value the host returns', async () => {
    const transport = makeTransport(1);
    teardown = installDesktopZoom(transport);
    await flushPromises();

    const event = await dispatchKeydown({ key: '=', ctrlKey: true });

    expect(event.defaultPrevented).toBe(true);
    expect(transport.setZoom).toHaveBeenCalledWith(expect.closeTo(1.1, 5));
  });

  it('resets to the default zoom on Ctrl+0', async () => {
    const transport = makeTransport(1.5);
    teardown = installDesktopZoom(transport);
    await flushPromises();

    await dispatchKeydown({ key: '0', ctrlKey: true });

    expect(transport.setZoom).toHaveBeenCalledWith(1);
  });

  it('clamps at the maximum bound and does not persist past it', async () => {
    const transport = makeTransport(3);
    teardown = installDesktopZoom(transport);
    await flushPromises();

    const event = await dispatchKeydown({ key: '=', ctrlKey: true });

    expect(event.defaultPrevented).toBe(true);
    expect(transport.setZoom).not.toHaveBeenCalled();
  });

  it('ignores combinations without a modifier or a matching key', async () => {
    const transport = makeTransport(1);
    teardown = installDesktopZoom(transport);
    await flushPromises();

    const withoutModifier = await dispatchKeydown({ key: '=' });
    const nonMatchingKey = await dispatchKeydown({ key: 'a', ctrlKey: true });

    expect(withoutModifier.defaultPrevented).toBe(false);
    expect(nonMatchingKey.defaultPrevented).toBe(false);
    expect(transport.setZoom).not.toHaveBeenCalled();
  });

  it('applies a hotkey pressed before the initial sync relative to the fetched zoom', async () => {
    let resolveGetZoom!: (result: { data: ReturnType<typeof desktopPreferences> }) => void;
    const getZoom = vi.fn(
      () =>
        new Promise<{ data: ReturnType<typeof desktopPreferences> }>((resolve) => {
          resolveGetZoom = resolve;
        }),
    );
    const setZoom = vi.fn((zoomFactor: number) => Promise.resolve({ data: desktopPreferences(zoomFactor) }));
    teardown = installDesktopZoom({ getZoom, setZoom });

    const event = new KeyboardEvent('keydown', { cancelable: true, key: '=', ctrlKey: true });
    window.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(setZoom).not.toHaveBeenCalled();

    resolveGetZoom({ data: desktopPreferences(1.5) });
    await flushPromises();

    expect(setZoom).toHaveBeenCalledTimes(1);
    expect(setZoom).toHaveBeenCalledWith(expect.closeTo(1.6, 5));
  });

  it('stops handling keys after teardown', async () => {
    const transport = makeTransport(1);
    const cleanup = installDesktopZoom(transport);
    await flushPromises();

    cleanup();
    await dispatchKeydown({ key: '=', ctrlKey: true });

    expect(transport.setZoom).not.toHaveBeenCalled();
  });
});
