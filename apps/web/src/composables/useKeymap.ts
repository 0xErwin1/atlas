import { computed, type MaybeRefOrGetter, toValue, type WatchStopHandle, watch } from 'vue';
import {
  getShortcutCatalog,
  KEYMAP_PRIORITIES,
  matchingShortcutHandlers,
  type RegisteredShortcutHandler,
  type ShortcutId,
  type ShortcutMeta,
} from '@/lib/keymap';

export interface ShortcutHandlerRegistration {
  id: ShortcutId;
  enabled?: MaybeRefOrGetter<boolean>;
  priority?: number;
  allowInText?: boolean;
  blockInMarkdown?: boolean;
  handler: (event: KeyboardEvent) => boolean | undefined;
}

interface ActiveShortcutHandler extends RegisteredShortcutHandler {
  enabled?: MaybeRefOrGetter<boolean>;
  lastEnabled: boolean;
  stopWatchingEnabled?: WatchStopHandle;
}

let handlers: ActiveShortcutHandler[] = [];
let orderCounter = 0;
let listenerInstallCount = 0;
let listenerInstalled = false;

const catalog = computed<ShortcutMeta[]>(() => getShortcutCatalog());

export function useKeymap() {
  return {
    catalog,
    registerShortcut,
  };
}

export function installKeymapListener(): () => void {
  listenerInstallCount += 1;

  if (!listenerInstalled) {
    window.addEventListener('keydown', onWindowKeydown);
    listenerInstalled = true;
  }

  let uninstalled = false;
  return () => {
    if (uninstalled) return;
    uninstalled = true;
    listenerInstallCount = Math.max(0, listenerInstallCount - 1);

    if (listenerInstallCount === 0 && listenerInstalled) {
      window.removeEventListener('keydown', onWindowKeydown);
      listenerInstalled = false;
    }
  };
}

export function registerShortcut(registration: ShortcutHandlerRegistration): () => void {
  const enabled = resolveEnabled(registration.enabled);
  const active: ActiveShortcutHandler = {
    id: registration.id,
    enabled: registration.enabled,
    priority: registration.priority ?? defaultPriorityForShortcut(registration.id),
    order: enabled ? nextOrder() : 0,
    allowInText: registration.allowInText,
    blockInMarkdown: registration.blockInMarkdown,
    handler: registration.handler,
    lastEnabled: enabled,
  };

  if (registration.enabled !== undefined) {
    active.stopWatchingEnabled = watch(
      () => resolveEnabled(registration.enabled),
      (isEnabled) => {
        if (isEnabled && !active.lastEnabled) {
          active.order = nextOrder();
        }
        active.lastEnabled = isEnabled;
      },
      { flush: 'sync' },
    );
  }

  handlers = [...handlers, active];

  let unregistered = false;
  return () => {
    if (unregistered) return;
    unregistered = true;
    active.stopWatchingEnabled?.();
    handlers = handlers.filter((handler) => handler !== active);
  };
}

export function resetKeymapForTests(): void {
  if (listenerInstalled) {
    window.removeEventListener('keydown', onWindowKeydown);
  }

  for (const handler of handlers) {
    handler.stopWatchingEnabled?.();
  }

  handlers = [];
  orderCounter = 0;
  listenerInstallCount = 0;
  listenerInstalled = false;
}

function onWindowKeydown(event: KeyboardEvent): void {
  const enabledHandlers = activeEnabledHandlers();
  const matches = matchingShortcutHandlers(event, enabledHandlers);

  for (const handler of matches) {
    const result = handler.handler(event);
    if (result !== false) {
      event.preventDefault();
      return;
    }
  }
}

function activeEnabledHandlers(): RegisteredShortcutHandler[] {
  return handlers.filter((handler) => {
    const enabled = resolveEnabled(handler.enabled);
    if (enabled && !handler.lastEnabled) {
      handler.order = nextOrder();
    }
    handler.lastEnabled = enabled;
    return enabled;
  });
}

function resolveEnabled(enabled: MaybeRefOrGetter<boolean> | undefined): boolean {
  return enabled === undefined ? true : toValue(enabled);
}

function nextOrder(): number {
  orderCounter += 1;
  return orderCounter;
}

function defaultPriorityForShortcut(id: ShortcutId): number {
  const priorityById: Record<ShortcutId, number> = {
    'command-palette': KEYMAP_PRIORITIES.global,
    'board-search': KEYMAP_PRIORITIES.board,
    'shortcuts-help': KEYMAP_PRIORITIES.global,
    escape: KEYMAP_PRIORITIES.global,
  };

  return priorityById[id];
}
