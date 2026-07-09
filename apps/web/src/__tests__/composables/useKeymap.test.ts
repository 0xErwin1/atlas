import { afterEach, describe, expect, it, vi } from 'vitest';
import { ref } from 'vue';
import { installKeymapListener, resetKeymapForTests, useKeymap } from '@/composables/useKeymap';
import { KEYMAP_PRIORITIES } from '@/lib/keymap';

function keydown(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  return new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init });
}

describe('useKeymap', () => {
  afterEach(() => {
    resetKeymapForTests();
  });

  it('installs one window listener and removes it when uninstalled', () => {
    const first = installKeymapListener();
    const second = installKeymapListener();
    const handler = vi.fn();
    useKeymap().registerShortcut({ id: 'escape', priority: KEYMAP_PRIORITIES.global, handler });

    window.dispatchEvent(keydown('Escape'));
    expect(handler).toHaveBeenCalledTimes(1);

    second();
    window.dispatchEvent(keydown('Escape'));
    expect(handler).toHaveBeenCalledTimes(2);

    first();
    window.dispatchEvent(keydown('Escape'));
    expect(handler).toHaveBeenCalledTimes(2);
  });

  it('routes to the latest enabled registration when priorities tie', () => {
    installKeymapListener();
    const first = vi.fn();
    const second = vi.fn();
    const { registerShortcut } = useKeymap();

    registerShortcut({ id: 'board-search', priority: KEYMAP_PRIORITIES.board, handler: first });
    registerShortcut({ id: 'board-search', priority: KEYMAP_PRIORITIES.board, handler: second });

    const event = keydown('/');
    window.dispatchEvent(event);

    expect(second).toHaveBeenCalledTimes(1);
    expect(first).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(true);
  });

  it('lets lower-precedence handlers run when a handler returns false', () => {
    installKeymapListener();
    const overlay = vi.fn(() => false);
    const task = vi.fn();
    const { registerShortcut } = useKeymap();

    registerShortcut({ id: 'escape', priority: KEYMAP_PRIORITIES.task, handler: task });
    registerShortcut({ id: 'escape', priority: KEYMAP_PRIORITIES.overlay, handler: overlay });

    window.dispatchEvent(keydown('Escape'));

    expect(overlay).toHaveBeenCalledTimes(1);
    expect(task).toHaveBeenCalledTimes(1);
  });

  it('honors enabled refs and refreshes activation order when re-enabled', () => {
    installKeymapListener();
    const firstEnabled = ref(false);
    const first = vi.fn();
    const second = vi.fn();
    const { registerShortcut } = useKeymap();

    registerShortcut({
      id: 'escape',
      priority: KEYMAP_PRIORITIES.board,
      enabled: firstEnabled,
      handler: first,
    });
    registerShortcut({ id: 'escape', priority: KEYMAP_PRIORITIES.board, handler: second });

    window.dispatchEvent(keydown('Escape'));
    expect(second).toHaveBeenCalledTimes(1);
    expect(first).not.toHaveBeenCalled();

    firstEnabled.value = true;
    window.dispatchEvent(keydown('Escape'));

    expect(first).toHaveBeenCalledTimes(1);
    expect(second).toHaveBeenCalledTimes(1);
  });

  it('orders same-priority enabled refs by the latest open transition before keydown', () => {
    installKeymapListener();
    const firstEnabled = ref(false);
    const secondEnabled = ref(false);
    const first = vi.fn();
    const second = vi.fn();
    const { registerShortcut } = useKeymap();

    registerShortcut({
      id: 'escape',
      priority: KEYMAP_PRIORITIES.overlay,
      enabled: firstEnabled,
      handler: first,
    });
    registerShortcut({
      id: 'escape',
      priority: KEYMAP_PRIORITIES.overlay,
      enabled: secondEnabled,
      handler: second,
    });

    secondEnabled.value = true;
    firstEnabled.value = true;
    window.dispatchEvent(keydown('Escape'));

    expect(first).toHaveBeenCalledTimes(1);
    expect(second).not.toHaveBeenCalled();
  });

  it('unregisters handlers and exposes catalog rows', () => {
    installKeymapListener();
    const handler = vi.fn();
    const { catalog, registerShortcut } = useKeymap();
    const unregister = registerShortcut({ id: 'shortcuts-help', handler });

    expect(catalog.value.some((shortcut) => shortcut.id === 'shortcuts-help')).toBe(true);

    unregister();
    window.dispatchEvent(keydown('?', { shiftKey: true }));

    expect(handler).not.toHaveBeenCalled();
  });
});
