import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  formatShortcut,
  formatShortcutKey,
  getShortcutCatalog,
  isMarkdownEditorTarget,
  isTextEntryTarget,
  KEYMAP_PRIORITIES,
  matchesShortcut,
  pickFirstShortcutHandler,
  type RegisteredShortcutHandler,
  type ShortcutId,
  type ShortcutMeta,
  shortcutCatalog,
} from '@/lib/keymap';

function keydown(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  return new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init });
}

function targetEvent(key: string, target: EventTarget, init: KeyboardEventInit = {}): KeyboardEvent {
  const event = keydown(key, init);
  Object.defineProperty(event, 'target', { value: target });
  return event;
}

function shortcutById(id: ShortcutId): ShortcutMeta {
  const shortcut = shortcutCatalog.find((entry) => entry.id === id);
  if (shortcut === undefined) {
    throw new Error(`Missing shortcut ${id}`);
  }
  return shortcut;
}

describe('keymap catalog', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('lists the v1 shortcuts from the spec scenarios', () => {
    expect(shortcutCatalog).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: 'command-palette',
          scope: 'global',
          keys: ['mod+k'],
          label: 'Command palette',
        }),
        expect.objectContaining({
          id: 'board-search',
          scope: 'board',
          keys: ['/'],
          label: 'Focus board search',
        }),
        expect.objectContaining({
          id: 'shortcuts-help',
          scope: 'global',
          keys: ['shift+?'],
          label: 'Keyboard shortcuts',
        }),
        expect.objectContaining({
          id: 'escape',
          scope: 'global',
          keys: ['escape'],
          label: 'Dismiss or go back',
        }),
      ]),
    );
  });

  it('returns defensive catalog copies for help surfaces', () => {
    const first = getShortcutCatalog();
    first.pop();

    expect(getShortcutCatalog()).toHaveLength(shortcutCatalog.length);
  });

  it('formats the platform modifier for display', () => {
    expect(formatShortcutKey('mod+k', 'Linux x86_64')).toBe('Ctrl+K');
    expect(formatShortcutKey('mod+k', 'MacIntel')).toBe('⌘K');
  });

  it('formats a catalog shortcut for toolbar titles', () => {
    vi.stubGlobal('navigator', { ...navigator, platform: 'Linux x86_64' });

    expect(formatShortcut('command-palette')).toBe('Ctrl+K');
  });
});

describe('matchesShortcut', () => {
  it('matches Cmd/Ctrl+K, slash, Shift+?, and Escape', () => {
    expect(matchesShortcut(keydown('k', { metaKey: true }), shortcutById('command-palette'))).toBe(true);
    expect(matchesShortcut(keydown('K', { ctrlKey: true }), shortcutById('command-palette'))).toBe(true);
    expect(matchesShortcut(keydown('/'), shortcutById('board-search'))).toBe(true);
    expect(matchesShortcut(keydown('?', { shiftKey: true }), shortcutById('shortcuts-help'))).toBe(true);
    expect(matchesShortcut(keydown('Escape'), shortcutById('escape'))).toBe(true);
  });

  it('rejects mismatched modifiers and composing events', () => {
    const command = shortcutById('command-palette');
    const slash = shortcutById('board-search');

    expect(matchesShortcut(keydown('k'), command)).toBe(false);
    expect(matchesShortcut(keydown('/', { shiftKey: true }), slash)).toBe(false);
    expect(matchesShortcut(keydown('/', { isComposing: true }), slash)).toBe(false);
  });
});

describe('text and editor guards', () => {
  it('detects form fields, contenteditable roots, and explicit text-entry markers', () => {
    const input = document.createElement('input');
    const textarea = document.createElement('textarea');
    const select = document.createElement('select');
    const editable = document.createElement('div');
    editable.contentEditable = 'true';
    const marked = document.createElement('div');
    marked.dataset.keymapTextEntry = '';

    expect(isTextEntryTarget(input)).toBe(true);
    expect(isTextEntryTarget(textarea)).toBe(true);
    expect(isTextEntryTarget(select)).toBe(true);
    expect(isTextEntryTarget(editable)).toBe(true);
    expect(isTextEntryTarget(marked)).toBe(true);
    expect(isTextEntryTarget(document.createElement('button'))).toBe(false);
  });

  it('detects CodeMirror and Markdown editor targets from descendants', () => {
    const root = document.createElement('div');
    root.className = 'cm-editor';
    const child = document.createElement('span');
    root.appendChild(child);

    expect(isMarkdownEditorTarget(child)).toBe(true);
    expect(isTextEntryTarget(child)).toBe(true);
    expect(isMarkdownEditorTarget(document.createElement('div'))).toBe(false);
  });
});

describe('shortcut priority rules', () => {
  it('selects the highest-priority matching handler before lower-priority handlers', () => {
    const event = keydown('Escape');
    const called: string[] = [];
    const handlers: RegisteredShortcutHandler[] = [
      {
        id: 'escape',
        priority: KEYMAP_PRIORITIES.global,
        order: 1,
        handler: vi.fn(() => {
          called.push('global');
          return undefined;
        }),
      },
      {
        id: 'escape',
        priority: KEYMAP_PRIORITIES.board,
        order: 2,
        handler: vi.fn(() => {
          called.push('board');
          return undefined;
        }),
      },
      {
        id: 'escape',
        priority: KEYMAP_PRIORITIES.overlay,
        order: 3,
        handler: vi.fn(() => {
          called.push('overlay');
          return undefined;
        }),
      },
    ];

    const picked = pickFirstShortcutHandler(event, handlers);
    picked?.handler(event);

    expect(called).toEqual(['overlay']);
  });

  it('breaks same-priority ties by latest activation order', () => {
    const event = keydown('/');
    const handlers: RegisteredShortcutHandler[] = [
      { id: 'board-search', priority: KEYMAP_PRIORITIES.board, order: 4, handler: vi.fn() },
      { id: 'board-search', priority: KEYMAP_PRIORITIES.board, order: 9, handler: vi.fn() },
    ];

    expect(pickFirstShortcutHandler(event, handlers)).toBe(handlers[1]);
  });

  it('guards text targets and Markdown editors unless a handler opts in', () => {
    const input = document.createElement('input');
    const markdown = document.createElement('div');
    markdown.className = 'cm-content';
    const commandHandler: RegisteredShortcutHandler = {
      id: 'command-palette',
      priority: KEYMAP_PRIORITIES.global,
      order: 1,
      allowInText: true,
      blockInMarkdown: true,
      handler: vi.fn(),
    };
    const slashHandler: RegisteredShortcutHandler = {
      id: 'board-search',
      priority: KEYMAP_PRIORITIES.board,
      order: 2,
      handler: vi.fn(),
    };

    expect(pickFirstShortcutHandler(targetEvent('k', input, { metaKey: true }), [commandHandler])).toBe(
      commandHandler,
    );
    expect(
      pickFirstShortcutHandler(targetEvent('k', markdown, { metaKey: true }), [commandHandler]),
    ).toBeNull();
    expect(pickFirstShortcutHandler(targetEvent('/', input), [slashHandler])).toBeNull();
  });
});
